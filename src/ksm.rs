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

    fn restore_to(&self, root: &Path) -> Result<(), String> {
        write_value(root, "pages_to_scan", self.pages_to_scan)?;
        write_value(root, "sleep_millisecs", self.sleep_millisecs)?;
        write_value(root, "run", self.run)
    }
}

pub struct KsmGuard {
    root: PathBuf,
    snapshot: KsmSnapshot,
}

impl KsmGuard {
    pub fn enable() -> Result<Self, String> {
        let root = PathBuf::from(KSM_ROOT);
        let snapshot = KsmSnapshot::read_from(&root)?;
        write_value(&root, "run", 1)?;
        Ok(Self { root, snapshot })
    }
}

impl Drop for KsmGuard {
    fn drop(&mut self) {
        let _ = self.snapshot.restore_to(&self.root);
    }
}
