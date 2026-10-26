# Semantic status after the first probe

Supported distribution update: Alpine 3.24.2 boots from ext4-backed qcow2 and
reaches BusyBox init, syslogd, crond, and an observed serial login prompt. The
default fixture stays minimal. An opt-in identity-validation fixture adds
OpenSSH and exercises BusyBox DHCP client identity in actual QEMU/KVM clones.
The tested Alpine kernel logged its VMGenID-triggered CRNG reseed after clone
restore. Existing userspace PRNG caches, arbitrary service identity caches, and
userspace random-state caches still require separate tests. Two synthetic
guests also have timerfd counter samples around pause/resume; they do not
isolate timer delivery during the pause or establish RTC alarm behavior.
See `examples/alpine/README.md`.

Update, 2026-10-24: the current synthetic guest receives a fresh host-kernel
entropy payload per clone, credits it with Linux `RNDADDENTROPY`, and forces
CRNG reseeding with `RNDRESEEDCRNG` before identity preparation and NIC hotplug.
The six-guest run on QEMU 11.1.2 / Linux 7.2.11 completed this protocol.
Machine IDs and locally administered unicast MACs are random per run and checked
for collisions within the clone batch. The kernel hostname is updated too.
Entropy payloads are excluded from result metadata; control sockets reside in
a mode-0700 output directory. This proves the explicit synthetic-kernel reseed
path, not reseeding of every existing userspace PRNG or application secret.
The six-guest range assigns unique static IPs; its socket multicast network
does not provide DHCP. A separate two-clone identity profile validates actual
OpenSSH host-key rotation and BusyBox DHCP option 61. Neither run proves how
arbitrary distro services or DHCP clients behave. Historical table rows below
are updated where recorded runs provide evidence; otherwise they describe the
original synthetic two-clone run.

KSM is opt-in and host-global. DenseVM changes only `run`, restores its own
change with readback verification, preserves concurrent changes to tuning
knobs, and refuses to interrupt `run=2` (host unmerging). SIGINT and SIGTERM
are handled as cancellation errors so VM/setting guards can clean up; SIGKILL
and host crashes cannot run cleanup. Restore errors are reported. Restoring
`run=0` stops scanning but does not undo unrelated merges that occurred while
the scanner ran. KSM can introduce cross-VM content/timing side channels and
CPU scanning cost; use only within an appropriate trust boundary.

`YES` below means the named behavior was observed in a recorded DenseVM probe.
`UNKNOWN` means the behavior remains untested for an ordinary Linux
distribution. Current synthetic probes use QEMU/KVM 11.1.2 on Linux 7.2.11.

