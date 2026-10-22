use densevm::capabilities::validate_template_support;
use serde_json::json;

#[test]
fn requires_ram_exclusion_and_writable_private_file_backend() {
    let migration = json!([{"capability":"x-ignore-shared","state":false}]);
    let properties =
        json!([{"name":"share"},{"name":"readonly"},{"name":"rom"},{"name":"mem-path"}]);
    assert!(validate_template_support(&migration, &properties).is_ok());
    assert!(
        validate_template_support(&json!([]), &properties)
            .unwrap_err()
            .contains("x-ignore-shared")
    );
    assert!(
        validate_template_support(&migration, &json!([{"name":"share"}]))
            .unwrap_err()
            .contains("readonly")
    );
    assert!(validate_template_support(&json!(null), &properties).is_err());
}

#[test]
fn doctor_reports_current_host_without_booting_a_guest() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_densevm"))
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["kernel_version"].is_string());
    assert!(report["page_size_bytes"].as_u64().unwrap() > 0);
    assert!(report["clock_ticks_per_second"].as_u64().unwrap() > 0);
    assert!(report["kvm"].get("accessible").is_some());
}
