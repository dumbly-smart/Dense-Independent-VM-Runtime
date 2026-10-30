# Cgroup sweep with template cache outside the measured scope

These six persistent eight-clone runs were executed inside a capped 3 GiB
systemd scope, but the template was generated before entering that scope. The
raw cgroup samples therefore do not include the charge for shared template
RAM pages: at clone startup, `memory.stat file` was only about 1 MiB. The
QEMU-process PSS and `smaps` measurements remain valid; the cgroup values are
only the charges assigned to this scope and must not be read as the complete
physical footprint of the clone set.

From clone startup to divergence, the recorded cgroup `memory.current` delta
was 2.5, 18.4, 31.1, 70.2, 133.9, and 260.8 MiB at 0%, 5%, 10%, 25%, 50%, and
100%. These deltas show the private anonymous-memory growth, but omit shared
template file-cache pages charged elsewhere. Each run used the 3 GiB cgroup
limit and verified all seven siblings after clone 1 stopped.

The corrected rerun that builds the seed, template, and clones in one measured
cgroup is [`../persistent-cgroup-divergence-sweep-fullscope/`](../persistent-cgroup-divergence-sweep-fullscope/).

## Reproducing this scope-only sample

```sh
systemd-run --user --scope \
  --property=MemoryAccounting=yes \
  --property=MemoryMax=3G \
  --unit=densevm-cgroup-sweep-unique \
  -- env TMPDIR=/home/$USER/dvtmp \
  python3 experiments/runtime-clone/persistent_sweep.py \
    --densevm target/debug/densevm \
    --manifest /path/to/already-built/generation.json \
    --out /path/to/new/sweep --count 8 --region-mib 32
```

Raw per-point records and the generated PSS/private-dirty plot remain here for
comparison with the corrected full-scope run.
