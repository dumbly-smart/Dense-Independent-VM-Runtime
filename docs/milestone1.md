# Milestone 1 acceptance record

## Supported distribution profile, 2026-10-24

`examples/alpine/prepare.sh` now builds a hash-verified Alpine 3.24.2 ext4
root image. A two-clone run on QEMU 11.1.2 / Linux 7.2.11 verified a checkpoint
with BusyBox init, syslogd, crond, getty, and an observed serial login prompt.
Both independent QEMU clones resumed, reseeded their kernel RNGs, received
unique machine IDs/hostnames/MACs before NIC hotplug, and passed RAM and
filesystem-file isolation plus process-kill and explicit parking checks. A
six-guest range run then verified five-target UDP reachability, per-guest
process separation, reboot and QEMU-kill isolation, and continued network
access to surviving targets. Eight clones passed the same isolation checks.
The prepared image has no SSH server or DHCP client, so it duplicates neither
SSH host keys nor a DHCP client identity. Arbitrary distribution services and
application secrets remain outside this demonstrated profile.

The original synthetic reference record is retained below. The eight-clone
0/5/10/25/50/100% buffer-divergence sweep is saved under
`results/2026-10-22/divergence/`; the percentage is of each 32 MiB test buffer,
not of all guest RAM. Host kernel/KVM memory overhead remains unmeasured.

Current raw results are in `results/2026-10-24/`. The combined six-guest run
measured baseline process PSS of 1,230,793 KiB and clone PSS of 276,596 KiB
before the test-region writes, rising to 473,537 KiB afterward. The eight-clone
zero- and full-test-region runs measured clone PSS rising from 348,210 to
349,429 KiB at 0% and from 352,801 to 615,175 KiB at 100% of the 32 MiB
region. Each run reports process PSS, not complete physical memory; KVM
metadata and host page tables are not included.

## Historical synthetic reference

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
restored and included in memory, disk, identity, and device measurements. Each
additional clone writes a unique RAM value and disk value; after all writes,
every clone reads its own values while A and B retain their original values.
The result records this as `extra_clone_isolation`.

The probe accepts `--divergence-mib 0..32` and `--divergence-percent 0..100`
to control how much of A's test region is dirtied. The percentage mode maps to
whole guest pages and records the selected page count and rounded percentage
in `result.json`.

This historical synthetic run assigned distinct machine-id, hostname, and
seed-token values in A and B. Its seed token was deterministic and was not
cryptographic entropy. The current guest path uses fresh host-kernel entropy
and forces guest CRNG reseeding before identity/network preparation. The
supported Alpine profile has no SSH server or DHCP client; SSH key rotation,
DHCP behavior, guest reaction to vmgenid, timer behavior, and kernel page-table
accounting remain unverified in `docs/semantic-differences.md`.
