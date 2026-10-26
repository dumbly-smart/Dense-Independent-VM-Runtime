# Cgroup memory scaling, repeat samples

This is a repeat of the 2/4/8-VM cgroup experiment in
[`../cgroup-scaling-run1/`](../cgroup-scaling-run1/). Each point used QEMU
11.1.2, Linux 7.2.11, KVM, synthetic initramfs guests with 256 MiB configured
RAM per VM, and a user systemd scope with cgroup v2 memory accounting.
Experiment output was on `/tmp` tmpfs.

| VMs | Run | Baseline process PSS (KiB) | Baseline `memory.current` (bytes) | Clone process PSS (KiB) | Clone `memory.current` (bytes) | Clone anon (bytes) | Clone file (bytes) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1 | 418,484 | 550,977,536 | 247,244 | 614,260,736 | 209,272,832 | 398,032,896 |
| 2 | 2 | 408,334 | 413,618,176 | 247,591 | 487,395,328 | 208,875,520 | 271,814,656 |
| 4 | 1 | 821,796 | 844,959,744 | 319,004 | 567,779,328 | 281,870,336 | 276,729,856 |
| 4 | 2 | 817,565 | 836,386,816 | 319,765 | 565,624,832 | 282,722,304 | 273,145,856 |
| 8 | 1 | 1,637,927 | 1,956,212,736 | 461,424 | 975,228,928 | 426,053,632 | 532,832,256 |
| 8 | 2 | 1,623,404 | 1,666,613,248 | 472,341 | 719,253,504 | 428,134,400 | 275,443,712 |

Process PSS is more repeatable than `memory.current` here: baseline PSS varied
by up to about 2.5%, and clone PSS by less than 3%. Cgroup current varied more,
especially for clone runs, as `memory.stat file` shifted substantially between
samples. In the first 8-VM run it was about 533 MiB; in the second it was
about 275 MiB. The RAM file and other output were on tmpfs, so file/shmem
charges, cache residency, and reclaim affected the cgroup total. Raw
`memory.stat` breakdowns are preserved under `2-vm/`, `4-vm/`, and `8-vm/`.

This shows that cgroup `memory.current` includes useful host charges that
process PSS misses, while also being sensitive to cache state. These two
samples do not establish a stable cgroup density curve. See the first-run
README for the full reproduction command and accounting caveats.
