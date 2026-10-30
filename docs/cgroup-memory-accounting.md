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

The persistent clone runner also samples cgroup-v2 memory at four stages:
before clone creation, after all clones start, after requested RAM/disk
divergence, and after clone 1 stops while siblings remain live. By default it
resolves the current process's cgroup-v2 membership; set `DENSEVM_CGROUP_PATH`
to sample an explicit cgroup. Missing or malformed files produce an
`unavailable` sample and do not invalidate the VM run. The persistent sweep
summary reports cgroup deltas separately from QEMU PSS.

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

## Persistent eight-clone divergence

The first cgroup sweep at
[`persistent-cgroup-divergence-sweep`](../results/kvm-recovery-2026-10-30/persistent-cgroup-divergence-sweep/)
created its template before entering the measured scope. Its cgroup deltas
captured private clone growth but missed the shared template cache charge.
Its PSS/smaps records are valid; its cgroup totals are incomplete for total
clone footprint.

A corrected 2026-10-30 sweep built the conventional seed, nested template, and
all eight persistent clones inside one transient systemd cgroup capped at 3
GiB. The cgroup's `memory.stat file` was about 583 MiB after clone startup,
showing that shared template file-cache pages were charged inside the same
scope. At 0%, 5%, 10%, 25%, 50%, and 100% divergence of each 32 MiB test
region, cgroup `memory.current` from clone startup to post-divergence rose
1.6, 18.6, 32.5, 70.7, 134.8, and 261.9 MiB. At 100%, `memory.stat` anonymous
memory rose 261.3 MiB, file memory 1.7 MiB, `kernel` 0.30 MiB, and
`sec_pagetables` 0.38 MiB. Full staged data, limit values, and reproduction:
[`full-scope cgroup sweep`](../results/kvm-recovery-2026-10-30/persistent-cgroup-divergence-sweep-fullscope/).

A separate conventional eight-VM KVM baseline under the same 3 GiB scope cap
measured 1,618,782 KiB QEMU-process PSS and cgroup `memory.current` of
1,660,567,552 bytes at guest readiness. The full-scope 100%-divergence clone
point measured 568.9 MiB QEMU PSS and 1,167.7 MiB cgroup memory. This is one
sequential comparison on one host; the clone cgroup includes about 585 MiB of
shared template file cache, while the baseline cgroup did not. Cgroup charges
can also depend on which scope first faults a shared page. Treat these values
as measured samples, not a general density bound. Raw baseline:
[`baseline-cgroup-eight.json`](../results/kvm-recovery-2026-10-30/baseline-cgroup-eight.json).
