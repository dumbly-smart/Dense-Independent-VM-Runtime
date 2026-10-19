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
            park_idle: false
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
            park_idle: false
        }
    );
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
