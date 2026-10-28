# DenseVM

DenseVM is a systems research prototype for QEMU/KVM VM templating. Its first
experiment freezes one Linux guest, resumes two independent QEMU processes
from that state, maps the same RAM file privately, and uses distinct qcow2
disk overlays. It measures isolation and host memory with QMP, `/proc/smaps`,
and `fincore`.

The default guest is a small static Rust `/init` running the host's Linux
kernel. A supported [Alpine ext4 guest](examples/alpine/README.md) also boots
normal distribution userspace and a serial login prompt. See [design](docs/design.md),
[measured findings](docs/first-probe.md), and
[semantic differences](docs/semantic-differences.md).

New to the project? Start with the from-scratch [explanation](explanation.md),
then read the [architecture](architecture.md) and [current progress](progress.md).
Use [reports.md](reports.md) to find raw experiment evidence and
[agents.md](agents.md) for contribution workflow.

## Reproduce the two-clone experiment

On the inspected Arch Linux host (QEMU 11.1.2, kernel 7.2.11), install or make
available `qemu-system-x86_64`, `qemu-img`, `rustc`, `cargo`, `cpio`,
`modinfo`, `fincore`, and `sha256sum`. The host kernel and its matching
`virtio_net`, `net_failover`, and `failover` modules must be present, as must
`/dev/kvm` and `/boot/vmlinuz-linux`. The probe uses 256 MiB guest RAM,
roughly 160 MiB of allocated template RAM file storage, and a 64 MiB virtual
disk. KSM stays in its existing state; the reference run had it disabled.

```sh
cargo test
cargo run -- doctor
cargo run -- stats /tmp/densevm-my-run/result.json
cargo test --test probe -- --ignored --nocapture
# Or run outside the test harness, specifying a NEW directory:
cargo run -- probe --out /tmp/densevm-my-run
# Limit the intentional RAM divergence used by the isolation probe:
cargo run -- probe --out /tmp/densevm-my-8m-run --divergence-mib 8
# Measure explicit QMP stop/resume of idle clones:
cargo run -- probe --out /tmp/densevm-park-run --park-idle
# Compare the same probe with KSM temporarily enabled; prior KSM settings are restored:
cargo run -- probe --out /tmp/densevm-my-ksm-run --ksm
# Run a paired COW-only vs KSM experiment with preflight and summary:
experiments/ksm/run.sh --out /tmp/densevm-ksm-pair --count 2
# Run a conventional baseline at a requested count:
cargo run -- benchmark baseline --count 8 --out /tmp/densevm-baseline-8
# Run the conventional baseline under QEMU TCG rather than KVM:
cargo run -- benchmark baseline --count 2 --accel tcg --out /tmp/densevm-tcg-2
# Count TCG translation events (trace output perturbs timings; limit two VMs):
cargo run -- benchmark baseline --count 1 --accel tcg --trace-translations --out /tmp/densevm-tcg-traced-1
# Inspect saved disk and live RAM metadata:
cargo run -- inspect /tmp/densevm-my-run/clone-a.qcow2 disk
cargo run -- memory-map QEMU_PID /tmp/densevm-my-run/memory.img
cargo run -- ksm status
# Inspect lineage metadata without hashing large RAM files:
cargo run -- template tree /path/to/templates
# Verify every generation and its manifest-referenced parent/child artifacts:
cargo run -- validate template-tree /path/to/templates
# Print a conservative, read-only template garbage-collection plan:
cargo run -- template gc-plan /path/to/templates --state-dir /var/lib/densevm --keep sha256:GENERATION_ID
```

The command prints the path to `result.json`. It leaves the RAM, migration,
disk, and QEMU log artifacts in that new directory for inspection. QEMU
processes are stopped when the experiment exits, including on error. The
output directory is never reused or deleted automatically.

The probe first boots the requested number of ordinary QEMU/KVM guests from
equivalent independent qcow2 images and measures them after each reports ready.
It then boots the template, freezes it with QMP, migrates nonshared state to
`machine-state`, and starts the same number of clones. Clone NICs are
hotplugged after restore so migrated NIC
state cannot duplicate the template MAC. The guest commands verify RAM and
disk isolation, guest-visible MACs, and continued operation of B after A's
QEMU process is killed. Results include whole-process RSS/PSS/private/shared
metrics, the exact RAM-file mapping, disk allocation, and template file cache
residency.

This prototype currently targets mechanisms verified on QEMU 11.1.2.
`x-ignore-shared` is experimental. Do not treat this probe as evidence that
arbitrary distributions, devices, or QEMU versions can be cloned safely.

The 15-item acceptance record is in [docs/milestone1.md](docs/milestone1.md).

