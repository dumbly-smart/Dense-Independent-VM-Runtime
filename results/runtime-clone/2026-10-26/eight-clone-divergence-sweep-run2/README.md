# Eight-clone RAM divergence sweep, repeat run

This is a second sequential sweep with eight ordinary baseline VMs and eight
template clones at each point. It ran on 2026-10-26 with QEMU 11.1.2, Linux
7.2.11, KVM, 4 KiB pages, 256 MiB guest RAM per VM, and the synthetic
initramfs workload. Clone A wrote 0%, 5%, 10%, 25%, 50%, and 100% of its
32 MiB test region in separate fresh runs. Each percentage directory contains
raw `result.json` and `densevm stats` `summary.json`; `sweep.json` records
driver verification, and `divergence.json` / `divergence.svg` provide the
plot input and chart.

Reproduce with:

```sh
python3 experiments/runtime-clone/sweep_driver.py \
  --densevm target/debug/densevm \
  --out /tmp/densevm-divergence-sweep \
  --count 8
```

## Process PSS

| RAM divergence | Ordinary baseline PSS (KiB) | Clone PSS before (KiB) | Clone PSS after (KiB) | Clone private after (KiB) | Clone PSS + full cache (KiB) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0% | 1,634,241 | 467,328 | 467,336 | 426,100 | 726,968 |
| 5% | 1,625,829 | 463,440 | 465,187 | 424,552 | 725,519 |
| 10% | 1,631,723 | 462,832 | 466,184 | 425,080 | 726,524 |
| 25% | 1,633,953 | 470,006 | 478,197 | 436,344 | 737,689 |
| 50% | 1,613,415 | 464,044 | 480,452 | 439,648 | 740,788 |
| 100% | 1,629,433 | 462,693 | 495,448 | 453,940 | 755,452 |

From 0% to 100%, aggregate clone PSS rose by 28,112 KiB and private memory
rose by 27,840 KiB. The prior sweep measured increases of 32,983 KiB and
33,236 KiB respectively. Both runs show growth on the scale of the 32 MiB
region, but the measured deltas vary by several MiB. Treat process PSS as a
noisy, incomplete host-memory measure: KVM metadata and host page tables are
excluded, and template-file cache accounting is approximate. Each sweep is
one sample at each divergence level; the two sweeps are not a statistical
distribution. All six runs reported verified guest RAM and disk isolation.
