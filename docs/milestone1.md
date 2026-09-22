# Milestone 1 acceptance record

Reference run: `results/probe-2026-09-22-qemu-11.1.1.json` on QEMU 11.1.1,
Linux 7.2.6, `/dev/kvm` available, and KSM disabled. The probe uses the host
Linux kernel plus a static Rust initramfs. It is a genuine KVM guest mechanism
test; it is not a distribution image or a claim about login services.

| Criterion | Status | Evidence |
| --- | --- | --- |
| Linux QEMU/KVM guest boots | PASS | Guest kernel reaches `DENSEVM READY`; QMP reports KVM present. |
| RAM is file-backed | PASS | Template command uses `memory-backend-file`; `memory.img` is recorded. |
| Stable checkpoint state | PASS | Serial RAM and disk probes return expected values before freeze. |
| Template is frozen | PASS | QMP `stop`, paused status, migration, QEMU exit, file sync, and read-only RAM/base disk. |
| Two new QEMU VMs created | PASS | Separate clone QEMU processes and QMP/serial sockets. |
| Both clones resume | PASS | Both `migrate-incoming` operations complete and guests answer `ping`. |
| Unchanged RAM is shared | PASS | Each clone's RAM mapping reports about 22 MiB shared before divergence. |
| A's writes become private | PASS | A's private RAM mapping grows from about 2 MiB to about 35 MiB after a 32 MiB write. |
| B does not observe A's RAM write | PASS | B continues returning the original RAM byte. |
| qcow2 overlays are used | PASS | `qemu-img --backing-chain` verifies both overlays point to one qcow2 base. |
| Disk writes are isolated | PASS | A and B write distinct values and read back only their own values. |
| MAC addresses differ | PASS | NICs are hotplugged after restore; guest sysfs reports distinct MACs. |
| A can be killed independently | PASS | B answers `ping` after A's QEMU process is killed. |
| Memory is materially lower than baseline | PASS with scope | Clone process PSS is 104,900 KiB versus 408,515 KiB for two baseline VMs. The report separately records 160,612 KiB of template file cache and excludes kernel KVM metadata. |
| Results are reproducible and machine-readable | PASS | JSON includes commands, migration results, PSS/smaps data, file residency, disk allocation, hashes, and all isolation booleans. |

The same probe now accepts 2 through 32 clones; clones after A and B are
restored and included in the result's `extra` measurements. A and B remain the
adversarial isolation pair.

This closes the Milestone 1 mechanism gate. It does not close the broader
semantic-fidelity work: machine-id, SSH keys, DHCP identity, entropy behavior,
ordinary distribution services/login, guest reaction to vmgenid, timer behavior,
and kernel page-table accounting remain explicitly unverified in
`docs/semantic-differences.md`.
