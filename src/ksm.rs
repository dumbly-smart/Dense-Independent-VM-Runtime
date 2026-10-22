use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const KSM_ROOT: &str = "/sys/kernel/mm/ksm";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct KsmSnapshot {
    pub run: u64,
    pub pages_to_scan: u64,
    pub sleep_millisecs: u64,
}

fn read_value(root: &Path, name: &str) -> Result<u64, String> {
    fs::read_to_string(root.join(name))
        .map_err(|e| format!("read KSM {name}: {e}"))?
        .trim()
        .parse()
        .map_err(|e| format!("parse KSM {name}: {e}"))
}

fn write_value(root: &Path, name: &str, value: u64) -> Result<(), String> {
    fs::write(root.join(name), format!("{value}\n")).map_err(|e| format!("write KSM {name}: {e}"))
}

impl KsmSnapshot {
    pub fn read() -> Result<Self, String> {
        Self::read_from(Path::new(KSM_ROOT))
    }

    fn read_from(root: &Path) -> Result<Self, String> {
        Ok(Self {
            run: read_value(root, "run")?,
            pages_to_scan: read_value(root, "pages_to_scan")?,
            sleep_millisecs: read_value(root, "sleep_millisecs")?,
        })
    }
}

pub struct KsmGuard {
    root: PathBuf,
    snapshot: KsmSnapshot,
    changed: bool,
}

impl KsmGuard {
    pub fn enable() -> Result<Self, String> {
        Self::enable_at(PathBuf::from(KSM_ROOT))
    }

    fn enable_at(root: PathBuf) -> Result<Self, String> {
        let snapshot = KsmSnapshot::read_from(&root)?;
        if snapshot.run > 1 {
            return Err("KSM is unmerging pages; refusing to interrupt host operation".into());
        }
        let mut guard = Self {
            root,
            snapshot,
            changed: false,
        };
        if snapshot.run == 0 {
            // Arm rollback before the write, including partial-write failures.
            guard.changed = true;
            write_value(&guard.root, "run", 1)?;
            if read_value(&guard.root, "run")? != 1 {
                return Err("KSM did not enable".into());
            }
        }
        Ok(guard)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if self.changed {
            write_value(&self.root, "run", self.snapshot.run)?;
            if read_value(&self.root, "run")? != self.snapshot.run {
                return Err("KSM restoration verification failed".into());
            }
            self.changed = false;
        }
        Ok(())
    }
}

impl Drop for KsmGuard {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("densevm: KSM restoration failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("densevm-ksm-test-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for (name, value) in [
            ("run", "0"),
            ("pages_to_scan", "100"),
            ("sleep_millisecs", "20"),
        ] {
            fs::write(root.join(name), value).unwrap();
        }
        root
    }

    #[test]
    fn guard_restores_only_its_own_change_and_reports_restore_failure() {
        let root = fixture();
        let mut guard = KsmGuard::enable_at(root.clone()).unwrap();
        assert_eq!(read_value(&root, "run").unwrap(), 1);
        fs::write(root.join("pages_to_scan"), "222").unwrap();
        guard.restore().unwrap();
        assert_eq!(read_value(&root, "run").unwrap(), 0);
        assert_eq!(read_value(&root, "pages_to_scan").unwrap(), 222);
        drop(guard);
        let mut guard = KsmGuard::enable_at(root.clone()).unwrap();
        fs::remove_file(root.join("run")).unwrap();
        fs::create_dir(root.join("run")).unwrap();
        assert!(guard.restore().is_err());
        fs::remove_dir(root.join("run")).unwrap();
        drop(guard);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_to_interrupt_a_host_unmerge_operation() {
        let root = fixture();
        fs::write(root.join("run"), "2").unwrap();
        assert!(KsmGuard::enable_at(root.clone()).is_err());
        assert_eq!(read_value(&root, "run").unwrap(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
