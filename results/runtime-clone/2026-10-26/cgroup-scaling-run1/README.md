# Cgroup memory scaling samples

These are the first of two runs each at 2, 4, and 8 VMs. All used QEMU 11.1.2, Linux
7.2.11, KVM, synthetic initramfs guests with 256 MiB configured RAM per VM,
and cgroup v2 `MemoryAccounting=yes` in a user systemd scope. The 2- and
4-VM raw records are in this directory; the 8-VM record is
[`eight-clone-cgroup-accounting-run1`](../eight-clone-cgroup-accounting-run1/).
All outputs were under `/tmp` tmpfs, so cgroup file/shmem values include
experiment artifacts.

| VM count | Configured guest RAM (MiB) | Baseline PSS (KiB) | Baseline `memory.current` (bytes) | Baseline anon (bytes) | Clone PSS (KiB) | Clone `memory.current` (bytes) | Clone anon (bytes) | Clone file (bytes) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 512 | 418,484 | 550,977,536 | 414,277,632 | 247,244 | 614,260,736 | 209,272,832 | 398,032,896 |
| 4 | 1,024 | 821,796 | 844,959,744 | 826,044,416 | 319,004 | 567,779,328 | 281,870,336 | 276,729,856 |
| 8 | 2,048 | 1,637,927 | 1,956,212,736 | 1,659,785,216 | 461,424 | 975,228,928 | 426,053,632 | 532,832,256 |

Ordinary-baseline process PSS and cgroup anonymous memory rise roughly with
VM count. Clone process PSS also rises with count, while cgroup
`memory.current` is not monotonic between the 2- and 4-VM samples because
charged file-cache pages varied substantially. The `/tmp` tmpfs experiment
files contribute to `memory.stat file` and `shmem`. This small set demonstrates
why the process PSS and cgroup views should be reported together, not treated
as interchangeable. These are single samples, not a statistical scaling
curve. `memory.current` is limited to the measured scope; kernel allocations
not charged to it may be absent.

Reproduce a point by launching the probe in a memory-accounted systemd scope
and setting `DENSEVM_CGROUP_PATH` to the scope path as shown in the eight-VM
sample README.

Repeat measurements are documented in
[`../cgroup-scaling-run2/`](../cgroup-scaling-run2/).