The baseline command supports counts from 1 through 32 and writes
`baseline.json`. The template restore probe supports 2 through 32 clones;
the first two receive detailed RAM, disk, reboot, and process-kill isolation
checks. With at least three clones, the first additional clone also receives a
guest-kernel crash-isolation check; all additional clones contribute identity,
memory-map, and liveness checks.
The probe defaults to one vCPU per guest; `--vcpus 1..4` applies the selected
count to baseline guests, the template, and all restored clones. Two-vCPU
restore samples are saved under `results/vcpu-multi2/` and
`results/vcpu-multi2-run2/`. QMP register snapshots in those samples cover
CPU#0 only. The optional register probe now requests all CPU blocks with
`info registers -a`; its post-change KVM result remains unverified because the
current host fails before guest readiness.
`--divergence-mib` controls how much of A's 32 MiB test region is dirtied;
`--divergence-percent` supports exact page-count sweeps from 0% to 100%.

## Current research status

| Status | Evidence |
| --- | --- |
| Demonstrated | Separate QEMU/KVM guests resume from one frozen state, share unchanged RAM pages, COW private RAM writes, and use private qcow2 overlays. Live 2/4/8/16 synthetic-clone scale measurements and a six-point 8-clone divergence sweep are saved under `results/`. |
| Isolation tested | Guest RAM and disk writes, unique MAC/machine-id/hostname/vmgenid, guest OOM, guest kernel panic, guest reboot, QEMU process termination, and a six-guest Alpine range. A four-clone Alpine run starts a process in one guest and verifies zero matching processes in all three siblings. |
| Partial | The opt-in Alpine profile verifies unique OpenSSH host-key fingerprints, DHCP option 61 identity, and the tested kernel's VMGenID reaction. Historical KVM snapshots captured CPU#0 only; the optional probe now requests all vCPU register blocks, but the post-change path has not yet run under KVM because current host guest entry fails. Two initial LAPIC snapshot runs matched at TPR 0x10. Two later guest-write runs changed A's TPR from 0x10 to 0x11 while B stayed 0x10, then changed B to 0x12 while A reported 0x10; both original values were restored. This demonstrates TPR subpriority isolation only. Other APIC registers, pending interrupts, and other MSRs remain unverified. CR0/CR2/CR3/CR4 and EFER matched in the first sample. Two parked-guest samples record wall clock, uptime, timerfd expirations, aggregate local-timer interrupts, and RTC epoch but cannot locate timer events within the pause. In four repeated RTC-alarm probes, each running clone fired once; all eight alarms armed before a seven-second QMP pause missed the 20-second post-resume wait on QEMU 11.1.2 / Linux 7.2.11. Timeout diagnostics show the alarm stayed enabled (`alarm_IRQ: yes`) but no event was pending (`alrm_pending: no`) after RTC time passed the alarm. Cgroup-v2 samples include charged kernel/KVM memory, but they do not cover every host allocation. Other distributions' identity behavior remains unverified. QMP parking nearly stops guest clock progress. |
| Host blocked | Positive KSM measurements require write access to host-global `/sys/kernel/mm/ksm/run`; this user cannot change it. Current KSM state is `run=0`; no positive KSM result is claimed. |
| Experimental | Repeated migration proofs materialize v2/v3/v4 child RAM/disk templates; v4 stores relative lineage and qcow2 backing references. A moved three-generation synthetic tree passed Python/Rust validation, qcow2 chain inspection, and a fresh QEMU restore/COW run. `densevm validate template-tree ROOT` hashes every discovered generation's referenced artifacts and reports duplicate IDs. `densevm template gc-plan ROOT --state-dir DIR` prints candidate orphan generations without deleting anything; active/starting clone generations, explicit keeps, and their ancestors are protected. Marked orphan staging directories can be inspected and safely removed after process-reference checks. Persistent deletion, power-loss durability, partial moves, and legacy v1-v3 relocation remain open. See [`docs/template-tree.md`](docs/template-tree.md). |
| Deferred | Destructive template-tree garbage collection and TCG translated-code sharing. KVM guests still execute their own instructions. |

The paired KSM harness and its output format are documented in
[`docs/ksm-experiment.md`](docs/ksm-experiment.md). It is ready for an
administrator-enabled host; the current permission refusal is recorded under
`results/2026-10-24/ksm-comparison-blocked/`.

The optional cgroup-v2 memory accounting wrapper and its measured limits are
documented in [`docs/cgroup-memory-accounting.md`](docs/cgroup-memory-accounting.md).

The current read-only capability snapshot is [`results/2026-10-24/host-capabilities.json`](results/2026-10-24/host-capabilities.json). The test suite runs with `cargo test --all-targets`; opt-in KVM integration tests can be run with `cargo test --test probe -- --ignored --nocapture` on a host with adequate memory and QEMU/KVM.
