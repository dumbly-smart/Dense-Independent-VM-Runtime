use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryTotals {
    pub rss_kib: u64,
    pub pss_kib: u64,
    pub shared_clean_kib: u64,
    pub shared_dirty_kib: u64,
    pub private_clean_kib: u64,
    pub private_dirty_kib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TcgTranslationSummary {
    pub translation_block_events: u64,
    pub unique_guest_pc_count: u64,
    pub events_at_guest_pcs_seen_multiple_times: u64,
    pub guest_pcs_seen_multiple_times: u64,
    pub max_events_for_one_guest_pc: u64,
    pub generated_block_events: Option<u64>,
    pub generated_host_code_bytes_cumulative: Option<u64>,
    pub translation_start_completion_event_count_delta: Option<u64>,
    pub generated_translation_wall_duration_us_cumulative: Option<u64>,
    pub generated_translation_wall_duration_us_max: Option<u64>,
    pub interpretation: &'static str,
}

pub fn parse_cgroup_memory(current: &str, stat: &str) -> Result<Value, String> {
    let current_bytes = current
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid cgroup memory.current: {error}"))?;
    let mut memory_stat = BTreeMap::new();
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        let key = fields
            .next()
            .ok_or_else(|| "empty cgroup memory.stat line".to_string())?;
        let value = fields
            .next()
            .ok_or_else(|| format!("cgroup memory.stat omitted value for {key}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid cgroup memory.stat {key}: {error}"))?;
        if fields.next().is_some() {
            return Err(format!("cgroup memory.stat has extra fields for {key}"));
        }
        if memory_stat.insert(key.to_string(), value).is_some() {
            return Err(format!("duplicate cgroup memory.stat key {key}"));
        }
    }
    if memory_stat.is_empty() {
        return Err("cgroup memory.stat was empty".to_string());
    }
    Ok(json!({
        "current_bytes":current_bytes,
        "memory_stat":memory_stat
    }))
}

impl MemoryTotals {
    fn zero() -> Self {
        Self {
            rss_kib: 0,
            pss_kib: 0,
            shared_clean_kib: 0,
            shared_dirty_kib: 0,
            private_clean_kib: 0,
            private_dirty_kib: 0,
        }
    }
    fn add(&mut self, other: Self) {
        self.rss_kib += other.rss_kib;
        self.pss_kib += other.pss_kib;
        self.shared_clean_kib += other.shared_clean_kib;
        self.shared_dirty_kib += other.shared_dirty_kib;
        self.private_clean_kib += other.private_clean_kib;
        self.private_dirty_kib += other.private_dirty_kib;
    }
}

pub fn estimate_ram_pages(totals: &MemoryTotals, page_size_bytes: u64) -> Result<Value, String> {
    if page_size_bytes == 0 {
        return Err("host page size must be positive".into());
    }
    let pages_for_kib = |kib: u64| -> Result<u64, String> {
        kib.checked_mul(1024)
            .and_then(|bytes| bytes.checked_add(page_size_bytes / 2))
            .map(|rounded| rounded / page_size_bytes)
            .ok_or_else(|| "RAM page estimate overflows".into())
    };
    let shared_kib = totals
        .shared_clean_kib
        .checked_add(totals.shared_dirty_kib)
        .ok_or("shared RAM KiB total overflows")?;
    let private_kib = totals
        .private_clean_kib
        .checked_add(totals.private_dirty_kib)
        .ok_or("private RAM KiB total overflows")?;
    Ok(json!({
        "page_size_bytes":page_size_bytes,
        "resident_pages_estimate":pages_for_kib(totals.rss_kib)?,
        "shared_pages_estimate":pages_for_kib(shared_kib)?,
        "private_pages_estimate":pages_for_kib(private_kib)?,
        "private_dirty_pages_estimate":pages_for_kib(totals.private_dirty_kib)?,
        "accounting":"rounded estimates derived from matching /proc/PID/smaps KiB totals; shared/private reflect smaps classifications, not guest-physical page positions; PSS is excluded because it apportions shared pages"
    }))
}

