# Eight-clone divergence sweep

Run on 2026-10-24 with QEMU 11.1.2, Linux 7.2.11, KVM, and the synthetic guest. Each point launched eight conventional baseline guests and eight DenseVM clones, then dirtied the same requested percentage of each clone's 32 MiB test buffer. Configured RAM was 256 MiB per guest. Reproduce with:

```sh
cargo build
experiments/divergence/run.sh /tmp/densevm-divergence target/debug/densevm 8
uv run --with matplotlib python experiments/divergence/plot.py \
  /tmp/densevm-divergence/divergence.json \
  /tmp/densevm-divergence/divergence.svg
```

`summary.json` contains the six aggregated machine-readable measurements; `divergence-{percent}pct.json` contains the full raw probe record for each point. [`divergence.svg`](divergence.svg) plots the measured process PSS and a separately labeled PSS-plus-template-cache upper bound.

| Test-buffer divergence | Baseline process PSS | Clone process PSS after writes | Clone PSS + full template cache | Clone ready time |
| ---: | ---: | ---: | ---: | ---: |
| 0% | 1,584 MiB | 456 MiB | 709 MiB | 1,436 ms |
| 5% | 1,583 MiB | 469 MiB | 722 MiB | 2,122 ms |
| 10% | 1,591 MiB | 477 MiB | 731 MiB | 1,865 ms |
| 25% | 1,594 MiB | 516 MiB | 770 MiB | 1,491 ms |
| 50% | 1,593 MiB | 580 MiB | 834 MiB | 1,303 ms |
| 100% | 1,576 MiB | 708 MiB | 962 MiB | 1,964 ms |

Values are summed process-level observations for the eight guests. The test region is 32 MiB, one eighth of configured guest RAM; 100% here does not mean all guest RAM diverged. The cache bound adds the entire resident template file to clone process PSS and therefore intentionally double-counts mapped template pages. Neither series includes complete host physical memory: KVM metadata, QEMU kernel allocations, and host page tables remain unknown. PSS is sampled from procfs while guests are live, so small run-to-run changes are expected. KSM remained disabled (`run=0`) for these runs.

Clone PSS increases by about 252 MiB from 0% to 100%, close to eight clones each privatizing 32 MiB. Disk allocation stayed at 200,704 bytes for the base plus 69,697,536 bytes across overlays in every point. These results support the expected COW divergence trend for the test buffer; they do not establish a full-guest memory density curve.
