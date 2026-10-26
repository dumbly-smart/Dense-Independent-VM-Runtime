//! Durable, per-clone lifecycle metadata.
//!
//! This module records ownership and state for independently launched QEMU
//! processes. It does not claim that a record alone can recover a VM after a
//! host crash; recovery must verify the process and QMP endpoint.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const RECORD_FILE: &str = "clone.json";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneState {
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CloneRecord {
    pub schema_version: u32,
    pub id: String,
    pub generation_id: String,
    pub qemu_pid: Option<u32>,
    #[serde(default)]
    pub qemu_start_ticks: Option<u64>,
    #[serde(default)]
    pub qemu_boot_id: Option<String>,
    #[serde(default)]
    pub launcher_pid: Option<u32>,
    #[serde(default)]
    pub launcher_start_ticks: Option<u64>,
    #[serde(default)]
    pub launcher_boot_id: Option<String>,
    pub qmp_socket: PathBuf,
    pub disk_overlay: PathBuf,
    pub machine_id: String,
    pub mac: String,
    pub state: CloneState,
}

impl CloneRecord {
    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.id)?;
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported clone record schema version {}",
                self.schema_version
            ));
        }
        if self.generation_id.len() != 71
            || !self.generation_id.starts_with("sha256:")
            || !self.generation_id[7..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("generation_id must be sha256 followed by 64 hexadecimal digits".into());
        }
        if !self.qmp_socket.is_absolute() || !self.disk_overlay.is_absolute() {
            return Err("QMP socket and disk overlay paths must be absolute".into());
        }
        if self.machine_id.len() != 32
            || !self
                .machine_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("machine_id must contain 32 lowercase hexadecimal digits".into());
        }
        let mac = self
            .mac
            .split(':')
            .map(|octet| {
                if octet.len() != 2
                    || !octet
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(());
                }
                u8::from_str_radix(octet, 16).map_err(|_| ())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "MAC must use six lowercase, two-digit hexadecimal octets".to_string())?;
        if mac.len() != 6 || mac[0] & 3 != 2 {
            return Err("MAC must be a locally administered unicast address".into());
        }
        if self.qemu_pid == Some(0) {
            return Err("QEMU PID must be positive when present".into());
        }
        if self.qemu_pid.is_none() && self.qemu_start_ticks.is_some() {
            return Err("QEMU start-time ticks require a QEMU PID".into());
        }
        if self.qemu_pid.is_none() && self.qemu_boot_id.is_some() {
            return Err("QEMU boot ID requires a QEMU PID".into());
        }
        if self.qemu_start_ticks.is_some() != self.qemu_boot_id.is_some() {
            return Err("QEMU start-time ticks and boot ID must be present together".into());
        }
        let launcher_fields = [
            self.launcher_pid.is_some(),
            self.launcher_start_ticks.is_some(),
            self.launcher_boot_id.is_some(),
        ];
        if launcher_fields.iter().any(|present| *present)
            && launcher_fields.iter().any(|present| !present)
        {
            return Err(
                "launcher PID, start-time ticks, and boot ID must be present together".into(),
            );
        }
        if self.launcher_pid == Some(0) {
            return Err("launcher PID must be positive when present".into());
        }
        if self.state == CloneState::Running && self.qemu_pid.is_none() {
            return Err("running clone record must include a QEMU PID".into());
        }
        Ok(())
    }
}

/// Registry operations are intended to be serialized by one manager process.
pub struct CloneRegistry {
    root: PathBuf,
}

