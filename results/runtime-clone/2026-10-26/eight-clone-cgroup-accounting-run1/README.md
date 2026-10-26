# Eight-clone cgroup memory accounting

This run compared eight conventional VMs with eight template clones using
QEMU 11.1.2, Linux 7.2.11, KVM, and the synthetic initramfs guest. Each VM
had 256 MiB configured RAM. The probe ran in a user systemd scope with cgroup
v2 memory accounting enabled. Output files were on `/tmp` tmpfs and therefore
their charged pages are included in the cgroup readings.

Reproduce on a system with a delegated user cgroup and `systemd-run`:

```sh
systemd-run --user --scope --property=MemoryAccounting=yes /bin/sh -c '
  p=$(cut -d: -f3 /proc/self/cgroup)
  export DENSEVM_CGROUP_PATH=/sys/fs/cgroup$p
  exec densevm probe --out /tmp/densevm-cgroup-run --count 8 --divergence-percent 0
'
densevm stats /tmp/densevm-cgroup-run/result.json
```

## Measurements

| Accounting view | Eight conventional VMs | Eight template clones |
| --- | ---: | ---: |
| Process PSS (KiB) | 1,637,927 | 461,438 after 0% divergence |
| Cgroup `memory.current` before clone divergence (bytes) | 1,956,212,736 | 975,228,928 |
| Cgroup `memory.current` after clone probe (bytes) | — | 975,073,280 |
| Cgroup `memory.stat` anonymous (bytes) | 1,659,785,216 | 426,053,632 before divergence |
| Cgroup `memory.stat` file (bytes) | 278,016,000 | 532,832,256 before divergence |
| Cgroup `memory.stat` kernel (bytes) | 13,717,504 | 14,979,072 before divergence |
| Template-RAM mapping PSS (KiB) | — | 184,925 before divergence |
| Template-RAM shared mapped bytes summed over clone mappings (KiB) | — | 208,136 before divergence |
| Template-RAM private mapped bytes (KiB) | — | 158,168 before divergence |

The clone cgroup's `memory.current` was about 50.1% lower than the conventional
baseline in this sample. The process-PSS comparison alone suggested a larger
reduction (about 71.8%) because PSS does not account for every file-cache,
tmpfs, and charged-kernel byte in the cgroup. These views overlap and must not
be added together. `memory.current` is scope-local accounting, not a
host-global total; KVM or other kernel allocations not charged to the scope
may remain uncounted.

The mapping PSS approximates the resident physical footprint attributed to
these template-file VMAs. The summed `Shared_*` fields count a shared page once
for each clone mapping, so they are a mapping total rather than a count of
unique physical pages.

The result captures sequential snapshots while the baseline VMs or clone VMs
were live. The output directory was tmpfs, so the file-cache and shmem
components include this experiment's temporary disk and template-RAM files.
Persistent-disk runs will have different cgroup file-cache accounting. Raw
samples and all `memory.stat` fields are in `result.json`; `summary.json` is
the `densevm stats` output.
