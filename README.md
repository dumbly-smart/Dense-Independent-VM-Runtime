# DenseVM

DenseVM is a systems research prototype for QEMU/KVM VM templating. Its first
experiment freezes one Linux guest, resumes two independent QEMU processes
from that state, maps the same RAM file privately, and uses distinct qcow2
disk overlays. It measures isolation and host memory with QMP, `/proc/smaps`,
and `fincore`.

The included guest is a small static Rust `/init` running the host's Linux
kernel. It proves the mechanism, not yet a general Linux distribution with a
login prompt. See [design](docs/design.md),
[measured findings](docs/first-probe.md), and
[semantic differences](docs/semantic-differences.md).

## Reproduce the two-clone experiment

On the inspected Arch Linux host (QEMU 11.1.1, kernel 7.2.6), install or make
available `qemu-system-x86_64`, `qemu-img`, `rustc`, `cargo`, `cpio`,
`modinfo`, `fincore`, and `sha256sum`. The host kernel and its matching
`virtio_net`, `net_failover`, and `failover` modules must be present, as must
`/dev/kvm` and `/boot/vmlinuz-linux`. The probe uses 256 MiB guest RAM,
roughly 160 MiB of allocated template RAM file storage, and a 64 MiB virtual
disk. KSM stays in its existing state; the reference run had it disabled.

```sh
cargo test
cargo run -- doctor
cargo test --test probe -- --ignored --nocapture
# Or run outside the test harness, specifying a NEW directory:
cargo run -- probe --out /tmp/densevm-my-run
# Limit the intentional RAM divergence used by the isolation probe:
cargo run -- probe --out /tmp/densevm-my-8m-run --divergence-mib 8
# Measure explicit QMP stop/resume of idle clones:
cargo run -- probe --out /tmp/densevm-park-run --park-idle
# Compare the same probe with KSM temporarily enabled; prior KSM settings are restored:
cargo run -- probe --out /tmp/densevm-my-ksm-run --ksm
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

This prototype is pinned to mechanisms confirmed on QEMU 11.1.1.
`x-ignore-shared` is experimental. Do not treat this probe as evidence that
arbitrary distributions, devices, or QEMU versions can be cloned safely.

The 15-item acceptance record is in [docs/milestone1.md](docs/milestone1.md).

The baseline command supports counts from 1 through 32 and writes
`baseline.json`. The template restore probe supports 2 through 32 clones;
the first two receive detailed RAM, disk, reboot, and process-kill isolation
checks, while additional clones contribute restore and memory-map measurements.
`--divergence-mib` controls how much of A's 32 MiB test region is dirtied;
`--divergence-percent` supports exact page-count sweeps from 0% to 100%.
