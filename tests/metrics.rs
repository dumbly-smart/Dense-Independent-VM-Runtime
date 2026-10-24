use densevm::metrics::{
    MemoryTotals, parse_cgroup_memory, parse_smaps_for_path, parse_smaps_rollup,
};

#[test]
fn parses_resident_shared_and_private_kib() {
    let sample = "Rss:                 120 kB\nPss:                  70 kB\nShared_Clean:         40 kB\nShared_Dirty:         20 kB\nPrivate_Clean:        10 kB\nPrivate_Dirty:        50 kB\n";
    let got = parse_smaps_rollup(sample).unwrap();
    assert_eq!(
        got,
        MemoryTotals {
            rss_kib: 120,
            pss_kib: 70,
            shared_clean_kib: 40,
            shared_dirty_kib: 20,
            private_clean_kib: 10,
            private_dirty_kib: 50,
        }
    );
}

#[test]
fn rejects_missing_pss_instead_of_reporting_zero() {
    assert!(parse_smaps_rollup("Rss: 120 kB\n").is_err());
}

#[test]
fn sums_only_the_template_ram_file_mappings() {
    let sample = "1000-2000 rw-p 00000000 00:00 0 /tmp/memory.img\nRss: 40 kB\nPss: 20 kB\nShared_Clean: 30 kB\nShared_Dirty: 0 kB\nPrivate_Clean: 0 kB\nPrivate_Dirty: 10 kB\n2000-3000 rw-p 00000000 00:00 0 [heap]\nRss: 90 kB\nPss: 90 kB\nShared_Clean: 0 kB\nShared_Dirty: 0 kB\nPrivate_Clean: 0 kB\nPrivate_Dirty: 90 kB\n3000-4000 rw-p 00000000 00:00 0 /tmp/memory.img\nRss: 20 kB\nPss: 10 kB\nShared_Clean: 20 kB\nShared_Dirty: 0 kB\nPrivate_Clean: 0 kB\nPrivate_Dirty: 0 kB\n";
    assert_eq!(
        parse_smaps_for_path(sample, "/tmp/memory.img").unwrap(),
        MemoryTotals {
            rss_kib: 60,
            pss_kib: 30,
            shared_clean_kib: 50,
            shared_dirty_kib: 0,
            private_clean_kib: 0,
            private_dirty_kib: 10,
        }
    );
    assert!(parse_smaps_for_path(sample, "/tmp/absent.img").is_err());
}

#[test]
fn supports_paths_with_spaces_and_rejects_shared_or_readonly_clone_mappings() {
    let sample = "1000-2000 rw-p 00000000 00:00 0 /tmp/a b.img\nRss: 4 kB\nPss: 4 kB\nShared_Clean: 0 kB\nShared_Dirty: 0 kB\nPrivate_Clean: 0 kB\nPrivate_Dirty: 4 kB\n";
    assert_eq!(
        parse_smaps_for_path(sample, "/tmp/a b.img")
            .unwrap()
            .pss_kib,
        4
    );
    assert!(densevm::metrics::verify_private_ram_mapping(sample, "/tmp/a b.img").is_ok());
    assert!(
        densevm::metrics::verify_private_ram_mapping(
            &sample.replace("rw-p", "rw-s"),
            "/tmp/a b.img"
        )
        .is_err()
    );
    assert!(
        densevm::metrics::verify_private_ram_mapping(
            &sample.replace("rw-p", "r--p"),
            "/tmp/a b.img"
        )
        .is_err()
    );
}

#[test]
fn parses_cgroup_v2_current_and_kernel_memory_breakdown() {
    let memory = parse_cgroup_memory(
        "1048576\n",
        "anon 500000\nfile 300000\nkernel 248576\npagetables 32768\nsec_pagetables 8192\nkernel_stack 4096\nslab 8192\npgfault 41\npgmajfault 3\n",
    )
    .unwrap();
    assert_eq!(memory["current_bytes"], 1_048_576);
    assert!(memory.get("memory_stat_bytes").is_none());
    assert_eq!(memory["memory_stat"]["kernel"], 248_576);
    assert_eq!(memory["memory_stat"]["pagetables"], 32_768);
    assert_eq!(memory["memory_stat"]["sec_pagetables"], 8_192);
    assert_eq!(memory["memory_stat"]["pgfault"], 41);
    assert_eq!(memory["memory_stat"]["pgmajfault"], 3);
}

#[test]
fn rejects_incomplete_or_malformed_cgroup_memory_samples() {
    assert!(parse_cgroup_memory("not a number", "kernel 1\n").is_err());
    assert!(parse_cgroup_memory("1", "kernel invalid\n").is_err());
    assert!(parse_cgroup_memory("1", "").is_err());
}
