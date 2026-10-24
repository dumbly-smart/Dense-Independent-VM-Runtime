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
# Inspect saved disk and live RAM metadata:
cargo run -- inspect /tmp/densevm-my-run/clone-a.qcow2 disk
cargo run -- memory-map QEMU_PID /tmp/densevm-my-run/memory.img
cargo run -- ksm status
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
`--divergence-mib` controls how much of A's 32 MiB test region is dirtied;
`--divergence-percent` supports exact page-count sweeps from 0% to 100%.

## Current research status

| Status | Evidence |
| --- | --- |
| Demonstrated | Separate QEMU/KVM guests resume from one frozen state, share unchanged RAM pages, COW private RAM writes, and use private qcow2 overlays. Live 2/4/8/16 synthetic-clone scale measurements and a six-point 8-clone divergence sweep are saved under `results/`. |
| Isolation tested | Guest RAM and disk writes, unique MAC/machine-id/hostname/vmgenid, guest OOM, guest kernel panic, guest reboot, QEMU process termination, process state, and a six-guest Alpine range. |
| Partial | The opt-in Alpine profile verifies unique OpenSSH host-key fingerprints, DHCP option 61 identity, and the tested kernel's VMGenID reaction. Two parked-guest samples measure wall clock, uptime, and timerfd counters, but do not isolate timer expiration during pause or cover hardware RTC. Other distributions' identity behavior, register-level checks, and kernel/KVM memory overhead remain unverified. QMP parking nearly stops guest clock progress. |
| Host blocked | Positive KSM measurements require write access to host-global `/sys/kernel/mm/ksm/run`; this user cannot change it. Current KSM state is `run=0`; no positive KSM result is claimed. |
| Deferred | TCG translated-code sharing and hierarchical template trees. KVM guests still execute their own instructions. |

The paired KSM harness and its output format are documented in
[`docs/ksm-experiment.md`](docs/ksm-experiment.md). It is ready for an
administrator-enabled host; the current permission refusal is recorded under
`results/2026-10-24/ksm-comparison-blocked/`.

The current read-only capability snapshot is [`results/2026-10-24/host-capabilities.json`](results/2026-10-24/host-capabilities.json). The test suite runs with `cargo test --all-targets`; opt-in KVM integration tests can be run with `cargo test --test probe -- --ignored --nocapture` on a host with adequate memory and QEMU/KVM.
