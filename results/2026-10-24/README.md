# Six-guest range run

## Host capability snapshot

[`host-capabilities.json`](host-capabilities.json) is a read-only `densevm doctor` report collected 2026-10-24. It records QEMU/qemu-img 11.1.2, Linux 7.2.11-arch1-1, 4096-byte pages, accessible `/dev/kvm`, the required `x-ignore-shared` migration capability, and KSM counters with `run=0`. The report does not change host configuration.

Run on 2026-10-24 with QEMU 11.1.2 and Linux 7.2.11, using the locally prepared Alpine 3.24.2 ext4 fixture. Reproduce with:

```sh
cargo run -- probe --guest /path/to/prepared-alpine --range --count 6 --divergence-all --park-idle --out /tmp/densevm-range
```

The full machine-readable measurement is in [`security-range-6.json`](security-range-6.json). The experiment launched six conventional baseline guests and six templated clones; each guest had 256 MiB configured RAM. The range used one UDP client and five targets on a loopback-bound QEMU multicast socket network.

| Measurement | Result |
| --- | ---: |
| Baseline: six QEMU processes, summed PSS | 1,214,875 KiB |
| Baseline time until six guests ready | 5,299 ms |
| Templated clones: summed PSS before test divergence | 272,865 KiB |
| Templated clones: summed PSS after each clone dirtied its 32 MiB test region | 469,747 KiB |
| Clone RAM mapping PSS, before → after divergence | 59,435 → 256,193 KiB |
| Shared base disk allocated | 12,128,256 bytes |
| Six overlay files allocated | 70,082,560 bytes total |

The initial clone process PSS was about 22.5% of the six-process conventional baseline PSS for this fixture and measurement point. After the private RAM writes it was about 38.7%. These are process-PSS comparisons, not complete host physical-memory totals: QEMU PSS omits KVM metadata, host page tables, and other kernel allocations. The 32 MiB divergence region is not the full 256 MiB guest RAM.

The run verified RAM and disk isolation, whole-test-disk isolation, unique guest/MAC/vmgenid identities, isolated process state, per-clone parking and resume, guest reboot of target A, survival of its sibling, and client connectivity to all five targets both before and after killing target T1's QEMU process. `KSM run` remained 0; this run measures template COW without KSM.

[`security-range-6-network.json`](security-range-6-network.json) is a follow-up with the same six-guest topology and a bounded UDP request burst. It sent 64 one-kilobyte datagrams from the client to T1, received all 64 identity-verified replies, and measured 1,241 microseconds end to end (about 51,571 requests/s and 422.5 Mbps of request payload). The socket multicast backend is local to this host and the sample is short, unpaced guest-to-guest traffic. Treat it as a diagnostic of this path, not as host NIC line rate or a general network benchmark.

## Active CPU sample

[`active-cpu-2.json`](active-cpu-2.json) records a separate matched two-VM synthetic-guest run. Each guest executes `cpu-burn 1500`; QEMU process CPU ticks are sampled during the same one-second window.

| Mode | Idle ticks per guest | Active CPU (% of one host core) | Summed process PSS |
| --- | ---: | ---: | ---: |
| Conventional baseline | 2, 3 | 101%, 102% | 421,471 KiB |
| Templated clones | 2, 2 | 102%, 102% | 109,078 KiB |

This indicates both modes consume about one host core per busy guest. DenseVM does not deduplicate guest instructions. The PSS comparison is specific to this synthetic guest and omits kernel memory overhead.

## Initial disk sync latency sample

[`disk-io-2.json`](disk-io-2.json) records a matched two-VM Alpine ext4 probe. Each guest writes 16 × 4 KiB to its own test file and calls `fdatasync` after each write. The values are per guest:

| Mode | Mean write+sync latency | p50 | p95 |
| --- | ---: | ---: | ---: |
| Conventional baseline 0 | 366 µs | 192 µs | 2,255 µs |
| Conventional baseline 1 | 444 µs | 217 µs | 3,102 µs |
| Templated clone 0 | 154 µs | 102 µs | 735 µs |
| Templated clone 1 | 207 µs | 146 µs | 1,089 µs |

The p95 spread and tiny 16-write sample make these initial observations only; they do not establish a disk performance advantage. The test measures guest-visible file write+sync latency over qcow2, including host cache and scheduling effects.

## Combined six-guest run

