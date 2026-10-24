use serde_json::Value;
use std::process::Command;
use std::time::SystemTime;

#[test]
#[ignore = "requires local KVM and the inspected Linux/QEMU host"]
fn two_clones_resume_and_isolate_ram_and_disk() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = std::env::temp_dir().join(format!("densevm-probe-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["probe", "--out", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "probe failed; artifacts at {}",
        out.display()
    );
    let result: Value =
        serde_json::from_slice(&std::fs::read(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["ram_isolation"], true);
    assert_eq!(result["disk_isolation"], true);
    assert_eq!(result["template_ram_unchanged"], true);
    assert_eq!(result["backing_verified"], true);
    assert_eq!(result["vmgenid_unique"], true);
    assert_eq!(result["identity_unique"], true);
    assert_eq!(result["ssh_host_keys"]["unique"], true);
    assert_eq!(result["guest_oom"]["isolation_verified"], true);
    assert!(
        result["template"]["qemu_command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "-object")
    );
    assert!(
        result["clones"]["a"]["qemu_command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "-incoming")
    );
    assert!(
        result["template"]["memory_file_resident_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(result["clones"]["a"]["memory"]["pss_kib"].as_u64().unwrap() > 0);
    assert!(result["clones"]["b"]["memory"]["pss_kib"].as_u64().unwrap() > 0);
    let a = &result["clones"]["a"];
    let b = &result["clones"]["b"];
    let a_shared = a["ram_mapping"]["shared_clean_kib"].as_u64().unwrap()
        + a["ram_mapping"]["shared_dirty_kib"].as_u64().unwrap();
    assert!(
        a_shared > 16 * 1024,
        "initial RAM mapping did not show shared pages"
    );
    let a_private_before = a["ram_mapping"]["private_dirty_kib"].as_u64().unwrap();
    let a_private_after = a["after_ram_write_ram_mapping"]["private_dirty_kib"]
        .as_u64()
        .unwrap();
    let b_private_before = b["ram_mapping"]["private_dirty_kib"].as_u64().unwrap();
    let b_private_after = b["after_a_ram_write_ram_mapping"]["private_dirty_kib"]
        .as_u64()
        .unwrap();
    assert!(a_private_after >= a_private_before + 24 * 1024);
    assert!(b_private_after < b_private_before + 4096);
}

#[test]
#[ignore = "requires local KVM and QEMU; runs the timerfd pause/resume experiment"]
fn parked_clones_report_guest_timerfd_samples() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = std::env::temp_dir().join(format!("densevm-timerfd-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args([
            "probe",
            "--skip-guest-oom",
            "--park-idle",
            "--divergence-percent",
            "0",
            "--out",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "probe failed; artifacts at {}",
        out.display()
    );
    let result: Value =
        serde_json::from_slice(&std::fs::read(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["parking_verified"], true);
    let samples = result["parking"]["guest_timerfd_expiration_samples"]
        .as_array()
        .unwrap();
    assert_eq!(samples.len(), 2);
    for sample in samples {
        assert!(sample["expirations_before_pause"].as_u64().is_some());
        assert!(sample["expirations_after_resume"].as_u64().is_some());
        assert!(sample["observed_delta"].as_u64().is_some());
    }
    assert!(result["ram_isolation"].as_bool().unwrap());
    assert!(result["disk_isolation"].as_bool().unwrap());
}

#[test]
#[ignore = "requires local KVM and three-clone memory budget"]
fn guest_kernel_crash_does_not_kill_sibling_clones() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = std::env::temp_dir().join(format!("densevm-crash-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args([
            "probe",
            "--count",
            "3",
            "--divergence-percent",
            "0",
            "--out",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "probe failed; artifacts at {}",
        out.display()
    );
    let result: Value =
        serde_json::from_slice(&std::fs::read(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["guest_crash"]["crash_observed"], true);
    assert_eq!(result["guest_crash"]["crashed_clone_qemu_alive"], true);
    assert_eq!(result["guest_crash"]["other_clones_survived"], true);
    assert_eq!(result["guest_crash"]["isolation_verified"], true);
}

#[test]
#[ignore = "requires local KVM and QEMU; sends SIGTERM only to its own supervisor"]
fn interruption_reaps_owned_qemu_processes() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out =
        std::env::temp_dir().join(format!("densevm-interrupt-{}-{stamp}", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["probe", "--out", out.to_str().unwrap()])
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !out.join("baseline-000.qmp.sock").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "probe exited before child was launched"
        );
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("QEMU did not start");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("interrupted probe failed to exit");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        if let Ok(bytes) = std::fs::read(entry.path().join("cmdline")) {
            let command = String::from_utf8_lossy(&bytes);
            let program = command.split('\0').next().unwrap_or("");
            assert!(
                !(program.ends_with("qemu-system-x86_64")
                    && command.contains(out.to_str().unwrap())),
                "leaked QEMU: {command}"
            );
        }
    }
}