pub fn parse_smaps_rollup(input: &str) -> Result<MemoryTotals, String> {
    fn read(input: &str, key: &str) -> Result<u64, String> {
        let line = input
            .lines()
            .find(|line| line.starts_with(key))
            .ok_or_else(|| format!("missing {key}"))?;
        let mut fields = line[key.len()..].split_whitespace();
        let value = fields
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid {key}: {error}"))?;
        if fields.next() != Some("kB") {
            return Err(format!("invalid unit for {key}"));
        }
        Ok(value)
    }
    Ok(MemoryTotals {
        rss_kib: read(input, "Rss:")?,
        pss_kib: read(input, "Pss:")?,
        shared_clean_kib: read(input, "Shared_Clean:")?,
        shared_dirty_kib: read(input, "Shared_Dirty:")?,
        private_clean_kib: read(input, "Private_Clean:")?,
        private_dirty_kib: read(input, "Private_Dirty:")?,
    })
}

pub fn parse_smaps_for_path(input: &str, path: &str) -> Result<MemoryTotals, String> {
    let mut totals = MemoryTotals::zero();
    let mut section = String::new();
    let mut selected = false;
    let mut found = false;
    for line in input.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        let header = first.split_once('-').is_some_and(|(a, b)| {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|x| x.is_ascii_hexdigit())
                && b.bytes().all(|x| x.is_ascii_hexdigit())
        });
        if header {
            if selected {
                totals.add(parse_smaps_rollup(&section)?);
                section.clear();
            }
            selected = mapping_path(line) == Some(path);
            found |= selected;
        } else if selected {
            section.push_str(line);
            section.push('\n');
        }
    }
    if selected {
        totals.add(parse_smaps_rollup(&section)?);
    }
    if found {
        Ok(totals)
    } else {
        Err(format!("no smaps mapping for {path}"))
    }
}

fn mapping_path(line: &str) -> Option<&str> {
    let mut rest = line;
    for _ in 0..5 {
        rest = rest.trim_start();
        rest = &rest[rest.find(char::is_whitespace)?..];
    }
    Some(rest.trim_start())
}

pub fn verify_private_ram_mapping(input: &str, path: &str) -> Result<(), String> {
    let mut found = false;
    for line in input
        .lines()
        .filter(|line| mapping_path(line) == Some(path))
    {
        found = true;
        if line.split_whitespace().nth(1) != Some("rw-p") {
            return Err(format!("clone RAM must be writable MAP_PRIVATE: {line}"));
        }
    }
    if !found {
        return Err(format!("no smaps mapping for {path}"));
    }
    Ok(())
}

pub fn parse_tcg_translation_events(input: &str) -> Result<u64, String> {
    Ok(parse_tcg_translation_summary(input)?.translation_block_events)
}

