use densevm::lifecycle::{CloneRecord, CloneRegistry, CloneState};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("densevm-reconcile-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn clone_reconcile_marks_a_dead_qemu_failed() {
    let root = temp_dir();
    let state_dir = root.join("state");
    let registry = CloneRegistry::open(&state_dir).unwrap();
    let record = CloneRecord {
        schema_version: 1,
        id: "alpha".into(),
        generation_id: format!("sha256:{}", "a".repeat(64)),
        qemu_pid: None,
        qmp_socket: state_dir.join("alpha/alpha.qmp.sock"),
        disk_overlay: state_dir.join("alpha/disk.qcow2"),
        machine_id: "0123456789abcdef0123456789abcdef".into(),
        mac: "02:00:00:00:00:01".into(),
        state: CloneState::Starting,
    };
    registry.create(&record).unwrap();
    registry
        .transition(
            "alpha",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "reconcile", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "reconcile command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(records[0]["state"], "failed");
    assert!(records[0]["qemu_pid"].is_null());
    fs::remove_dir_all(root).unwrap();
}
