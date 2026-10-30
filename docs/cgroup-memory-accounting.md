# Cgroup memory accounting

Process PSS does not include every memory charge associated with a QEMU guest.
On a delegated Linux cgroup v2 host, the optional wrapper records the memory
controller's `memory.current` and `memory.stat` alongside the existing PSS
snapshots:

```sh
cargo build
experiments/cgroup/run-probe.sh \
  --out /path/to/new-output-directory \
  --count 2 \
  --skip-guest-oom \
  --divergence-percent 0
```

The wrapper finds a writable ancestor where the `memory` controller is already
enabled, creates a temporary child cgroup, and runs only the DenseVM probe
process tree inside it. It removes that child on exit. It does not enable a
controller, set a memory limit, or move unrelated processes. If no suitable
delegated cgroup exists, it refuses to run. Set `DENSEVM_BIN` to choose a built
DenseVM executable.

The result stores cgroup samples at conventional baseline readiness and at the
templated-clone snapshots before and after requested divergence. Each sample
contains the cgroup path, `memory.current`, and the full parsed `memory.stat`
map. `memory.stat` contains both byte-valued memory fields and non-byte
counters such as `pgfault`, so its JSON field name is intentionally unit
neutral. Without the wrapper, these fields say `not-requested`. PSS remains a
separate measurement; do not add PSS to cgroup memory because the values overlap.

Linux documents that cgroup v2 accounts major user and kernel memory classes,
but says its coverage is not completely watertight. `memory.stat` includes
kernel fields such as `kernel`, `pagetables`, and `sec_pagetables`; on x86 and
arm64, `sec_pagetables` includes KVM MMU allocations. This provides evidence
about charged kernel/KVM memory, not a complete inventory of all host kernel
allocations. See the [Linux cgroup v2 memory controller documentation](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html).

## Current sample

[`results/2026-10-24/cgroup-memory-summary.json`](../results/2026-10-24/cgroup-memory-summary.json)
summarizes two two-VM runs with 256 MiB configured RAM per guest and 0% requested
divergence. Each raw probe contains baseline and clone PSS and cgroup snapshots.
Across the runs, baseline process PSS was 406,919–424,456 KiB and clone PSS was
112,826–114,462 KiB. Baseline cgroup `memory.current` was 427,040,768–457,453,568
bytes; clone cgroup `memory.current` before divergence was 301,002,752–383,746,048
bytes. Cgroup `kernel` charges were 3,432,448–3,960,832 bytes for baseline and
7,823,360–7,847,936 bytes for the clone phase. Baseline `sec_pagetables` varied
from 122,880 to 745,472 bytes; the clone phase measured 409,600 bytes.

These are two sequential samples, not a controlled estimate of all host RAM;
`memory.current` varied with page-cache residency. The clone cgroup includes
the shared RAM file and disk page cache. Cgroup
charges for shared page-cache pages can depend on which cgroup first faulted
them in. The data shows why process PSS and cgroup accounting answer different
questions: PSS falls sharply for clones while file-cache and kernel charges
remain visible in the cgroup measurement.

An additional 2026-10-30 run after the KVM recovery is recorded at
[`results/kvm-recovery-2026-10-30/cg/`](../results/kvm-recovery-2026-10-30/cg/).
For two 256 MiB guests, conventional baseline PSS was 412,170 KiB and cgroup
`memory.current` was 416,010,240 bytes; clone PSS before divergence was 108,884
KiB and cgroup `memory.current` was 356,093,952 bytes. Baseline/clone
`sec_pagetables` were 405,504 / 409,600 bytes, and clone `kernel` charge was
7,684,096 bytes. These are one additional sample, not a revised estimate of
host-wide savings; the cgroup includes cache charges and does not guarantee
that every KVM allocation is charged inside the measured scope.

A matched 100%-of-probe-region divergence run is also recorded at
[`results/kvm-recovery-2026-10-30/cg100/`](../results/kvm-recovery-2026-10-30/cg100/).
This dirtied 32 MiB of clone A's 256 MiB guest RAM. A's QEMU PSS rose 32,813
KiB and its template mapping `Private_Dirty` rose 32,864 KiB; B's PSS rose 181
KiB and its mapping `Private_Dirty` rose 16 KiB. Cgroup `memory.current` rose
28,975,104 bytes, `kernel` rose 40,960 bytes, and `sec_pagetables` rose 45,056
bytes. This is one sample; cache charges and cgroup ownership affect the
relationship between these measurements. See the linked run README and raw
JSON for the full values.