[`security-range-6-combined.json`](security-range-6-combined.json) runs the range and all three workload measurements together. The six-VM baseline summed 1,230,793 KiB PSS and became ready in 4,601 ms. The six templated clones summed 276,596 KiB PSS before 32 MiB-per-clone test divergence and 473,537 KiB after. Their RAM-file mapping PSS rose from 62,169 to 259,101 KiB. This process-PSS comparison excludes KVM metadata and host page tables. The run also recorded one-second active CPU samples near 99–101% of one core per VM, 16-write ext4 sync samples per guest, and a 64 KiB UDP request burst with 0% observed loss in 1,330 µs (394.2 Mbps request payload). All range connectivity and isolation checks passed, including sibling survival after T1 reboot and QEMU termination. `KSM run` remained 0.

## Eight-clone range, 0% test-region divergence

[`security-range-8-zero.json`](security-range-8-zero.json) records eight 256 MiB Alpine guests. The eight-VM baseline summed 1,623,396 KiB process PSS; templated clones summed 348,210 KiB before test writes and 349,429 KiB after. The explicit 32 MiB divergence region remained unchanged in all eight clones (`ram_divergence_bytes=0`); other normal guest activity still caused about 1.2 MiB additional RAM-mapping PSS. The clones all passed RAM/disk/identity/process/reboot/parking and network reachability checks. As in the other runs, process PSS excludes KVM metadata and host page tables.

[`security-range-8-full-divergence.json`](security-range-8-full-divergence.json) is a separate eight-clone run with all 8,192 pages (32 MiB) of the test region dirtied independently by every clone. Summed clone process PSS rose from 352,801 to 615,175 KiB; the RAM-file mapping PSS rose from 71,475 to 333,852 KiB, an increase of 262,377 KiB. This closely tracks eight private 32 MiB regions. All-clone divergence and sibling isolation checks passed. The test region is one eighth of configured guest RAM; this is not a full-guest divergence experiment. These 0% and 100% endpoints were separate runs and should not be treated as a controlled curve without the intermediate sweep.

[`parking-6.json`](parking-6.json) records an explicit six-clone park/resume sample. All six QMP stop states and resume states were verified, the six QEMU processes accumulated zero scheduler ticks over the one-second parked sample, QMP resume commands completed in 6 ms total, and every guest responded after resume. Guest RTC and timer catch-up behavior is still unknown; this experiment makes no claim of continuous-time semantic equivalence.

[`parking-clock-2.json`](parking-clock-2.json) records guest wall-clock and `/proc/uptime` samples immediately before and after a one-second parked interval. Across two synthetic guests, wall time advanced 8–9 ms and uptime advanced 10 ms while host time advanced about one second. This is a measured pause-time difference for this QEMU/kernel configuration. It does not test periodic timer delivery or hardware RTC behavior.

[`ssh-identity-2.json`](ssh-identity-2.json) records the updated identity protocol on the synthetic guest. Machine IDs and hostnames differed across baseline and clones; all reported `SSH-IDENTITY ABSENT`, as expected for an image without OpenSSH. The probe now rotates inherited OpenSSH host keys after RNG reseeding and checks their public-key fingerprints for duplicates when such keys exist. This run does not demonstrate regeneration because the fixture contains no SSH keys.

[`ssh-dhcp-identity-2.json`](ssh-dhcp-identity-2.json) records the guest protocol's explicit `DHCP-IDENTITY UNKNOWN` response. The test profile uses unique static IPs and does not run a DHCP client; MAC uniqueness is not treated as proof of DHCP identity uniqueness.

## Guest OOM isolation

[`guest-oom-2.json`](guest-oom-2.json) records a two-clone synthetic KVM probe on QEMU 11.1.2 / Linux 7.2.11. Each guest is configured with 256 MiB. Inside clone A, eight workers each faulted in a 64 MiB anonymous mapping (`MAP_NORESERVE`); the guest kernel's `/proc/vmstat` `oom_kill` count increased by 6 and six worker processes were killed. Both clone guests remained responsive after pressure. The probe only reports success when a worker was killed and the guest counter increased. This exercises guest OOM isolation, not host OOM handling or the behavior of a production distribution's memory manager.

## Final two-clone verification

[`final-two-clone-2.json`](final-two-clone-2.json) is the successful disk-backed rerun after adding an explicit guest identity-preparation gate. It verifies two-clone RAM and disk isolation, unique MAC and vmgenid values, guest OOM isolation, and survival of clone B after clone A is killed. The run used a synthetic Alpine initramfs, 256 MiB per guest, and 0% requested divergence in the 32 MiB test region. Process PSS excludes KVM metadata and host page tables. An earlier live run writing its artifacts to tmpfs `/tmp` encountered a guest disk-write `EIO` while that filesystem was 81% used; the overlay passed `qemu-img check`, and the cause was not isolated. The rerun wrote under disk-backed `/home` and passed. Treat the first failure as unresolved rather than as a proven tmpfs-capacity bug.

