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
    let clocks = result["parking"]["guest_clock_samples"].as_array().unwrap();
    assert_eq!(clocks.len(), 2);
    for sample in clocks {
        assert!(sample.get("local_timer_interrupts_before").is_some());
        assert!(sample.get("local_timer_interrupts_after").is_some());
        assert!(sample.get("rtc_epoch_before_seconds").is_some());
        assert!(sample.get("rtc_epoch_after_seconds").is_some());
    }
    assert!(result["ram_isolation"].as_bool().unwrap());
    assert!(result["disk_isolation"].as_bool().unwrap());
}

#[test]
#[ignore = "requires local KVM and QEMU; captures guest general-purpose registers with QMP"]
fn clones_report_independent_vcpu_register_snapshots() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = std::env::temp_dir().join(format!("densevm-vcpu-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args([
            "probe",
            "--skip-guest-oom",
            "--vcpu-register-state",
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
    assert_eq!(result["vcpu_register_state_verified"], true);
    assert_eq!(result["process_isolation"], Value::Null);
    assert_eq!(result["process_isolation_status"], "not-requested");
    assert_eq!(result["vcpu_register_state"]["status"], "verified");
    let a = &result["vcpu_register_state"]["a"];
    let b = &result["vcpu_register_state"]["b"];
    assert_eq!(a["gs_base_expected"], a["gs_base_observed"]);
    assert_eq!(b["gs_base_expected"], b["gs_base_observed"]);
    assert!(
        a["lapic_output"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert!(
        b["lapic_output"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert_ne!(a["gs_base_observed"], b["gs_base_observed"]);
    assert!(
        result["vcpu_register_state"]["a"]["registers"]
            .as_object()
            .unwrap()
            .len()
            >= 14
    );
    assert!(
        result["vcpu_register_state"]["b"]["registers"]
            .as_object()
            .unwrap()
            .len()
            >= 14
    );
    assert!(
        !result["vcpu_register_state"]["differing_general_purpose_registers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "requires local KVM and guest x2APIC MSR or xAPIC /dev/mem access"]
fn clones_keep_guest_lapic_tpr_writes_private() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out =
        std::env::temp_dir().join(format!("densevm-lapic-tpr-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args([
            "probe",
            "--skip-guest-oom",
            "--lapic-tpr-state",
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
    assert_eq!(result["vcpu_register_state_verified"], true);
    assert_eq!(
        result["vcpu_register_state"]["lapic_tpr_state"]["status"],
        "verified"
    );
    let tpr = &result["vcpu_register_state"]["lapic_tpr_state"];
    assert_eq!(tpr["observed"]["a_after_a_write"], tpr["a"]["target_tpr"]);
    assert_eq!(tpr["observed"]["b_after_a_write"], tpr["b"]["original_tpr"]);
    assert!(
        tpr["observed"]["a_after_b_write"] == tpr["a"]["target_tpr"]
            || tpr["observed"]["a_after_b_write"] == tpr["a"]["original_tpr"]
    );
    assert_ne!(tpr["observed"]["a_after_b_write"], tpr["b"]["target_tpr"]);
    assert_eq!(tpr["observed"]["b_after_b_write"], tpr["b"]["target_tpr"]);
    assert_eq!(tpr["observed"]["a_restored"], tpr["a"]["original_tpr"]);
    assert_eq!(tpr["observed"]["b_restored"], tpr["b"]["original_tpr"]);
}

#[test]
#[ignore = "requires local KVM and QEMU; compares running and QMP-paused RTC alarm delivery"]
fn probe_records_guest_rtc_alarm_delivery_during_running_and_paused_trials() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out =
        std::env::temp_dir().join(format!("densevm-rtc-alarm-{}-{stamp}", std::process::id()));
    let status = Command::new(env!("CARGO_BIN_EXE_densevm"))
        .args([
            "probe",
            "--skip-guest-oom",
            "--rtc-alarm",
            "--divergence-percent",
            "0",
            "--out",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    let result: Value =
        serde_json::from_slice(&std::fs::read(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["rtc_alarm"]["rtc_alarm_flag"], "RTC_AF (0x20)");
    for clone in ["a", "b"] {
        assert!(
            result["rtc_alarm"]["control_running"][clone]["response"]
                .as_str()
                .is_some()
        );
        assert!(
            result["rtc_alarm"]["paused"][clone]["response"]
                .as_str()
                .is_some()
        );
        if result["rtc_alarm"]["paused"][clone]["alarm_flag_observed"] == false {
            let response = result["rtc_alarm"]["paused"][clone]["response"]
                .as_str()
                .unwrap();
            assert!(response.contains("rtc_now="));
            assert!(response.contains("rtc_alarm="));
            assert!(response.contains("alarm_IRQ"));
            assert!(response.contains("alrm_pending"));
        }
    }
    if status.success() {
        assert_eq!(result["rtc_alarm_verified"], true);
    } else {
        assert_eq!(result["rtc_alarm_verified"], false);
    }
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
