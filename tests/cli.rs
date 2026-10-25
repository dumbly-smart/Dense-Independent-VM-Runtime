use densevm::cli::{Command, parse_args};
use std::path::PathBuf;

#[test]
fn parses_probe_with_count_and_output() {
    assert_eq!(
        parse_args(["densevm", "probe", "--out", "/tmp/run", "--count", "8"]).unwrap(),
        Command::Probe {
            out: PathBuf::from("/tmp/run"),
            count: 8,
            ksm: false,
            divergence_pages: 8192,
            park_idle: false,
            divergence_all: false,
            guest_dir: None,
            range: false,
            skip_guest_oom: false,
            vcpu_register_state: false,
            lapic_tpr_state: false,
            rtc_alarm: false
        }
    );
    assert_eq!(
        parse_args([
            "densevm",
            "probe",
            "--out",
            "/tmp/run",
            "--divergence-mib",
            "8"
        ])
        .unwrap(),
        Command::Probe {
            out: PathBuf::from("/tmp/run"),
            count: 2,
            ksm: false,
            divergence_pages: 2048,
            park_idle: false,
            divergence_all: false,
            guest_dir: None,
            range: false,
            skip_guest_oom: false,
            vcpu_register_state: false,
            lapic_tpr_state: false,
            rtc_alarm: false
        }
    );
}

#[test]
fn supports_skipping_the_guest_oom_workload_for_targeted_identity_validation() {
    let result = parse_args([
        "densevm",
        "probe",
        "--out",
        "/tmp/run",
        "--count",
        "2",
        "--skip-guest-oom",
    ]);
    assert!(matches!(
        result,
        Ok(Command::Probe {
            skip_guest_oom: true,
            ..
        })
    ));
}

#[test]
fn enables_vcpu_register_sampling_only_when_requested() {
    let result = parse_args([
        "densevm",
        "probe",
        "--out",
        "/tmp/run",
        "--vcpu-register-state",
    ]);
    assert!(matches!(
        result,
        Ok(Command::Probe {
            vcpu_register_state: true,
            ..
        })
    ));
}

#[test]
fn lapic_tpr_probe_enables_register_capture() {
    let result = parse_args(["densevm", "probe", "--out", "/tmp/run", "--lapic-tpr-state"]);
    assert!(matches!(
        result,
        Ok(Command::Probe {
            vcpu_register_state: true,
            lapic_tpr_state: true,
            ..
        })
    ));
}

#[test]
fn enables_rtc_alarm_sampling_only_when_requested() {
    let result = parse_args(["densevm", "probe", "--out", "/tmp/run", "--rtc-alarm"]);
    assert!(matches!(
        result,
        Ok(Command::Probe {
            rtc_alarm: true,
            ..
        })
    ));
}

#[test]
fn parses_all_clone_divergence() {
    let result = parse_args([
        "densevm",
        "probe",
        "--out",
        "/tmp/run",
        "--count",
        "8",
        "--divergence-percent",
        "25",
        "--divergence-all",
    ])
    .unwrap();
    assert!(matches!(
        result,
        Command::Probe {
            count: 8,
            divergence_pages: 2048,
            divergence_all: true,
            ..
        }
    ));
}

#[test]
fn accepts_an_explicit_prepared_distribution_guest() {
    let result = parse_args([
        "densevm",
        "probe",
        "--out",
        "/tmp/run",
        "--guest",
        "/tmp/alpine",
    ])
    .unwrap();
    assert!(
        matches!(result, Command::Probe { guest_dir:Some(path), .. } if path == PathBuf::from("/tmp/alpine"))
    );
}

#[test]
fn accepts_a_shared_range_for_a_prepared_guest() {
    let result = parse_args([
        "densevm",
        "probe",
        "--out",
        "/tmp/run",
        "--guest",
        "/tmp/alpine",
        "--range",
        "--count",
        "6",
    ])
    .unwrap();
    assert!(matches!(
        result,
        Command::Probe {
            range: true,
            count: 6,
            ..
        }
    ));
}

#[test]
fn parses_inspection_commands() {
    assert_eq!(
        parse_args(["densevm", "inspect", "clone.qcow2", "disk"]).unwrap(),
        Command::InspectDisk {
            image: PathBuf::from("clone.qcow2")
        }
    );
    assert_eq!(
        parse_args(["densevm", "memory-map", "1234", "/tmp/memory.img"]).unwrap(),
        Command::MemoryMap {
            pid: 1234,
            path: PathBuf::from("/tmp/memory.img")
        }
    );
}

#[test]
fn parses_template_generation_inspection_command() {
    assert_eq!(
        parse_args(["densevm", "inspect", "template", "/tmp/generation.json"]),
        Ok(Command::InspectTemplate {
            manifest: "/tmp/generation.json".into(),
        })
    );
    assert!(
        parse_args([
            "densevm",
            "inspect",
            "template",
            "/tmp/generation.json",
            "extra"
        ])
        .is_err()
    );
}

#[test]
fn parses_template_generation_validation_command() {
    assert_eq!(
        parse_args(["densevm", "validate", "template", "/tmp/generation.json"]),
        Ok(Command::ValidateTemplate {
            manifest: "/tmp/generation.json".into(),
        })
    );
    assert!(
        parse_args([
            "densevm",
            "validate",
            "template",
            "/tmp/generation.json",
            "extra"
        ])
        .is_err()
    );
}

#[test]
fn rejects_unknown_commands_and_invalid_counts() {
    assert!(parse_args(["densevm", "unknown"]).is_err());
    for value in ["33", "not-a-number"] {
        assert!(
            parse_args([
                "densevm",
                "probe",
                "--out",
                "/tmp/run",
                "--divergence-mib",
                value
            ])
            .is_err(),
            "accepted invalid divergence size {value}"
        );
    }
    assert!(
        parse_args([
            "densevm",
            "probe",
            "--out",
            "/tmp/run",
            "--divergence-percent",
            "101"
        ])
        .is_err()
    );
    assert!(
        parse_args([
            "densevm",
            "benchmark",
            "baseline",
            "--count",
            "0",
            "--out",
            "/tmp/run"
        ])
        .is_err()
    );
}