The final post-change Cargo integration reruns are saved as [`final-two-clone-cargo-2.json`](final-two-clone-cargo-2.json) and [`final-kernel-crash-cargo-3.json`](final-kernel-crash-cargo-3.json). Both used a disk-backed temporary directory with a short path (Unix domain sockets impose a path-length limit). The two-clone test passed RAM/disk isolation and sibling survival; the three-clone test observed the guest kernel crash, verified the crashed VM's QEMU/QMP process remained independently alive for inspection, and confirmed the sibling guests remained responsive. The opt-in owned-QEMU interruption cleanup test also passed.

## Alpine identity-validation profile

[`identity-validation-2.json`](identity-validation-2.json) and
[`identity-validation-guest-metadata.json`](identity-validation-guest-metadata.json)
record the opt-in Alpine profile with two restored clones. Both clones generated
three unique SSH host keys after entropy preparation, sent distinct
machine-id-derived DHCP option 61 identifiers observed in guest packet capture,
and obtained DHCP leases. Each kernel logged one VMGenID-triggered CRNG reseed
event relative to the template baseline, before the explicit host entropy
injection. RAM/disk isolation and sibling survival also passed. The independent
guest OOM workload was explicitly skipped for this identity-focused run and is
reported as skipped in the JSON; see [`guest-oom-2.json`](guest-oom-2.json) for
the separate OOM result. Other distributions' DHCP clients and userspace
randomness behavior remain unmeasured.

## KSM comparison preflight

[`ksm-comparison-blocked/preflight.json`](ksm-comparison-blocked/preflight.json)
is the paired harness's read-only preflight record. It found `/sys/kernel/mm/ksm/run`
at `0` and not writable by this user, so it stopped before launching either
probe. No KSM setting changed and no positive KSM merge data is claimed. The
runner and comparison format are documented in [`docs/ksm-experiment.md`](../../docs/ksm-experiment.md).

## Timerfd samples around parking

[`parking-timerfd-2-run1.json`](parking-timerfd-2-run1.json) and
[`parking-timerfd-2-run2.json`](parking-timerfd-2-run2.json) are repeated
two-clone synthetic probes with a one-second QMP pause. Across the four guest
samples, realtime advanced 7–15 ms and `/proc/uptime` advanced 0–20 ms. A
10 ms-period guest `CLOCK_MONOTONIC` timerfd accumulated 1–3 expirations
between the pre-pause and post-resume reads. Each result also verifies zero
QEMU CPU ticks while parked, successful resume, and RAM/disk isolation. The
post-resume counts include guest and serial-command execution after resume;
they do not establish whether expirations occurred during the pause or measure
APIC interrupt counts. Hardware RTC behavior remains unmeasured. The OOM
workload was explicitly skipped in these timing-focused runs.

## Local timer interrupt and RTC samples

[`parking-clock-devices-2-run1.json`](parking-clock-devices-2-run1.json) and
[`parking-clock-devices-2-run2.json`](parking-clock-devices-2-run2.json) repeat
the two-clone pause test with guest `/proc/interrupts` `LOC` and RTC
`since_epoch` reads. The two guests' realtime advanced 7–8 ms and uptime
advanced 0–10 ms around the approximately one-second host pause. Each guest's
RTC epoch advanced one second; local timer interrupt totals increased by
18–21, and 10 ms-period timerfd counters increased by two. All counter reads
were taken after resume, so these samples do not show which interrupts were
delivered while paused. They establish the behavior of these counters for this
QEMU/kernel configuration, not independent RTC alarm delivery or exact APIC
timing.

## Timerfd samples around parking

[`parking-timerfd-2-run1.json`](parking-timerfd-2-run1.json) and
[`parking-timerfd-2-run2.json`](parking-timerfd-2-run2.json) are repeated
two-clone synthetic probes with a one-second QMP pause. Across the four guest
samples, realtime advanced 7–15 ms and `/proc/uptime` advanced 0–20 ms. A
10 ms-period guest `CLOCK_MONOTONIC` timerfd accumulated 1–3 expirations
between the pre-pause and post-resume reads. Each result also verifies zero
QEMU CPU ticks while parked, successful resume, and RAM/disk isolation. The
post-resume counts include guest and serial-command execution after resume;
they do not establish whether expirations occurred during the pause or measure
APIC interrupt counts. Hardware RTC behavior remains unmeasured. The OOM
workload was explicitly skipped in these timing-focused runs.
