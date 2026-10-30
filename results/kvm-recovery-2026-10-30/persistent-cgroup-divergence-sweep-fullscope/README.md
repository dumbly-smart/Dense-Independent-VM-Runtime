# Full-scope cgroup comparison for persistent clones

This corrected sweep builds the conventional seed and nested template, then
runs all six eight-clone divergence points inside one transient systemd scope.
This keeps the template files and their page-cache charges in the same cgroup
as the persistent clones. The scope used cgroup-v2 `MemoryAccounting=yes` and
`MemoryMax=3G`; each recorded `memory.max` was 3,221,225,472 bytes. Each guest
has 256 MiB configured RAM and the changed region is 32 MiB.

| Divergence in each clone | QEMU PSS increase | RAM mapping `Private_Dirty` increase | Cgroup `memory.current` increase, clone start to divergence | Cgroup `memory.current` after divergence |
| ---: | ---: | ---: | ---: | ---: |
| 0% | 0.8 MiB | 0.1 MiB | 1.6 MiB | 907.3 MiB |
| 5% | 15.8 MiB | 13.0 MiB | 18.6 MiB | 923.6 MiB |
| 10% | 28.8 MiB | 26.0 MiB | 32.5 MiB | 936.3 MiB |
| 25% | 67.1 MiB | 64.5 MiB | 70.7 MiB | 976.6 MiB |
| 50% | 131.3 MiB | 128.7 MiB | 134.8 MiB | 1,040.3 MiB |
| 100% | 260.1 MiB | 257.5 MiB | 261.9 MiB | 1,167.7 MiB |

At 100%, `memory.stat` anonymous memory rose from 296.4 to 557.7 MiB between
clone start and post-divergence. File memory rose from 583.8 to 585.5 MiB;
`kernel` rose from 22.45 to 22.75 MiB; `sec_pagetables` rose from 1.53 to 1.91
MiB. The unchanged template file cache stays charged to the cgroup throughout
the sweep. All six points passed RAM/disk isolation and clone-1-stop sibling
checks, with no leftover QEMU processes.

A separate conventional eight-VM baseline used the same QEMU version, guest
RAM size, KVM acceleration, and 3 GiB cgroup cap. At readiness it measured
1,618,782 KiB aggregate QEMU PSS and cgroup `memory.current` of
1,660,567,552 bytes (1,583.6 MiB). Its `memory.stat` reported 1,563.9 MiB anon,
2.6 MiB file, 12.2 MiB `kernel`, and 1.0 MiB `sec_pagetables`. The raw baseline
is [`../baseline-cgroup-eight.json`](../baseline-cgroup-eight.json).

The 100%-divergence clone point measured 568.9 MiB aggregate QEMU PSS and
1,167.7 MiB cgroup `memory.current`. In this sample, the cgroup total is about
416 MiB lower than the conventional baseline, even with about 585 MiB charged
file cache for the shared template. This is an observed comparison between
two sequential, isolated-scope runs on one host, not a general density bound.
PSS and cgroup values overlap and must not be added. Cgroup charges can depend
on which scope first faults a shared file page, and neither metric accounts
for memory outside its measured processes or cgroup.

## Reproduction

Build DenseVM and run setup plus sweep in one new scope. The temporary
directory must be short enough to keep QEMU Unix socket paths below Linux's
pathname limit. Use a fresh output directory and unit name each time.

```sh
RUN=/home/$USER/dvfull
TMP=/home/$USER/dvtmp
mkdir -p "$RUN" "$TMP"
systemd-run --user --scope \
  --property=MemoryAccounting=yes \
  --property=MemoryMax=3G \
  --unit=densevm-cgroup-full-unique \
  -- /bin/bash -c '
    set -euo pipefail
    export TMPDIR="$2"
    target/debug/densevm probe --count 2 --out "$1/seed"
    python3 experiments/template-tree/probe.py \
      --template-dir "$1/seed" --out "$1/tree"
    python3 experiments/runtime-clone/persistent_sweep.py \
      --densevm target/debug/densevm \
      --manifest "$1/tree/generation.json" \
      --out "$1/sweep" --count 8 --region-mib 32
  ' _ "$RUN" "$TMP"
```

For the conventional baseline, use a second scope with the same properties.
The command derives that scope's cgroup-v2 path and exports it so DenseVM
records the cgroup snapshot at guest readiness:

```sh
BASE=/home/$USER/dvbase
mkdir -p "$BASE"
systemd-run --user --scope \
  --property=MemoryAccounting=yes \
  --property=MemoryMax=3G \
  --unit=densevm-cgroup-baseline-unique \
  -- /bin/bash -c '
    set -euo pipefail
    REL=$(cut -d: -f3 /proc/self/cgroup)
    export DENSEVM_CGROUP_PATH="/sys/fs/cgroup${REL}"
    target/debug/densevm benchmark baseline --out "$1/baseline" --count 8
  ' _ "$BASE"
```

The full per-point JSON, cgroup snapshots, machine-readable sweep index, and
SVG graph are stored alongside this README.
