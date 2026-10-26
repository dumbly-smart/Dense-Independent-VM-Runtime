use densevm::report::summarize;
use serde_json::{Value, json};

fn memory(pss: u64) -> Value {
    json!({"pss_kib":pss,"rss_kib":pss+10,"private_clean_kib":0,"private_dirty_kib":20,"shared_clean_kib":10,"shared_dirty_kib":0})
}

fn result() -> Value {
    json!({"guest_ram_mib":256,"baseline":{"count":2,"vms":[{"memory":memory(1000)},{"memory":memory(1000)}]},"clones":{"count":2},"snapshots":{
        "before_divergence":{"template_file_resident_bytes":1024,"clones":[{"memory":memory(500),"ram_mapping":memory(100)},{"memory":memory(500),"ram_mapping":memory(100)}]},
        "after_divergence":{"template_file_resident_bytes":1024,"clones":[{"memory":memory(600),"ram_mapping":memory(200)},{"memory":memory(600),"ram_mapping":memory(200)}]}
    }})
}

#[test]
fn reports_measured_pss_and_a_separately_labeled_cache_upper_bound() {
    let report = summarize(&result()).unwrap();
    assert_eq!(report["baseline_process_pss_kib"], 2000);
    assert_eq!(report["before_divergence"]["process_pss_kib"], 1000);
    assert_eq!(report["after_divergence"]["process_pss_kib"], 1200);
    assert_eq!(
        report["before_divergence"]["process_pss_plus_full_template_cache_kib"],
        1001
    );
    assert_eq!(report["before_divergence"]["process_private_kib"], 40);
    assert_eq!(
        report["before_divergence"]["template_ram_mapping"]["pss_kib"],
        200
    );
    assert_eq!(
        report["before_divergence"]["template_ram_mapping"]["shared_mapped_kib"],
        20
    );
    assert_eq!(
        report["after_divergence"]["template_ram_mapping"]["private_kib"],
        40
    );
    assert_eq!(report["configured_guest_ram_kib"], 512 * 1024);
    assert_eq!(
        report["cgroup_memory"]["baseline"]["status"],
        "not-recorded"
    );
}

#[test]
fn preserves_cgroup_memory_samples_for_baseline_and_clone_snapshots() {
    let mut data = result();
    data["baseline"]["cgroup_memory_at_measurement"] = json!({
        "status":"measured",
        "current_bytes":1_900_000,
        "memory_stat":{"anon":1_500_000,"kernel":100_000}
    });
    data["snapshots"]["before_divergence"]["cgroup_memory"] = json!({
        "status":"measured",
        "current_bytes":900_000,
        "memory_stat":{"anon":400_000,"file":450_000,"kernel":50_000}
    });
    data["snapshots"]["after_divergence"]["cgroup_memory"] = json!({
        "status":"unavailable",
        "reason":"scope was removed"
    });

    let report = summarize(&data).unwrap();

    assert_eq!(
        report["cgroup_memory"]["baseline"]["current_bytes"],
        1_900_000
    );
    assert_eq!(
        report["cgroup_memory"]["baseline"]["memory_stat"]["kernel"],
        100_000
    );
    assert_eq!(
        report["cgroup_memory"]["clones_before_divergence"]["current_bytes"],
        900_000
    );
    assert_eq!(
        report["cgroup_memory"]["clones_after_divergence"]["status"],
        "unavailable"
    );
}

#[test]
fn missing_measurements_are_errors_and_counts_must_match() {
    let mut data = result();
    data["snapshots"]["before_divergence"]["clones"][0]["memory"]
        .as_object_mut()
        .unwrap()
        .remove("pss_kib");
    assert!(summarize(&data).is_err());
    let mut data = result();
    data["clones"]["count"] = json!(8);
    assert!(summarize(&data).is_err());
}
