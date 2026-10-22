# Live runs on QEMU 11.1.2 / Linux 7.2.11

`divergence/probe-8-*pct.json` contains six real eight-clone runs. Every run
passed RAM, disk, extra-clone, identity, template-RAM-integrity, and sibling
survival checks. The percentage applies to a 32 MiB buffer in **each** 256 MiB
guest. At 100%, only one eighth of configured guest RAM intentionally diverges.

| Buffer divergence | Conventional process PSS, MiB | Clone process PSS, MiB | Clone PSS + full template cache, MiB |
| --- | ---: | ---: | ---: |
| 0% | 1537.5 | 316.2 | 572.2 |
| 5% | 1578.6 | 330.5 | 586.5 |
| 10% | 1590.5 | 343.4 | 599.4 |
| 25% | 1594.0 | 383.1 | 639.1 |
| 50% | 1592.2 | 446.7 | 702.7 |
| 100% | 1577.3 | 574.9 | 830.9 |

The final column double-counts mapped template cache pages. It is a
conservative user-space/cache estimate, not total host physical memory.
Kernel/KVM metadata, host page tables, and baseline unmapped backing-file
cache are not included. The roughly 259 MiB PSS growth across the sweep is
consistent with eight independently dirtied 32 MiB buffers; it is an observed
result for this workload, not a forecast for ordinary applications.

The [graph](divergence/memory.svg) was generated with matplotlib from
`divergence/summary.json` using `experiments/divergence/plot.py`.
This is one sweep without confidence intervals; repeated runs are needed for
statistical performance claims. The runtime gained cancellation handling
during the sweep; QEMU commands and host versions are recorded per result,
but these early results do not embed an exact runtime executable hash.

`alpine-2.json` is the supported Alpine 3.24.2 ext4-root run with an observed
service/login checkpoint, RAM/filesystem isolation, identity preparation,
independent QEMU processes, and explicit parking. `alpine-2-summary.json`
contains the derived PSS/cache summary. No SSH or DHCP client is installed.

Large artifacts remain outside git under `/home/xtrmn8/densevm-sweep-20261022`
and `/tmp/densevm-alpine-login-guest` / `/tmp/densevm-alpine-complete-run`.
Temporary paths can disappear; the checked-in preparation scripts, source
archive checksum, command lines, and JSON data are the reproducibility record.
The KSM comparison was unavailable: sysfs `run` is owned by root and the
current account cannot enable it. Its value remained 0.

Filesystem fidelity correction: the first Alpine fixture above extracted
files as host UID 1000, which `mkfs.ext4 -d` preserved. Its RAM/disk isolation
results remain valid, but it is not evidence of faithful Unix file ownership.
The builder now runs under fakeroot, preserves archive UID/GID metadata, and
checks `/etc/passwd` is root:root in the generated ext4 image. The subsequent
`alpine-owned-2.json` run uses the corrected fixture.
