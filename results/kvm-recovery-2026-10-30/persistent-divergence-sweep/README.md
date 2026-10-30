# Persistent eight-clone divergence sweep

This run used QEMU 11.1.2 / Linux 7.2.11, an eight-clone persistent template,
and a 32 MiB guest test region per clone. Every point starts a fresh set of
eight clones and dirties the stated fraction of the region in all eight. The
runner also stopped clone 1 at each point, verified its QEMU PID exited, and
read the RAM and disk markers from all seven remaining clones.

| Divergence per clone | Aggregate QEMU PSS before | After | PSS increase | Template mapping `Private_Dirty` increase |
| ---: | ---: | ---: | ---: | ---: |
| 0% | 311.1 MiB | 311.9 MiB | 0.8 MiB | 0.1 MiB |
| 5% | 312.3 MiB | 327.9 MiB | 15.6 MiB | 12.7 MiB |
| 10% | 312.2 MiB | 340.8 MiB | 28.6 MiB | 25.9 MiB |
| 25% | 312.5 MiB | 379.4 MiB | 66.9 MiB | 64.4 MiB |
| 50% | 311.9 MiB | 442.6 MiB | 130.7 MiB | 128.6 MiB |
| 100% | 311.3 MiB | 570.8 MiB | 259.5 MiB | 256.8 MiB |

PSS describes the eight QEMU processes only; this does not include KVM/kernel
memory or provide a matched conventional-VM baseline. At 100%, each guest
modified 32 MiB, one eighth of its configured 256 MiB RAM, not all guest RAM.
The run passed identity uniqueness, RAM/disk isolation, template hash, sibling
survival, and process cleanup checks at each percentage. These are six
sequential samples from one host, not repeated averages.

## Reproduction

```sh
cargo build
RUN=/tmp/densevm-current-divergence
target/debug/densevm probe --count 2 --out "$RUN/seed"
python3 experiments/template-tree/probe.py \
  --template-dir "$RUN/seed" --out "$RUN/tree"
python3 experiments/runtime-clone/persistent_sweep.py \
  --densevm target/debug/densevm \
  --manifest "$RUN/tree/generation.json" \
  --out "$RUN/sweep" --count 8 --region-mib 32
```

The raw per-point results, machine-readable summary, and SVG plot are in this
directory. The sweep runner leaves partial results and writes a failed status
if a percentage does not verify.