impl CloneRegistry {
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root)
            .map_err(|error| format!("create clone registry {}: {error}", root.display()))?;
        let metadata = fs::symlink_metadata(root)
            .map_err(|error| format!("inspect clone registry {}: {error}", root.display()))?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "clone registry root must be a real directory: {}",
                root.display()
            ));
        }
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure clone registry {}: {error}", root.display()))?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("resolve clone registry: {error}"))?;
        Ok(Self { root })
    }

    pub fn create(&self, record: &CloneRecord) -> Result<(), String> {
        record.validate()?;
        for existing in self.list()? {
            if existing.machine_id == record.machine_id || existing.mac == record.mac {
                return Err(format!(
                    "clone {} duplicates a registered machine ID or MAC address",
                    record.id
                ));
            }
        }
        let directory = self.root.join(&record.id);
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(&directory)
            .map_err(|error| format!("reserve clone ID {}: {error}", record.id))?;
        if let Err(error) = write_record_atomic(&directory, record) {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<CloneRecord, String> {
        validate_id(id)?;
        let directory = self.root.join(id);
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|error| format!("inspect clone {id}: {error}"))?;
        if !metadata.file_type().is_dir() {
            return Err(format!("clone entry {id} is not a real directory"));
        }
        let path = directory.join(RECORD_FILE);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect clone record {id}: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err(format!("clone record {id} is not a regular file"));
        }
        let bytes = fs::read(&path).map_err(|error| format!("read clone record {id}: {error}"))?;
        let record: CloneRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse clone record {id}: {error}"))?;
        record.validate()?;
        if record.id != id {
            return Err(format!("clone record ID mismatch for {id}"));
        }
        Ok(record)
    }

    pub fn list(&self) -> Result<Vec<CloneRecord>, String> {
        let mut records = Vec::new();
        let entries = fs::read_dir(&self.root)
            .map_err(|error| format!("list clone registry {}: {error}", self.root.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("read clone registry entry: {error}"))?;
            if entry
                .file_type()
                .map_err(|error| format!("inspect clone registry entry: {error}"))?
                .is_dir()
            {
                let id = entry.file_name().to_string_lossy().into_owned();
                records.push(self.get(&id)?);
            }
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(records)
    }

    pub fn reconcile(&self) -> Result<Vec<CloneRecord>, String> {
        for record in self.list()? {
            if record.state == CloneState::Starting {
                let (Some(launcher_pid), Some(start_ticks)) =
                    (record.launcher_pid, record.launcher_start_ticks)
                else {
                    continue;
                };
                if process_matches(
                    launcher_pid,
                    Some(start_ticks),
                    record.launcher_boot_id.as_deref(),
                )? {
                    continue;
                }
                if let Some(pid) = record.qemu_pid {
                    if process_matches(
                        pid,
                        record.qemu_start_ticks,
                        record.qemu_boot_id.as_deref(),
                    )? {
                        continue;
                    }
                }
                if qmp_endpoint_reachable(&record.qmp_socket)? {
                    continue;
                }
                self.transition(&record.id, CloneState::Starting, CloneState::Failed, None)?;
                continue;
            }
            if record.state != CloneState::Running {
                continue;
            }
            let Some(pid) = record.qemu_pid else {
                continue;
            };
            if process_matches(pid, record.qemu_start_ticks, record.qemu_boot_id.as_deref())?
                || qmp_endpoint_reachable(&record.qmp_socket)?
            {
                continue;
            }
            self.transition(&record.id, CloneState::Running, CloneState::Failed, None)?;
        }
        self.list()
    }

    pub fn record_starting_qemu_pid(&self, id: &str, pid: u32) -> Result<CloneRecord, String> {
        if pid == 0 {
            return Err("QEMU PID must be positive".into());
        }
        let mut record = self.get(id)?;
        if record.state != CloneState::Starting {
            return Err(format!("clone {id} is {:?}, not starting", record.state));
        }
        let start_ticks = process_start_ticks(pid)?
            .ok_or_else(|| format!("QEMU PID {pid} disappeared during clone start"))?;
        let boot_id = current_boot_id()?;
        record.qemu_pid = Some(pid);
        record.qemu_start_ticks = Some(start_ticks);
        record.qemu_boot_id = Some(boot_id);
        record.validate()?;
        write_record_atomic(&self.root.join(id), &record)?;
        Ok(record)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let record = self.get(id)?;
        if !matches!(record.state, CloneState::Stopped | CloneState::Failed) {
            return Err(format!(
                "clone {id} is {:?}; only stopped or failed clones can be deleted",
                record.state
            ));
        }
        if let Some(pid) = record.qemu_pid {
            if process_matches(pid, record.qemu_start_ticks, record.qemu_boot_id.as_deref())? {
                return Err(format!(
                    "clone {id} still has a live recorded QEMU PID {pid}"
                ));
            }
        }
        if qmp_endpoint_reachable(&record.qmp_socket)? {
            return Err(format!(
                "clone {id} QMP endpoint is reachable; refusing to delete ambiguous state"
            ));
        }

        let clone_dir = self.root.join(id);
        for artifact in [&record.qmp_socket, &record.disk_overlay] {
            if artifact.parent() != Some(clone_dir.as_path()) {
                return Err(format!(
                    "clone {id} artifact path is outside its state directory: {}",
                    artifact.display()
                ));
            }
        }
        let metadata = fs::symlink_metadata(&clone_dir).map_err(|error| {
            format!(
                "inspect clone state directory {}: {error}",
                clone_dir.display()
            )
        })?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "clone state path is not a real directory: {}",
                clone_dir.display()
            ));
        }
        fs::remove_dir_all(&clone_dir).map_err(|error| {
            format!(
                "delete clone state directory {}: {error}",
                clone_dir.display()
            )
        })
    }

    pub fn transition(
        &self,
        id: &str,
        expected: CloneState,
        next: CloneState,
        qemu_pid: Option<u32>,
    ) -> Result<CloneRecord, String> {
        let mut record = self.get(id)?;
        if record.state != expected {
            return Err(format!(
                "clone {id} is {:?}; expected {:?}",
                record.state, expected
            ));
        }
        if !valid_transition(expected, next) {
            return Err(format!(
                "invalid clone state transition {expected:?} -> {next:?}"
            ));
        }
        record.state = next;
        if record.qemu_pid != qemu_pid {
            record.qemu_start_ticks = None;
            record.qemu_boot_id = None;
        }
        record.qemu_pid = qemu_pid;
        if qemu_pid.is_none() {
            record.qemu_start_ticks = None;
            record.qemu_boot_id = None;
        }
        if next != CloneState::Starting {
            record.launcher_pid = None;
            record.launcher_start_ticks = None;
            record.launcher_boot_id = None;
        }
        record.validate()?;
        write_record_atomic(&self.root.join(id), &record)?;
        Ok(record)
    }

    pub fn stop(&self, id: &str, timeout: Duration) -> Result<CloneRecord, String> {
        let record = self.get(id)?;
        if record.state != CloneState::Running {
            return Err(format!("clone {id} is {:?}, not running", record.state));
        }
        if timeout.is_zero() {
            return Err("clone stop timeout must be positive".into());
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or("clone stop timeout is too large")?;
        let mut qmp = match crate::qmp::Qmp::connect(&record.qmp_socket, timeout) {
            Ok(qmp) => qmp,
            Err(error) => {
                if let Some(pid) = record.qemu_pid {
                    if !process_matches(
                        pid,
                        record.qemu_start_ticks,
                        record.qemu_boot_id.as_deref(),
                    )? && !qmp_endpoint_reachable(&record.qmp_socket)?
                    {
                        return self.transition(id, CloneState::Running, CloneState::Failed, None);
                    }
                }
                return Err(error);
            }
        };
        let identity = qmp.execute("query-name", None)?;
        if identity["name"].as_str() != Some(id) {
            return Err(format!(
                "QMP endpoint for clone {id} reports a different VM name: {identity}"
            ));
        }
        qmp.execute("quit", None)?;
        drop(qmp);

        let pid = record
            .qemu_pid
            .ok_or_else(|| format!("running clone {id} has no recorded QEMU PID"))?;
        let mut qmp_closed = false;
        loop {
            if !qmp_closed {
                match UnixStream::connect(&record.qmp_socket) {
                    Ok(stream) => {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        qmp_closed = true;
                    }
                    Err(error) => {
                        return Err(format!("check QMP endpoint after quit: {error}"));
                    }
                }
            }
            if qmp_closed
                && !process_matches(pid, record.qemu_start_ticks, record.qemu_boot_id.as_deref())?
            {
                return self.transition(id, CloneState::Running, CloneState::Stopped, None);
            }
            if Instant::now() >= deadline {
                return Err(format!("QEMU for clone {id} did not stop before timeout"));
            }
            thread::sleep(
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

fn process_exists(pid: u32) -> Result<bool, String> {
    match fs::symlink_metadata(format!("/proc/{pid}")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("inspect recorded QEMU PID {pid}: {error}")),
    }
}

fn process_matches(
    pid: u32,
    expected_start_ticks: Option<u64>,
    expected_boot_id: Option<&str>,
) -> Result<bool, String> {
    if let Some(expected_boot_id) = expected_boot_id {
        if current_boot_id()? != expected_boot_id {
            return Ok(false);
        }
    }
    if let Some(expected_start_ticks) = expected_start_ticks {
        return Ok(process_start_ticks(pid)? == Some(expected_start_ticks));
    }
    process_exists(pid)
}

pub(crate) fn current_boot_id() -> Result<String, String> {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|boot_id| boot_id.trim().to_string())
        .map_err(|error| format!("read Linux boot ID: {error}"))
}

pub(crate) fn process_start_ticks(pid: u32) -> Result<Option<u64>, String> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read process start time for PID {pid}: {error}")),
    };
    let (_, fields) = stat
        .rsplit_once(')')
        .ok_or_else(|| format!("malformed /proc/{pid}/stat process name"))?;
    let start_ticks = fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| format!("/proc/{pid}/stat omitted start-time ticks"))?
        .parse::<u64>()
        .map_err(|error| format!("invalid /proc/{pid}/stat start-time ticks: {error}"))?;
    Ok(Some(start_ticks))
}

