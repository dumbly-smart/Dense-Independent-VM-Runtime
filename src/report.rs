use serde_json::{Value, json};
use std::path::Path;

fn number(value: &Value, field: &str) -> Result<u64, String> {
    value[field]
        .as_u64()
        .ok_or_else(|| format!("missing or invalid measurement {field}"))
}

fn sum_memory(vms: &[Value], field: &str) -> Result<u64, String> {
    vms.iter().try_fold(0u64, |sum, vm| {
        sum.checked_add(number(&vm["memory"], field)?)
            .ok_or_else(|| "memory total overflow".to_string())
    })
}

fn sum_ram_mapping(clones: &[Value], field: &str) -> Result<u64, String> {
    clones.iter().try_fold(0u64, |sum, clone| {
        sum.checked_add(number(&clone["ram_mapping"], field)?)
            .ok_or_else(|| "template RAM mapping total overflow".to_string())
    })
}

fn snapshot_summary(snapshot: &Value, count: u64) -> Result<Value, String> {
    let clones = snapshot["clones"]
        .as_array()
        .filter(|vms| vms.len() as u64 == count)
        .ok_or("snapshot clone count mismatch")?;
    let pss = sum_memory(clones, "pss_kib")?;
    let cache = number(snapshot, "template_file_resident_bytes")? / 1024;
    let shared_clean = sum_ram_mapping(clones, "shared_clean_kib")?;
    let shared_dirty = sum_ram_mapping(clones, "shared_dirty_kib")?;
    let private_clean = sum_ram_mapping(clones, "private_clean_kib")?;
    let private_dirty = sum_ram_mapping(clones, "private_dirty_kib")?;
    let ram_mapping = json!({
        "pss_kib":sum_ram_mapping(clones, "pss_kib")?,
        "shared_clean_kib":shared_clean,
        "shared_dirty_kib":shared_dirty,
        "shared_mapped_kib":shared_clean.checked_add(shared_dirty).ok_or("template RAM mapping total overflow")?,
        "private_clean_kib":private_clean,
        "private_dirty_kib":private_dirty,
        "private_kib":private_clean.checked_add(private_dirty).ok_or("template RAM mapping total overflow")?,
        "accounting":"sum across clone mappings of the template RAM file; shared mapped pages are counted once per clone mapping, while mapping PSS apportions shared pages",
    });
    Ok(json!({
        "process_pss_kib":pss,
        "process_rss_kib":sum_memory(clones, "rss_kib")?,
        "process_private_kib":sum_memory(clones, "private_clean_kib")? + sum_memory(clones, "private_dirty_kib")?,
        "template_ram_mapping":ram_mapping,
        "template_file_resident_kib":cache,
        "process_pss_plus_full_template_cache_kib":pss.checked_add(cache).ok_or("memory total overflow")?,
        "cache_accounting":"PSS plus full template cache intentionally double-counts mapped template pages; this is a conservative user-space bound, not total host memory",
    }))
}

pub fn summarize(result: &Value) -> Result<Value, String> {
    let count = number(&result["clones"], "count")?;
    if count == 0 || count > 32 || number(&result["baseline"], "count")? != count {
        return Err("baseline and clone counts must match and be between 1 and 32".into());
    }
    let baseline = result["baseline"]["vms"]
        .as_array()
        .filter(|vms| vms.len() as u64 == count)
        .ok_or("baseline VM count mismatch")?;
    let baseline_pss = sum_memory(baseline, "pss_kib")?;
    let before = snapshot_summary(&result["snapshots"]["before_divergence"], count)?;
    let after = snapshot_summary(&result["snapshots"]["after_divergence"], count)?;
    let configured = number(result, "guest_ram_mib")?
        .checked_mul(count)
        .and_then(|v| v.checked_mul(1024))
        .ok_or("configured RAM total overflow")?;
    let cgroup_memory = json!({
        "baseline":result["baseline"].get("cgroup_memory_at_measurement").cloned().unwrap_or(json!({"status":"not-recorded"})),
        "clones_before_divergence":result["snapshots"]["before_divergence"].get("cgroup_memory").cloned().unwrap_or(json!({"status":"not-recorded"})),
        "clones_after_divergence":result["snapshots"]["after_divergence"].get("cgroup_memory").cloned().unwrap_or(json!({"status":"not-recorded"})),
        "scope":"cgroup-v2 memory.current includes anonymous memory, file cache, and memory.stat kernel charges in the measured cgroup; it may not include external or uncharged KVM allocations",
    });
    Ok(json!({
        "clone_count":count,
        "configured_guest_ram_kib":configured,
        "baseline_process_pss_kib":baseline_pss,
        "before_divergence":before,
        "after_divergence":after,
        "cgroup_memory":cgroup_memory,
        "divergence_percent":result["effective_ram_divergence_percent"],
        "divergence_region_bytes":result["divergence_region_bytes"],
        "divergence_scope":result["divergence_scope"],
        "disk_storage":result["disk_storage"],
        "host":result["host"],
        "kernel_memory_overhead":"process PSS excludes kernel memory; cgroup.memory_stat reports charges visible to its cgroup; coverage of KVM and external allocations remains UNKNOWN",
        "baseline_cache_accounting":"baseline process PSS excludes unmapped disk/kernel/initramfs file cache",
    }))
}

pub fn from_path(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let result: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    summarize(&result)
}
