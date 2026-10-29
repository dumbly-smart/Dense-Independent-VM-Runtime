use densevm::cli::{BaselineAccelerator, Command, parse_args};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn parses_probe_with_count_and_output() {
    assert_eq!(
        parse_args(["densevm", "probe", "--out", "/tmp/run", "--count", "8"]).unwrap(),
        Command::Probe {
            out: PathBuf::from("/tmp/run"),
            count: 8,
            vcpus: 1,
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
            vcpus: 1,
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
fn accepts_only_supported_probe_vcpu_counts() {
    assert!(matches!(
        parse_args(["densevm", "probe", "--out", "/tmp/run", "--vcpus", "2"]),
        Ok(Command::Probe { vcpus: 2, .. })
    ));
    for count in ["0", "5"] {
        assert!(parse_args(["densevm", "probe", "--out", "/tmp/run", "--vcpus", count]).is_err());
    }
}

#[test]
fn parses_persistent_clone_listing() {
    assert_eq!(
        parse_args([
            "densevm",
            "clone",
            "list",
            "--state-dir",
            "/var/lib/densevm"
        ]),
        Ok(Command::CloneList {
            state_dir: PathBuf::from("/var/lib/densevm"),
        })
    );
    assert!(parse_args(["densevm", "clone", "list"]).is_err());
}

#[test]
fn parses_template_tree_validation() {
    assert_eq!(
        parse_args(["densevm", "validate", "template-tree", "/templates"]),
        Ok(Command::ValidateTemplateTree {
            root: PathBuf::from("/templates"),
        })
    );
}

#[test]
fn failed_template_tree_validation_prints_report_and_exits_nonzero() {
    let root = std::env::temp_dir().join(format!(
        "densevm-invalid-template-tree-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let generation = root.join("generation-a");
    fs::create_dir_all(&generation).unwrap();
    fs::write(
        generation.join("generation.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "generation_id": "sha256:incomplete",
            "parent": {"template_dir": "/missing/parent"},
            "ram": {"path": "ram.img", "sha256": "invalid", "logical_size_bytes": 4096},
            "machine_state": {"path": "state", "sha256": "invalid"},
            "disk": {"path": "disk.qcow2", "sha256": "invalid", "backing_path": "/missing/parent/base.qcow2"}
        }))
        .unwrap(),
    )
    .unwrap();

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["validate", "template-tree"])
        .arg(&root)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!output.status.success());
    assert_eq!(report["status"], "failed");
    assert_eq!(report["generations"][0]["validation"]["status"], "failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parses_baseline_accelerator_with_kvm_default_and_tcg_option() {
    assert_eq!(
        parse_args(["densevm", "benchmark", "baseline", "--out", "/tmp/base"]),
        Ok(Command::BenchmarkBaseline {
            out: PathBuf::from("/tmp/base"),
            count: 1,
            accelerator: BaselineAccelerator::Kvm,
            trace_translations: false,
        })
    );
    assert_eq!(
        parse_args([
            "densevm",
            "benchmark",
            "baseline",
            "--out",
            "/tmp/tcg",
            "--count",
            "2",
            "--accel",
            "tcg",
        ]),
        Ok(Command::BenchmarkBaseline {
            out: PathBuf::from("/tmp/tcg"),
            count: 2,
            accelerator: BaselineAccelerator::Tcg,
            trace_translations: false,
        })
    );
    assert!(
        parse_args([
            "densevm",
            "benchmark",
            "baseline",
            "--out",
            "/tmp/invalid",
            "--accel",
            "other",
        ])
        .is_err()
    );
}

#[test]
fn enables_translation_trace_only_for_tcg_baselines() {
    assert_eq!(
        parse_args([
            "densevm",
            "benchmark",
            "baseline",
            "--out",
            "/tmp/tcg-trace",
            "--accel",
            "tcg",
            "--trace-translations",
        ]),
        Ok(Command::BenchmarkBaseline {
            out: PathBuf::from("/tmp/tcg-trace"),
            count: 1,
            accelerator: BaselineAccelerator::Tcg,
            trace_translations: true,
        })
    );
    assert!(
        parse_args([
            "densevm",
            "benchmark",
            "baseline",
            "--out",
            "/tmp/kvm-trace",
            "--trace-translations",
        ])
        .is_err()
    );
}

#[test]
fn parses_clone_reconciliation_with_an_explicit_state_directory() {
    assert_eq!(
        parse_args([
            "densevm",
            "clone",
            "reconcile",
            "--state-dir",
            "/var/lib/densevm"
        ]),
        Ok(Command::CloneReconcile {
            state_dir: PathBuf::from("/var/lib/densevm"),
        })
    );
    assert!(parse_args(["densevm", "clone", "reconcile"]).is_err());
}

#[test]
fn parses_clone_stop_with_an_explicit_state_directory() {
    assert_eq!(
        parse_args([
            "densevm",
            "clone",
            "stop",
            "alpha",
            "--state-dir",
            "/var/lib/densevm",
        ]),
        Ok(Command::CloneStop {
            id: "alpha".into(),
            state_dir: PathBuf::from("/var/lib/densevm"),
        })
    );
}

#[test]
fn parses_clone_delete_with_an_explicit_state_directory() {
    assert_eq!(
        parse_args([
            "densevm",
            "clone",
            "delete",
            "alpha",
            "--state-dir",
            "/var/lib/densevm",
        ]),
        Ok(Command::CloneDelete {
            id: "alpha".into(),
            state_dir: PathBuf::from("/var/lib/densevm"),
        })
    );
}

#[test]
fn parses_clone_start_from_a_verified_template_manifest() {
    assert_eq!(
        parse_args([
            "densevm",
            "clone",
            "start",
            "alpha",
            "--template",
            "/templates/root/generation.json",
            "--state-dir",
            "/var/lib/densevm",
        ]),
        Ok(Command::CloneStart {
            id: "alpha".into(),
            manifest: PathBuf::from("/templates/root/generation.json"),
            state_dir: PathBuf::from("/var/lib/densevm"),
        })
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
fn parses_template_tree_catalog_command() {
    assert_eq!(
        parse_args(["densevm", "template", "tree", "/tmp/templates"]),
        Ok(Command::TemplateTree {
            root: "/tmp/templates".into(),
        })
    );
    assert!(parse_args(["densevm", "template", "tree"]).is_err());
}

#[test]
fn parses_read_only_template_gc_plan() {
    assert_eq!(
        parse_args([
            "densevm",
            "template",
            "gc-plan",
            "/templates",
            "--state-dir",
            "/var/lib/densevm",
            "--keep",
            "sha256:one",
            "--keep",
            "sha256:two",
        ]),
        Ok(Command::TemplateGcPlan {
            root: "/templates".into(),
            state_dir: "/var/lib/densevm".into(),
            keep_ids: vec!["sha256:one".into(), "sha256:two".into()],
        })
    );
    assert!(parse_args(["densevm", "template", "gc-plan", "/templates"]).is_err());
}

#[test]
fn blocked_template_gc_plan_prints_report_and_exits_nonzero() {
    let root = std::env::temp_dir().join(format!(
        "densevm-blocked-gc-plan-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let catalog = root.join("catalog");
    let state_dir = root.join("state");
    fs::create_dir_all(&catalog).unwrap();
    fs::create_dir_all(&state_dir).unwrap();

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_densevm"))
        .args(["template", "gc-plan"])
        .arg(&catalog)
        .args(["--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(!output.status.success());
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["candidate_generation_ids"], serde_json::json!([]));
    assert!(report["blockers"].as_array().unwrap().len() > 0);
    assert_eq!(fs::read_dir(&state_dir).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
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
