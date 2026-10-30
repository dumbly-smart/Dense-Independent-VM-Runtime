# Two-clone cgroup memory sample

Run on 2026-10-30 with QEMU 11.1.2 / Linux 7.2.11-arch1-1 using:

```sh
experiments/cgroup/run-probe.sh --out results/kvm-recovery-2026-10-30/cg \
  --count 2 --skip-guest-oom --divergence-percent 0
```

The wrapper created a child under the already-enabled delegated cgroup memory
controller and did not set a memory limit or change host controller settings.
The two guests had 256 MiB configured RAM each. Baseline and clone PSS are
sums across two QEMU processes. The result JSON retains all cgroup snapshots,
PSS values, guest isolation flags, and host metadata. Temporary qcow2, RAM,
initramfs, and machine-state artifacts were removed after collection to keep
this record small; the command above recreates them.

| Phase | Aggregate QEMU PSS | cgroup `memory.current` | `kernel` | `sec_pagetables` |
| --- | ---: | ---: | ---: | ---: |
| Conventional baseline | 402.5 MiB | 396.7 MiB | 3.4 MiB | 396 KiB |
| Template clones before divergence | 106.3 MiB | 339.6 MiB | 7.3 MiB | 400 KiB |
| Template clones after divergence | 106.3 MiB | 339.8 MiB | 7.3 MiB | 400 KiB |

The cgroup `memory.current` value is not process PSS and can include file-cache
charges, while process PSS excludes kernel memory. The clone cgroup charged
273,006,592 bytes as `file`; the template RAM mapping and disk cache are part
of the observed footprint. `sec_pagetables` includes KVM MMU allocations on
this kernel, but neither this field nor cgroup accounting is a complete
inventory of host memory. This is one run; page-cache residency affects these
values. `result.json` is the machine-readable raw record; `summary.json` is
generated from it with `densevm stats`.
