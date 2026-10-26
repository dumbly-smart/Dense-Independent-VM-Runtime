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
    let count = input
        .lines()
        .filter(|line| line.starts_with("translate_block "))
        .count();
    if count == 0 {
        return Err("QEMU trace contains no translate_block events".into());
    }
    u64::try_from(count).map_err(|_| "translate_block event count exceeds u64".into())
}
