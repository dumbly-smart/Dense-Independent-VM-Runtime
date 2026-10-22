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
    assert_eq!(report["configured_guest_ram_kib"], 512 * 1024);
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
