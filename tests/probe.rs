use serde_json::Value;
use std::process::Command;
use std::time::SystemTime;

#[test]
#[ignore = "requires local KVM, host kernel and QEMU 11.1.1"]
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
