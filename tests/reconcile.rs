use densevm::lifecycle::{CloneRecord, CloneRegistry, CloneState};
use serde_json::Value;
use std::fs;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
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
        qemu_start_ticks: None,
        qemu_boot_id: None,
        launcher_pid: None,
        launcher_start_ticks: None,
        launcher_boot_id: None,
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

#[test]
fn clone_reconcile_marks_an_abandoned_start_failed_when_its_launcher_is_gone() {
    let root = temp_dir();
    let state_dir = root.join("state");
    let clone_dir = state_dir.join("alpha");
    fs::create_dir_all(&clone_dir).unwrap();
    let record = serde_json::json!({
        "schema_version": 1,
        "id": "alpha",
        "generation_id": format!("sha256:{}", "a".repeat(64)),
        "qemu_pid": null,
        "qmp_socket": clone_dir.join("alpha.qmp.sock"),
        "disk_overlay": clone_dir.join("disk.qcow2"),
        "machine_id": "0123456789abcdef0123456789abcdef",
        "mac": "02:00:00:00:00:01",
        "state": "starting",
        "launcher_pid": u32::MAX,
        "launcher_start_ticks": 1,
        "launcher_boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim(),
    });
    fs::write(
        clone_dir.join("clone.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
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

#[test]
fn clone_reconcile_keeps_a_starting_record_while_its_launcher_is_alive() {
    let root = temp_dir();
    let state_dir = root.join("state");
    let clone_dir = state_dir.join("alpha");
    fs::create_dir_all(&clone_dir).unwrap();
    let mut launcher = Command::new("sleep").arg("30").spawn().unwrap();
    let stat = fs::read_to_string(format!("/proc/{}/stat", launcher.id())).unwrap();
    let start_ticks = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let record = serde_json::json!({
        "schema_version": 1,
        "id": "alpha",
        "generation_id": format!("sha256:{}", "a".repeat(64)),
        "qemu_pid": null,
        "qmp_socket": clone_dir.join("alpha.qmp.sock"),
        "disk_overlay": clone_dir.join("disk.qcow2"),
        "machine_id": "0123456789abcdef0123456789abcdef",
        "mac": "02:00:00:00:00:01",
        "state": "starting",
        "launcher_pid": launcher.id(),
        "launcher_start_ticks": start_ticks,
        "launcher_boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim(),
    });
    fs::write(
        clone_dir.join("clone.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();

    let live = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "reconcile", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(live.status.success());
    let records: Value = serde_json::from_slice(&live.stdout).unwrap();
    assert_eq!(records[0]["state"], "starting");

    launcher.kill().unwrap();
    launcher.wait().unwrap();
    let abandoned = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "reconcile", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(abandoned.status.success());
    let records: Value = serde_json::from_slice(&abandoned.stdout).unwrap();
    assert_eq!(records[0]["state"], "failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn clone_reconcile_ignores_a_reused_pid_for_a_starting_qemu() {
    let root = temp_dir();
    let state_dir = root.join("state");
    let clone_dir = state_dir.join("alpha");
    fs::create_dir_all(&clone_dir).unwrap();
    let mut unrelated_process = Command::new("sleep").arg("30").spawn().unwrap();
    let record = serde_json::json!({
        "schema_version": 1,
        "id": "alpha",
        "generation_id": format!("sha256:{}", "a".repeat(64)),
        "qemu_pid": unrelated_process.id(),
        "qemu_start_ticks": 1,
        "qemu_boot_id": "previous-boot",
        "launcher_pid": u32::MAX,
        "launcher_start_ticks": 1,
        "launcher_boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim(),
        "qmp_socket": clone_dir.join("alpha.qmp.sock"),
        "disk_overlay": clone_dir.join("disk.qcow2"),
        "machine_id": "0123456789abcdef0123456789abcdef",
        "mac": "02:00:00:00:00:01",
        "state": "starting"
    });
    fs::write(
        clone_dir.join("clone.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "reconcile", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    unrelated_process.kill().unwrap();
    unrelated_process.wait().unwrap();

    assert!(output.status.success());
    let records: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(records[0]["state"], "failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn clone_delete_removes_stopped_artifacts_and_refuses_live_or_ambiguous_clones() {
    let root = temp_dir();
    let state_dir = root.join("state");
    let registry = CloneRegistry::open(&state_dir).unwrap();

    let mut stopped = CloneRecord {
        schema_version: 1,
        id: "stopped".into(),
        generation_id: format!("sha256:{}", "a".repeat(64)),
        qemu_pid: None,
        qemu_start_ticks: None,
        qemu_boot_id: None,
        launcher_pid: None,
        launcher_start_ticks: None,
        launcher_boot_id: None,
        qmp_socket: state_dir.join("stopped/stopped.qmp.sock"),
        disk_overlay: state_dir.join("stopped/disk.qcow2"),
        machine_id: "0123456789abcdef0123456789abcdef".into(),
        mac: "02:00:00:00:00:01".into(),
        state: CloneState::Starting,
    };
    registry.create(&stopped).unwrap();
    registry
        .transition(
            "stopped",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();
    stopped = registry
        .transition("stopped", CloneState::Running, CloneState::Stopped, None)
        .unwrap();
    fs::write(&stopped.disk_overlay, b"private overlay").unwrap();
    fs::write(state_dir.join("stopped/qemu.log"), b"stopped clone").unwrap();

    let deleted = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "delete", "stopped", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "delete failed: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert!(!state_dir.join("stopped").exists());

    let mut running = stopped.clone();
    running.id = "running".into();
    running.qmp_socket = state_dir.join("running/running.qmp.sock");
    running.disk_overlay = state_dir.join("running/disk.qcow2");
    running.machine_id = "1123456789abcdef0123456789abcdef".into();
    running.mac = "02:00:00:00:00:02".into();
    running.state = CloneState::Starting;
    registry.create(&running).unwrap();
    registry
        .transition(
            "running",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "delete", "running", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(state_dir.join("running/clone.json").exists());

    let mut ambiguous = stopped;
    ambiguous.id = "ambiguous".into();
    ambiguous.qmp_socket = state_dir.join("ambiguous/ambiguous.qmp.sock");
    ambiguous.disk_overlay = state_dir.join("ambiguous/disk.qcow2");
    ambiguous.machine_id = "2123456789abcdef0123456789abcdef".into();
    ambiguous.mac = "02:00:00:00:00:03".into();
    ambiguous.state = CloneState::Starting;
    registry.create(&ambiguous).unwrap();
    registry
        .transition(
            "ambiguous",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();
    registry
        .transition("ambiguous", CloneState::Running, CloneState::Stopped, None)
        .unwrap();
    let listener = UnixListener::bind(&ambiguous.qmp_socket).unwrap();
    let server = thread::spawn(move || listener.accept().unwrap());
    let refused = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "delete", "ambiguous", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(state_dir.join("ambiguous/clone.json").exists());
    drop(server.join().unwrap());

    let mut outside = ambiguous;
    outside.id = "outside".into();
    outside.qmp_socket = state_dir.join("outside/outside.qmp.sock");
    outside.disk_overlay = root.join("outside.qcow2");
    outside.machine_id = "3123456789abcdef0123456789abcdef".into();
    outside.mac = "02:00:00:00:00:04".into();
    registry.create(&outside).unwrap();
    registry
        .transition(
            "outside",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();
    registry
        .transition("outside", CloneState::Running, CloneState::Stopped, None)
        .unwrap();
    fs::write(&outside.disk_overlay, b"must survive").unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "delete", "outside", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(outside.disk_overlay.exists());

    let mut failed = outside.clone();
    failed.id = "failed".into();
    failed.qmp_socket = state_dir.join("failed/failed.qmp.sock");
    failed.disk_overlay = state_dir.join("failed/disk.qcow2");
    failed.machine_id = "4123456789abcdef0123456789abcdef".into();
    failed.mac = "02:00:00:00:00:05".into();
    failed.state = CloneState::Starting;
    registry.create(&failed).unwrap();
    registry
        .transition(
            "failed",
            CloneState::Starting,
            CloneState::Running,
            Some(u32::MAX),
        )
        .unwrap();
    registry.reconcile().unwrap();
    fs::write(&failed.disk_overlay, b"orphaned overlay").unwrap();
    let deleted = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["clone", "delete", "failed", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "failed clone delete failed: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert!(!state_dir.join("failed").exists());
    fs::remove_dir_all(root).unwrap();
}