| Property | Conventional VM | DenseVM probe | Evidence or limit |
| --- | --- | --- | --- |
| Separate QEMU process | YES | YES | Separate QEMU PIDs; killing target T1 left the client and targets T2–T5 responsive in the six-guest run. |
| Persistent clone lifecycle and crash state | YES | PARTIAL | Real QEMU runs verified `clone start/stop`, waited for QEMU PID exit before reporting stopped, and reconciled a SIGKILLed clone as failed. A persistent runtime clone from a relocated nested schema-v4 generation inherited its parent markers and isolated RAM/disk writes. Live/ambiguous endpoints and incomplete `starting` records remain unresolved. Details: [`docs/clone-lifecycle.md`](clone-lifecycle.md); evidence: [`persistent-lifecycle-run1`](../results/runtime-clone/2026-10-26/persistent-lifecycle-run1/README.md) and [`persistent-template-tree-run1`](../results/runtime-clone/2026-10-26/persistent-template-tree-run1/README.md). Other device/network configurations remain unknown. |
| Guest RAM writes isolated | YES | YES | The 21-VM Alpine range run dirtied 100% of clone A's 32 MiB test region; clone A observed its write and all siblings retained their own RAM values. This is 100% of the test region, not all guest RAM. Evidence: `results/runtime-clone/2026-10-26/alpine-twenty-target-range-run1/`. |
| Shared unchanged RAM | optional KSM | YES | In the 21-VM Alpine comparison (5.25 GiB configured guest RAM), aggregate clone process PSS was 997,844 KiB before and 1,030,264 KiB after clone A dirtied 32 MiB; ordinary baseline PSS was 4,222,425 KiB. The conservative PSS-plus-full-template-cache bound was 1,258,932 / 1,291,352 KiB. This single run used a tmpfs output directory; kernel/KVM metadata is excluded and disk allocation there is not persistent-storage usage. |
| Cgroup-v2 memory accounting | optional | YES in an isolated user scope | An eight-VM synthetic run measured `memory.current` at 1,956,212,736 bytes for ordinary QEMU processes and 975,228,928 bytes for template clones before divergence. The scope includes its charged file cache, tmpfs artifacts, and `memory.stat` kernel charges; allocations outside the cgroup or not charged to it remain UNKNOWN. Evidence: `results/runtime-clone/2026-10-26/eight-clone-cgroup-accounting-run1/`. |
| Template RAM mapping metrics | optional | YES | `densevm stats` aggregates template-file mapping PSS plus `Shared_*` and `Private_*` fields. Shared mapped totals count every clone mapping and are not a unique-physical-page count. Eight-clone sample: mapping PSS 184,925 KiB, shared mapped total 208,136 KiB, and private mapped total 158,168 KiB before divergence. |
| Disk mutations isolated | YES | YES | The 21-VM Alpine range run verified independent overlays, a full-disk write, and sibling disk reads after the write. |
| Shared disk backing | optional | YES | `qemu-img info --backing-chain` resolved both overlays to one qcow2 base. |
| Hierarchical template restore | possible | PARTIAL | Repeated synthetic-guest runs materialized three generated levels; each parent created two descendants with shared resident RAM and isolated A's writes from B. Schema v3 preserves actual parent artifact paths, Rust validation checks hashes, and the catalog resolves lineage within a store. Persistent clone lifecycle, relocation, recovery, and other device/network configurations remain unverified. Evidence: [`docs/template-tree.md`](template-tree.md) and `results/template-tree/2026-10-26/`. |
| Independent vCPU/register state | YES | PARTIAL | Three two-clone KVM runs captured QEMU's `info registers` output while clones ran different seeded workloads with deliberately assigned user GS bases. QEMU reported both GS bases exactly; GPR snapshots differed between clones. CR0/CR2/CR3/CR4 and EFER matched in the first pair. Two repeated runs captured `info lapic` for each clone; each pair of initial dumps was identical and reported TPR 0x10. Other MSRs and multi-vCPU state remain UNKNOWN. Evidence: `results/2026-10-24/vcpu-register-state-2.json` and the two `vcpu-register-state-lapic-2*.json` records; method and limits: `docs/vcpu-register-state.md`. |
| Independent LAPIC TPR mutations | YES | PARTIAL | Two repeated guest TPR divergence probes set A to 0x11 while B remained 0x10, then set B to 0x12 while A reported 0x10. Both original TPRs were restored and QMP verified. This tests only the TPR subpriority field; other APIC registers and pending interrupts remain UNKNOWN. Evidence: `results/2026-10-24/lapic-tpr-divergence-2.json` and `lapic-tpr-divergence-2-run2.json`. |
| Independent virtual device state | YES | PARTIAL | Separate QMP, serial, block and hotplugged NIC observed. Guest timerfd, aggregate `/proc/interrupts` `LOC`, RTC epoch counters, and QEMU LAPIC snapshots are captured. TPR writes were independently observed; other APIC mutation, per-vCPU interrupt state, exact interrupt timing, and RTC alarm delivery remain unverified. |
| Unique MAC visible inside guest | YES | YES | The 21-VM Alpine range run verified distinct MACs and guest-visible identity values across all 21 clones. Guest sysfs returned distinct MACs after NIC hotplug; launch-time MAC changes alone failed because migration restored the template MAC. |
| Unique vmgenid in QEMU | optional | YES | QMP returned distinct GUIDs for template and clones. |
| Guest VMGenID response | expected | YES for tested Linux kernel | Each Alpine clone logged one `crng reseeded due to virtual machine fork` event after restore, compared with the template baseline and before explicit host entropy injection. This verifies the tested kernel's VMGenID response, not userspace RNG consumers. |
| Independent machine-id, SSH keys, hostname | YES when configured | YES in identity-validation profile | The prepared Alpine profile generated inherited OpenSSH keys before template freeze. Two clones rotated them after guest RNG reseeding; all three host-key fingerprints per clone were present and distinct. Other SSH implementations remain UNKNOWN. Evidence: `results/2026-10-24/identity-validation-2.json`. |
| Independent entropy/RNG state | expected | PARTIAL | Synthetic guest receives fresh kernel entropy and forces CRNG reseed before identity/network setup. Existing userspace PRNG state and application secrets remain untested. |
| Independent DHCP identity/IP | expected | YES for tested client; otherwise UNKNOWN | Two identity-validation clones sent distinct machine-id-derived DHCP option 61 values in BusyBox `udhcpc` packets, confirmed by guest `AF_PACKET` capture, and obtained leases. This does not prove how another distribution's DHCP client behaves. Evidence: `results/2026-10-24/identity-validation-2.json`. |
| Separate guest process state | YES | YES | The 21-VM Alpine range run started `/bin/sleep` in one clone; every other live clone reported zero matching processes. Synthetic initramfs runs mark this check NOT REQUESTED. Evidence: `results/runtime-clone/2026-10-26/alpine-twenty-target-range-run1/`. |
| Guest reboot isolation | YES | YES | Rebooting target A exits its QEMU process under `-no-reboot`; B remains responsive. |
| Guest kernel crash isolation | YES | YES | A three-clone synthetic run triggered a SysRq kernel panic in one guest, observed the panic on serial and QMP/QEMU survival, and verified the other clones remained responsive. Ordinary distribution crash behavior remains untested. |
| Guest OOM isolation | YES | YES for tested Alpine profile | A 21-VM run triggered guest OOM in one target: two pressure workers were OOM-killed, the guest OOM counter increased by three, and all guests remained responsive. The guest test temporarily protects its coordinator and raises worker OOM scores; this does not test host OOM isolation. Evidence: `results/runtime-clone/2026-10-26/alpine-twenty-target-range-run1/`. |
| Shared virtual Ethernet segment | optional | YES | Twenty-one separate virtio NICs use a QEMU socket multicast backend bound to host loopback; the attacker reached all 20 targets before target `10.77.0.10` was killed and reached surviving target `10.77.0.11` afterward. This is a shared range network, not host-user isolation. |
| Guest network throughput | workload-dependent | PARTIAL | A 64 KiB UDP request burst delivered 64/64 packets with 0% observed loss in 6,237 µs in the 21-VM range. This is one short localhost multicast sample, not host NIC line rate. |
| Guest disk write+sync latency | workload-dependent | PARTIAL | A two-VM Alpine run recorded 16 × 4 KiB ext4 writes with `fdatasync`; p95 values varied from 735 to 3,102 µs across four guests. The small sample is not a performance conclusion. |
| Independent wall clock/timers | YES | PARTIAL | Two repeated two-clone runs around an approximately one-second QMP pause measured realtime deltas of 7–8 ms and `/proc/uptime` deltas of 0–10 ms. Each guest's RTC `since_epoch` advanced 1 second. The aggregate `LOC` counter advanced 18–21 and a 10 ms guest `CLOCK_MONOTONIC` timerfd accumulated 2 expirations between pre-pause and post-resume reads. Four repeated RTC alarm probes verified `RTC_AF` delivery in both running clones (one event each, flags 160), but all eight paused-trial alarms missed after a seven-second QMP pause and 20-second post-resume wait. Diagnostics show the RTC advanced 22 seconds past its alarm while `/proc/driver/rtc` reported `alarm_IRQ: yes` and `alrm_pending: no`. This narrows the observation to the guest RTC/interrupt path without identifying the failing QEMU callback or IRQ step. Results are specific to QEMU 11.1.2 / Linux 7.2.11 and the synthetic guest. Post-resume timerfd/counter samples cannot locate event delivery during the pause. Per-vCPU interrupt state remains UNKNOWN. Evidence: `results/2026-10-24/parking-clock-devices-2-run1.json`, `parking-clock-devices-2-run2.json`, and the four `rtc-alarm-pause-2*.json` records. |
| Explicit idle parking | optional | PARTIAL | Six paused QEMU processes accumulated zero CPU ticks over one second and resumed in 6 ms total. The guest clock sample shows guest time nearly stops during this interval; parking changes time semantics. |
| Kernel and KVM memory overhead included in result | applicable | PARTIAL | Two cgroup-v2 runs recorded `memory.current` and `memory.stat` at baseline and clone snapshots. `kernel` charges were 3.4–4.0 MB baseline and 7.8 MB for clones; `sec_pagetables` (which includes KVM MMU allocations on the tested kernel) varied from 123–745 KiB baseline and was 400 KiB for clones. Cgroup charges include file cache and are not a complete host-memory inventory. Evidence: `results/2026-10-24/cgroup-memory-summary.json`; method and limits: `docs/cgroup-memory-accounting.md`. |

The host's KSM `run` value was `0` in the reference experiment. This result is
template file COW, not KSM. Each guest still executes its own instructions on
KVM; no CPU work is deduplicated. The source RAM file and disk base are made
read-only after freezing and their backing relationships and hashes are
checked by the probe, but a privileged host actor can still change them.

Probe results include host-global KSM counters and `ksmd` CPU ticks before and
after the run when procfs/sysfs expose them. These deltas are not attributable
solely to DenseVM if other host processes use KSM concurrently; CPU values are
raw scheduler ticks and should be converted using the host's clock-tick rate.
`experiments/ksm/run.sh` runs matched COW-only and KSM probes and summarizes
clone PSS, RAM-mapping PSS, global KSM counters, and `ksmd` ticks. The current
host fails its read-only preflight because `/sys/kernel/mm/ksm/run` is not
writable; no positive KSM merge measurement is available yet. See
`docs/ksm-experiment.md` for the exact procedure.
