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
| Guest RAM writes isolated | YES | YES | Six-guest run checked private writes across all clones; each dirtied a 32 MiB test region while siblings retained their own values. |
| Shared unchanged RAM | optional KSM | YES | Eight-clone Alpine run: RAM mapping PSS was 71,475 KiB before and 333,852 KiB after each clone dirtied its 32 MiB test region. Mapping PSS is not total host RAM; the test region is one eighth of configured guest RAM. |
| Disk mutations isolated | YES | YES | The six-guest run checked separate overlay writes and filled a bounded test-file region without sibling changes. |
| Shared disk backing | optional | YES | `qemu-img info --backing-chain` resolved both overlays to one qcow2 base. |
| Independent vCPU/register state | YES | UNKNOWN | Separate KVM/QEMU instances restore VMState, but no register-level adversarial test yet. |
| Independent virtual device state | YES | PARTIAL | Separate QMP, serial, block and hotplugged NIC observed. Guest timerfd, aggregate `/proc/interrupts` `LOC`, and RTC epoch counters are sampled around pause/resume; exact interrupt timing/state and RTC alarm delivery remain untested. |
| Unique MAC visible inside guest | YES | YES | Guest sysfs returned distinct MACs after NIC hotplug. Launch-time MAC changes alone failed because migration restored the template MAC. |
| Unique vmgenid in QEMU | optional | YES | QMP returned distinct GUIDs for template and clones. |
| Guest VMGenID response | expected | YES for tested Linux kernel | Each Alpine clone logged one `crng reseeded due to virtual machine fork` event after restore, compared with the template baseline and before explicit host entropy injection. This verifies the tested kernel's VMGenID response, not userspace RNG consumers. |
| Independent machine-id, SSH keys, hostname | YES when configured | YES in identity-validation profile | The prepared Alpine profile generated inherited OpenSSH keys before template freeze. Two clones rotated them after guest RNG reseeding; all three host-key fingerprints per clone were present and distinct. Other SSH implementations remain UNKNOWN. Evidence: `results/2026-10-24/identity-validation-2.json`. |
| Independent entropy/RNG state | expected | PARTIAL | Synthetic guest receives fresh kernel entropy and forces CRNG reseed before identity/network setup. Existing userspace PRNG state and application secrets remain untested. |
| Independent DHCP identity/IP | expected | YES for tested client; otherwise UNKNOWN | Two identity-validation clones sent distinct machine-id-derived DHCP option 61 values in BusyBox `udhcpc` packets, confirmed by guest `AF_PACKET` capture, and obtained leases. This does not prove how another distribution's DHCP client behaves. Evidence: `results/2026-10-24/identity-validation-2.json`. |
| Separate guest process state | YES | YES | Range starts a process only in T1; every sibling reports zero matching processes. |
| Guest reboot isolation | YES | YES | Rebooting target A exits its QEMU process under `-no-reboot`; B remains responsive. |
| Guest kernel crash isolation | YES | YES | A three-clone synthetic run triggered a SysRq kernel panic in one guest, observed the panic on serial and QMP/QEMU survival, and verified the other clones remained responsive. Ordinary distribution crash behavior remains untested. |
| Shared virtual Ethernet segment | optional | YES | Six separate virtio NICs use a QEMU socket multicast backend bound to host loopback; targets respond before and after T1 is killed. This is a shared range network, not host-user isolation. |
| Guest network throughput | workload-dependent | PARTIAL | A 64 KiB UDP request burst completed with 0% observed loss in 1,241 µs. This is a short guest-to-guest sample on a local multicast backend, not host NIC line rate. |
| Guest disk write+sync latency | workload-dependent | PARTIAL | A two-VM Alpine run recorded 16 × 4 KiB ext4 writes with `fdatasync`; p95 values varied from 735 to 3,102 µs across four guests. The small sample is not a performance conclusion. |
| Independent wall clock/timers | YES | PARTIAL | Two repeated two-clone runs around an approximately one-second QMP pause measured realtime deltas of 7–8 ms and `/proc/uptime` deltas of 0–10 ms. Each guest's RTC `since_epoch` advanced 1 second. The aggregate `LOC` counter advanced 18–21 and a 10 ms guest `CLOCK_MONOTONIC` timerfd accumulated 2 expirations between pre-pause and post-resume reads. Post-resume samples include guest/serial activity and cannot locate when events were delivered during the pause. RTC alarm behavior and per-vCPU interrupt state remain UNKNOWN. Raw records: `results/2026-10-24/parking-clock-devices-2-run1.json` and `parking-clock-devices-2-run2.json`. |
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