pub fn parse_tcg_translation_summary(input: &str) -> Result<TcgTranslationSummary, String> {
    let mut events = 0u64;
    let mut events_by_guest_pc = BTreeMap::<u64, u64>::new();
    let mut generated_block_events = 0u64;
    let mut generated_host_code_bytes_cumulative = 0u64;
    let mut duration_events = 0u64;
    let mut generated_translation_wall_duration_us_cumulative = 0u64;
    let mut generated_translation_wall_duration_us_max = 0u64;
    for line in input.lines() {
        if line.starts_with("translate_block ") {
            events = events
                .checked_add(1)
                .ok_or_else(|| "translate_block event count exceeds u64".to_string())?;
            let pc = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix("pc:"))
                .and_then(|field| field.trim_end_matches(',').strip_prefix("0x"))
                .ok_or_else(|| {
                    format!("translate_block event omitted a hexadecimal guest PC: {line}")
                })?;
            let pc = u64::from_str_radix(pc, 16)
                .map_err(|error| format!("invalid guest PC in translate_block event: {error}"))?;
            let count = events_by_guest_pc.entry(pc).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| "per-PC translation event count exceeds u64".to_string())?;
        } else if line.starts_with("translate_block_complete ") {
            let size = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix("code_size:"))
                .and_then(|field| field.trim_end_matches(',').parse::<u64>().ok())
                .ok_or_else(|| {
                    format!("translate_block_complete omitted a valid code_size: {line}")
                })?;
            generated_block_events = generated_block_events
                .checked_add(1)
                .ok_or_else(|| "generated block event count exceeds u64".to_string())?;
            generated_host_code_bytes_cumulative = generated_host_code_bytes_cumulative
                .checked_add(size)
                .ok_or_else(|| "generated host-code byte count exceeds u64".to_string())?;
            if let Some(duration) = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix("duration_us:"))
            {
                let duration = duration
                    .trim_end_matches(',')
                    .parse::<u64>()
                    .map_err(|error| {
                        format!("invalid duration_us in translate_block_complete event: {error}")
                    })?;
                duration_events = duration_events.checked_add(1).ok_or_else(|| {
                    "generated block duration event count exceeds u64".to_string()
                })?;
                generated_translation_wall_duration_us_cumulative =
                    generated_translation_wall_duration_us_cumulative
                        .checked_add(duration)
                        .ok_or_else(|| {
                            "cumulative TCG translation duration exceeds u64".to_string()
                        })?;
                generated_translation_wall_duration_us_max =
                    generated_translation_wall_duration_us_max.max(duration);
            }
        }
    }
    if events == 0 {
        return Err("QEMU trace contains no translate_block events".into());
    }
    let unique_guest_pc_count = u64::try_from(events_by_guest_pc.len())
        .map_err(|_| "unique guest PC count exceeds u64".to_string())?;
    if generated_block_events > events {
        return Err(format!(
            "QEMU reported {generated_block_events} completed TBs but only {events} translation starts"
        ));
    }
    if duration_events > generated_block_events {
        return Err(format!(
            "QEMU reported {duration_events} translation durations for {generated_block_events} generated blocks"
        ));
    }
    let mut events_at_guest_pcs_seen_multiple_times = 0u64;
    let mut guest_pcs_seen_multiple_times = 0u64;
    let mut max_events_for_one_guest_pc = 0u64;
    for count in events_by_guest_pc.values().copied() {
        max_events_for_one_guest_pc = max_events_for_one_guest_pc.max(count);
        if count > 1 {
            guest_pcs_seen_multiple_times = guest_pcs_seen_multiple_times
                .checked_add(1)
                .ok_or_else(|| "repeated guest PC count exceeds u64".to_string())?;
            events_at_guest_pcs_seen_multiple_times = events_at_guest_pcs_seen_multiple_times
                .checked_add(count)
                .ok_or_else(|| "repeated-PC event count exceeds u64".to_string())?;
        }
    }
    Ok(TcgTranslationSummary {
        translation_block_events: events,
        unique_guest_pc_count,
        events_at_guest_pcs_seen_multiple_times,
        guest_pcs_seen_multiple_times,
        max_events_for_one_guest_pc,
        generated_block_events: (generated_block_events > 0).then_some(generated_block_events),
        generated_host_code_bytes_cumulative: (generated_block_events > 0)
            .then_some(generated_host_code_bytes_cumulative),
        translation_start_completion_event_count_delta: (generated_block_events > 0)
            .then_some(events - generated_block_events),
        generated_translation_wall_duration_us_cumulative: (duration_events > 0)
            .then_some(generated_translation_wall_duration_us_cumulative),
        generated_translation_wall_duration_us_max: (duration_events > 0)
            .then_some(generated_translation_wall_duration_us_max),
        interpretation: "guest-PC reuse only; a PC may identify multiple TBs under different translation state, so this is not a unique-TB or retranslations count",
    })
}