fn qmp_endpoint_reachable(path: &Path) -> Result<bool, String> {
    match UnixStream::connect(path) {
        Ok(stream) => {
            drop(stream);
            Ok(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(format!("inspect QMP endpoint {}: {error}", path.display())),
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("clone ID must be 1-64 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(())
}

fn valid_transition(from: CloneState, to: CloneState) -> bool {
    matches!(
        (from, to),
        (CloneState::Starting, CloneState::Running)
            | (CloneState::Starting, CloneState::Failed)
            | (CloneState::Running, CloneState::Stopped)
            | (CloneState::Running, CloneState::Failed)
    )
}

fn write_record_atomic(directory: &Path, record: &CloneRecord) -> Result<(), String> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".clone-{}-{sequence}.tmp", std::process::id()));
    let target = directory.join(RECORD_FILE);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("create temporary clone record: {error}"))?;
        serde_json::to_writer_pretty(&mut file, record)
            .map_err(|error| format!("serialize clone record: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("finish clone record: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync clone record: {error}"))?;
        fs::rename(&temporary, &target)
            .map_err(|error| format!("publish clone record: {error}"))?;
        File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync clone record directory: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::process::Command;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "densevm-lifecycle-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn record(id: &str) -> CloneRecord {
        CloneRecord {
            schema_version: 1,
            id: id.into(),
            generation_id: format!("sha256:{}", "a".repeat(64)),
            qemu_pid: None,
            qemu_start_ticks: None,
            qemu_boot_id: None,
            launcher_pid: None,
            launcher_start_ticks: None,
            launcher_boot_id: None,
            qmp_socket: PathBuf::from("/run/densevm/alpha.qmp"),
            disk_overlay: PathBuf::from("/var/lib/densevm/alpha.qcow2"),
            machine_id: "0123456789abcdef0123456789abcdef".into(),
            mac: "02:00:00:00:00:01".into(),
            state: CloneState::Starting,
        }
    }

    fn qmp_server(path: &Path, name: &str, remove_on_quit: bool) -> thread::JoinHandle<()> {
        let listener = UnixListener::bind(path).unwrap();
        let path = path.to_path_buf();
        let name = name.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"{\"QMP\":{\"version\":{}}}\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&line).unwrap()["execute"],
                "qmp_capabilities"
            );
            stream.write_all(b"{\"return\":{}}\n").unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&line).unwrap()["execute"],
                "query-name"
            );
            stream
                .write_all(format!("{{\"return\":{{\"name\":\"{name}\"}}}}\n").as_bytes())
                .unwrap();
            if !remove_on_quit {
                return;
            }
            line.clear();
            reader.read_line(&mut line).unwrap();
            let command: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(command["execute"], "quit");
            stream.write_all(b"{\"return\":{}}\n").unwrap();
            let _ = stream.shutdown(Shutdown::Both);
            drop(reader);
            drop(stream);
            drop(listener);
            fs::remove_file(path).unwrap();
        })
    }

    #[test]
    fn registry_persists_private_clone_metadata_without_entropy() {
        let root = temp_dir("persist");
        let registry = CloneRegistry::open(&root).unwrap();
        registry.create(&record("alpha")).unwrap();

        let loaded = registry.get("alpha").unwrap();
        assert_eq!(loaded.id, "alpha");
        assert_eq!(loaded.state, CloneState::Starting);
        assert_eq!(loaded.mac, "02:00:00:00:00:01");
        let json = fs::read_to_string(root.join("alpha/clone.json")).unwrap();
        assert!(!json.contains("entropy"));
        assert_eq!(registry.list().unwrap().len(), 1);
        drop(registry);
        let reopened = CloneRegistry::open(&root).unwrap();
        assert_eq!(
            reopened.get("alpha").unwrap().generation_id,
            loaded.generation_id
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_refuses_duplicate_ids_and_invalid_transitions() {
        let root = temp_dir("transitions");
        let registry = CloneRegistry::open(&root).unwrap();
        registry.create(&record("alpha")).unwrap();
        assert!(registry.create(&record("alpha")).is_err());
        assert!(registry.create(&record("beta")).is_err());
        assert!(
            registry
                .transition("alpha", CloneState::Starting, CloneState::Stopped, None)
                .is_err()
        );
        let running = registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(1234),
            )
            .unwrap();
        assert_eq!(running.qemu_pid, Some(1234));
        assert!(
            registry
                .transition("alpha", CloneState::Starting, CloneState::Failed, None)
                .is_err()
        );
        let stopped = registry
            .transition("alpha", CloneState::Running, CloneState::Stopped, None)
            .unwrap();
        assert_eq!(stopped.state, CloneState::Stopped);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_verifies_qmp_name_and_waits_for_the_socket_to_close() {
        let root = temp_dir("stop");
        let mut clone = record("alpha");
        clone.qmp_socket = root.join("alpha.qmp");
        let registry = CloneRegistry::open(&root.join("registry")).unwrap();
        registry.create(&clone).unwrap();
        registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(1234),
            )
            .unwrap();
        let server = qmp_server(&clone.qmp_socket, "alpha", true);

        let stopped = registry.stop("alpha", Duration::from_secs(1)).unwrap();
        assert_eq!(stopped.state, CloneState::Stopped);
        assert_eq!(stopped.qemu_pid, None);
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_waits_for_the_recorded_process_to_exit_after_qmp_closes() {
        let root = temp_dir("stop-process-exit");
        let mut clone = record("alpha");
        clone.qmp_socket = root.join("alpha.qmp");
        let registry = CloneRegistry::open(&root.join("registry")).unwrap();
        registry.create(&clone).unwrap();
        let mut process = Command::new("sleep").arg("30").spawn().unwrap();
        registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(process.id()),
            )
            .unwrap();
        let server = qmp_server(&clone.qmp_socket, "alpha", true);
        let state_dir = root.join("registry");
        let stop = thread::spawn(move || {
            CloneRegistry::open(&state_dir)
                .unwrap()
                .stop("alpha", Duration::from_secs(2))
        });

        server.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        let stop_waited_for_process = !stop.is_finished();
        process.kill().unwrap();
        process.wait().unwrap();
        let stopped = stop.join().unwrap().unwrap();

        assert!(stop_waited_for_process);
        assert_eq!(stopped.state, CloneState::Stopped);
        assert_eq!(stopped.qemu_pid, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_refuses_a_qmp_endpoint_owned_by_another_vm() {
        let root = temp_dir("wrong-qmp");
        let mut clone = record("alpha");
        clone.qmp_socket = root.join("alpha.qmp");
        let registry = CloneRegistry::open(&root.join("registry")).unwrap();
        registry.create(&clone).unwrap();
        registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(1234),
            )
            .unwrap();
        let server = qmp_server(&clone.qmp_socket, "beta", false);

        assert!(registry.stop("alpha", Duration::from_millis(100)).is_err());
        assert_eq!(registry.get("alpha").unwrap().state, CloneState::Running);
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_marks_a_crashed_running_record_failed() {
        let root = temp_dir("stale-process");
        let mut clone = record("alpha");
        clone.qmp_socket = root.join("missing.qmp");
        let registry = CloneRegistry::open(&root.join("registry")).unwrap();
        registry.create(&clone).unwrap();
        registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(u32::MAX),
            )
            .unwrap();

        let stopped = registry.stop("alpha", Duration::from_millis(100)).unwrap();
        assert_eq!(stopped.state, CloneState::Failed);
        assert_eq!(stopped.qemu_pid, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconcile_keeps_a_running_record_when_its_qmp_socket_is_reachable() {
        let root = temp_dir("live-qmp-with-stale-pid");
        let registry = CloneRegistry::open(&root.join("registry")).unwrap();
        let mut clone = record("alpha");
        clone.qmp_socket = root.join("alpha.qmp");
        registry.create(&clone).unwrap();
        registry
            .transition(
                "alpha",
                CloneState::Starting,
                CloneState::Running,
                Some(u32::MAX),
            )
            .unwrap();
        let listener = UnixListener::bind(&clone.qmp_socket).unwrap();
        let server = thread::spawn(move || listener.accept().unwrap());

        let records = registry.reconcile().unwrap();

        assert_eq!(records[0].state, CloneState::Running);
        assert_eq!(records[0].qemu_pid, Some(u32::MAX));
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_validation_rejects_unsafe_ids_and_duplicated_identity_format() {
        let mut invalid = record("../alpha");
        assert!(invalid.validate().is_err());
        invalid = record("alpha");
        invalid.mac = "01:00:00:00:00:01".into();
        assert!(invalid.validate().is_err());
        invalid.mac = "2:00:00:00:00:01".into();
        assert!(invalid.validate().is_err());
    }
}
