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
