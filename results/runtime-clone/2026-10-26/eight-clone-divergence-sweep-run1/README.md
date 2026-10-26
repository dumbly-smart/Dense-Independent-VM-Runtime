# Eight-clone RAM divergence sweep

Each data point is a separate run on 2026-10-26 using QEMU 11.1.2, Linux
7.2.11, KVM, 4 KiB pages, eight 256 MiB guests, and the same generated guest
artifacts for that run's independent baseline and restored clones. Clone A
writes the requested fraction of a 32 MiB guest RAM test region. The raw guest
metadata identifies this as a synthetic initramfs workload. Each
percentage directory contains the raw `result.json` and the output of
`densevm stats` in `summary.json`. `divergence.json` consolidates the six
measurements for plotting; `divergence.svg` is the generated chart. The plot
title uses the guest profile recorded in the raw result.

Reproduce each point with a fresh output directory:

```sh
densevm probe --out RUN_00 --count 8 --divergence-percent 0
densevm probe --out RUN_05 --count 8 --divergence-percent 5
densevm probe --out RUN_10 --count 8 --divergence-percent 10
densevm probe --out RUN_25 --count 8 --divergence-percent 25
densevm probe --out RUN_50 --count 8 --divergence-percent 50
densevm probe --out RUN_100 --count 8 --divergence-percent 100
densevm stats RUN_DIRECTORY/result.json
```

## Measured process memory

Values below are aggregate across eight QEMU processes. PSS and private memory
come from `/proc`; the cache column is the stats command's conservative
process-PSS-plus-full-template-cache bound.

| RAM divergence | Ordinary baseline PSS (KiB) | Clone PSS before (KiB) | Clone PSS after (KiB) | Clone private after (KiB) | Clone PSS + full cache (KiB) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0% | 1,625,501 | 462,966 | 462,997 | 421,416 | 725,141 |
| 5% | 1,631,204 | 462,587 | 464,251 | 423,064 | 726,395 |
| 10% | 1,577,420 | 463,125 | 466,414 | 425,556 | 728,558 |
| 25% | 1,625,180 | 463,313 | 471,536 | 430,196 | 733,680 |
| 50% | 1,631,348 | 462,268 | 478,602 | 437,592 | 740,746 |
| 100% | 1,628,054 | 463,201 | 495,980 | 454,652 | 758,124 |

From 0% to 100%, clone PSS after divergence increased by 32,983 KiB and
aggregate private memory increased by 33,236 KiB, approximately the 32 MiB
test region written by clone A. The other clones' RAM isolation checks passed
in each run. Each run's conventional baseline PSS varied from 1,577,420 to
1,631,348 KiB, so the table retains each baseline rather than treating one
sample as a fixed control.

These are process-level measurements, not exact total host memory. The
template RAM file was fully resident in cache; the PSS-plus-cache figure
double-counts some mapped pages. KVM metadata and host page tables are
`UNKNOWN`. Each result is one sample; collect repeated runs before making
statistical claims. Disk allocation is after the probe's disk-isolation
workload and does not represent pristine overlay size.

## Disk allocation during the probe

The two primary clone overlays each reported 200,704 allocated bytes at
creation. After the isolation workload, total allocation across all eight
clone overlays was 69,701,632 bytes in every run. Clone A, which performed the
full-disk write, ended at 67,379,200 bytes; clone B ended at 331,776 bytes.
The six additional clone overlays reported 331,776 bytes each. These
post-workload values show the cost of the test's deliberate disk writes and
must not be read as initial clone storage.
