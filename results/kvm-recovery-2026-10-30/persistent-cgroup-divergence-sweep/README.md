# Cgroup-accounted persistent divergence sweep

The six runs repeated the persistent eight-clone divergence sweep inside a
dedicated user systemd scope with cgroup-v2 memory accounting and a 3 GiB
`MemoryMax`. Each raw result records `memory.max` as 3,221,225,472 bytes and
samples `memory.current` plus `memory.stat` before clone startup, after startup,
after divergence, and after clone 1 stopped. QEMU PSS and RAM-mapping smaps
remain separately reported.

| Divergence per clone | QEMU PSS increase | RAM mapping `Private_Dirty` increase | Cgroup `memory.current` increase after clone start |
| ---: | ---: | ---: | ---: |
| 0% | 0.2 MiB | 0.1 MiB | 2.5 MiB |
| 5% | 16.5 MiB | 13.1 MiB | 18.4 MiB |
| 10% | 28.8 MiB | 25.9 MiB | 31.1 MiB |
| 25% | 67.2 MiB | 64.4 MiB | 70.2 MiB |
| 50% | 131.3 MiB | 128.6 MiB | 133.9 MiB |
| 100% | 259.0 MiB | 256.9 MiB | 260.8 MiB |

At the 100% point, each clone changed 32 MiB, one eighth of its configured
256 MiB guest RAM. Between the `after_clone_start` and `after_divergence`
samples, `memory.stat` anonymous memory rose 261.35 MiB, file memory rose 0.50
MiB, `kernel` rose 0.26 MiB, and `sec_pagetables` rose 0.38 MiB. These cgroup
charges track the expected COW growth, but do not identify which individual
guest pages were charged.

The cgroup is an isolated experiment scope, not the entire host. Its
`memory.current` includes file-cache and charged kernel memory; the
`before_clones` sample also includes cached seed/template artifacts. The
clone-start-to-divergence delta is the cleaner comparison, but still includes
guest commands and other scope activity during that interval. No matched
conventional-VM cgroup baseline was collected in this sweep. The 3 GiB cap
limits experiment scope growth; it is not a claim that the eight guests use
only 3 GiB or less on an unconstrained host.

## Reproduction

Build DenseVM, then run the setup and sweep together inside a transient systemd
scope so template file-cache charges and all QEMU processes are accounted in
the same cgroup:

```sh
RUN=/home/$USER/dvcg
TMP=/home/$USER/dvtmp
mkdir -p "$RUN" "$TMP"
systemd-run --user --scope \
  --property=MemoryAccounting=yes \
  --property=MemoryMax=3G \
  --unit=densevm-cgroup-sweep-unique \
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

Use a fresh `RUN` directory and a unique `--unit` name for each run. Keep the
temporary directory short enough for QEMU Unix-socket paths to fit Linux's
pathname limit. Raw per-point results and the generated plot are in this
directory.
