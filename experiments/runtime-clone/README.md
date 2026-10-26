# Runtime clone experiments

## Re-run the sequential RAM divergence sweep

Build `densevm`, ensure KVM is available, then run from the repository root:

```sh
python3 experiments/runtime-clone/sweep_driver.py \
  --densevm target/debug/densevm \
  --out results/runtime-clone/NEW-RUN \
  --count 8
```

The output directory must not already exist. The driver runs the six standard
divergence points (0%, 5%, 10%, 25%, 50%, 100%) sequentially. Each `densevm
probe` invocation performs its own available-memory check before starting
guests. The driver writes the raw probe result, a `densevm stats` summary, and
an incrementally updated `sweep.json` index for each completed point. It
marks the sweep verified only when every requested percentage, clone count,
and RAM/disk isolation check matches the request.

Use `--percentages 0 25 100` to select a smaller ascending sweep. The probe
supports at most 32 clones; the host's memory guard may reject a requested
count before launching VMs.

## Persistent runtime clone sweeps

To exercise the persistent `clone start/stop` path against a prepared template
generation, run:

```sh
python3 experiments/runtime-clone/persistent_sweep.py \
  --manifest /path/to/generation.json \
  --out results/runtime-clone/NEW-PERSISTENT-RUN \
  --count 8
```

The default sweep changes 0%, 5%, 10%, 25%, 50%, and 100% of a 32 MiB guest
test region in every clone, using a different byte value per VM. Each point
starts separate QEMU processes, reads RAM and disk markers from all guests,
checks clone identities and immutable template hashes, records process PSS and
template-file mapping counters before and after divergence, then stops every
clone. The output contains raw results, an incrementally written `sweep.json`,
and an SVG graph. Its memory results are QEMU process measurements and do not
include all KVM/kernel allocations.

To select a smaller ascending set of values, use
`--percentages 0 25 100`. The current RAM probe is limited to a 32 MiB region;
full-guest-RAM divergence is a separate experiment. Disk overlays receive one
private marker write at each nonzero point, which verifies disk isolation but
does not scale disk divergence with the RAM percentage.
