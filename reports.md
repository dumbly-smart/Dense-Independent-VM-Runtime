# DenseVM reports and evidence index

This page is an index to saved experiments, not a replacement for their raw
JSON and logs. Claims below are bounded by the guest, QEMU, kernel, and sample
count recorded by each experiment. The detailed dated history is
[`docs/progress.md`](docs/progress.md); semantic scope is in
[`docs/semantic-differences.md`](docs/semantic-differences.md).

## How to read a result

- **Configured RAM** is what the guest sees. It is not host physical use.
- **RSS** counts every resident page in a process, including pages shared with
  another process. Summing RSS double-counts shared pages.
- **PSS** divides shared resident pages among mappings. Summed QEMU PSS is a
  useful user-space comparison, but it is not a complete host-memory total.
- **Template cache** is resident data from the backing RAM file. A cache upper
  bound plus process PSS is conservative and may double-count some pages.
- **Cgroup `memory.current`** includes memory charged to that cgroup, such as
  some kernel and file-cache pages. It can omit allocations and is sensitive
  to cache/reclaim behavior.
- **Private dirty pages** in the clone's exact template-RAM mapping are direct
  evidence of COW divergence. They are not all private memory in the process.
- **Virtual disk size** is the guest-visible capacity. Allocated bytes are
  the host storage consumed by a qcow2 file; base plus overlays and chain
  relationships must be reported separately.
- **Translation-block count** is not a count of unique TBs. Repeated guest PC
  values are only a PC-reuse indicator.
- **Translation duration** in the QEMU patch is monotonic elapsed wall time
  around successful `setjmp_gen_code` calls. It is not thread CPU time and can
  include host scheduling delay.

## Main experiment families

| Evidence | What it covers | Main limit |
| --- | --- | --- |
| [`results/probe-2026-09-22-qemu-11.1.1.json`](results/probe-2026-09-22-qemu-11.1.1.json) | Initial QEMU 11.1.1 synthetic two-clone RAM/disk COW proof. | Historical host/QEMU; process PSS and tiny guest do not represent full host cost. |
| [`results/2026-10-24/`](results/2026-10-24/) | Host capabilities, identity, timers, disk I/O, network, OOM, crash, register/APIC, cgroup, and multi-guest range experiments. | Mostly synthetic guest; individual probes have bounded scope and small sample counts. |
| [`results/divergence/2026-10-24/`](results/divergence/2026-10-24/) | Eight-clone controlled 0–100% test-buffer divergence curve. | 32 MiB test buffer is only one eighth of configured guest RAM; PSS excludes KVM/kernel memory. |
| [`results/scale/2026-10-24/`](results/scale/2026-10-24/) | Conventional versus template-clone PSS at 2, 4, 8, and 16 clones. | Process PSS only; 32 clones were declined by the safe-memory preflight. |
| [`results/template-tree/2026-10-25/`](results/template-tree/2026-10-25/) and [`2026-10-26/`](results/template-tree/2026-10-26/) | Multi-generation RAM/disk template materialization and relocation checks. `validate template-tree` verifies discovered generations; `template gc-plan` reports dry-run candidates after validating integrity and clone references. | Planner never deletes files; the catalog is not locked against concurrent mutation, and persistent lifecycle/durability remain incomplete. |
| [`results/runtime-clone/2026-10-26/`](results/runtime-clone/2026-10-26/) | Persistent clone lifecycle, 8-clone divergence/cgroup measurements, Alpine range, and 21-VM security range. | KVM/kernel costs are not fully included in process PSS; some range data used tmpfs. |
| [`results/tcg/2026-10-26/`](results/tcg/2026-10-26/) | Repeated conventional KVM/TCG baseline, TCG trace, patched code-size/duration, and stock fallback. | TCG and KVM use different CPU models; small sequential samples. |
| [`results/tcg/2026-10-27/`](results/tcg/2026-10-27/) | Three patched phase traces, three untraced controls, and stock-QEMU phase fallback. | Tracing and QMP phase stops perturb execution; only three sequential samples per patched mode. |
| [`results/vcpu-multi2/`](results/vcpu-multi2/) and [`run2`](results/vcpu-multi2-run2/) | Two-vCPU template migration, restore, and clone isolation, repeated twice. | These historical snapshots show CPU#0 only; later live capture of both CPU indexes is recorded under `results/kvm-recovery-2026-10-30/`. |
| [`results/vcpu-qmp-smoke-2026-10-29/`](results/vcpu-qmp-smoke-2026-10-29/), [`results/vcpu-topology-2026-10-29/`](results/vcpu-topology-2026-10-29/), [`results/kvm-entry-2026-10-29/`](results/kvm-entry-2026-10-29/), and [`results/kvm-cpu-model-smoke-2026-10-29/`](results/kvm-cpu-model-smoke-2026-10-29/) | TCG smoke for two-vCPU QMP/HMP interfaces; KVM attempts at one and two vCPUs and under `host`/`qemu64` CPU models. | Those 2026-10-29 attempts failed before guest readiness; the later recovery result is listed separately. |
| [`results/kvm-recovery-2026-10-30/`](results/kvm-recovery-2026-10-30/) | Successful two-clone KVM restore with RAM/disk divergence, unique identity, reboot, and sibling survival; one- and two-vCPU register capture; per-CPU TPR isolation on CPUs 0 and 1; cgroup-v2 baseline and 32 MiB divergence measurements. | Small synthetic-guest samples; cgroup includes cache charges but is not a complete host-memory inventory. APIC state beyond TPR and pending interrupts remain unverified. |
| [`results/2026-10-24/ksm-comparison-blocked/`](results/2026-10-24/ksm-comparison-blocked/) | Read-only KSM capability/preflight output. | Positive KSM merge requires host-global write permission unavailable to this user. |

## Selected measured outcomes

### Shared-memory density

The 21-VM Alpine range comparison used 20 targets plus an attacker. Before
intentional RAM divergence, aggregate clone QEMU process PSS was 997,844 KiB;
the equivalent conventional process baseline was 4,222,425 KiB. After a clone
dirtied its 32 MiB test area, aggregate clone PSS was 1,030,264 KiB. This is a
large measured process-PSS reduction for that run, not a claim that all 21
guests' configured RAM fit in 1 GiB. Kernel/KVM memory is excluded, and the
evidence is one run. See
[`alpine-twenty-target-range-run1`](results/runtime-clone/2026-10-26/alpine-twenty-target-range-run1/).

The repeated persistent eight-clone sweep showed template-mapping
`Private_Dirty` growing by approximately 0.3, 13.4, 26.2, 64.7, 128.9, and
257.2 MiB as every clone dirtied 0%, 5%, 10%, 25%, 50%, and 100% of its 32 MiB
test region. At the fully dirtied endpoint, QEMU process PSS increased by
about 259.4 MiB. The 100% value means 100% of the test region, not all guest
RAM. See the two persistent divergence sweep directories listed above.

### TCG phase traces

Three patched QEMU 11.1.2 phase runs recorded:

| Phase | Translation starts | Completed blocks | Generated bytes (cumulative) | Successful generation wall time |
| --- | ---: | ---: | ---: | ---: |
| Boot/readiness | 107,795–108,304 | 107,775–108,284 | 33,724,574–33,900,046 | 1,225,067–1,343,860 µs |
| Post-ready idle | 189–517 | 189–517 | 65,660–153,949 | 3,824–5,863 µs |
| Active CPU workload | 711–763 | 711–763 | 226,886–244,054 | 8,920–18,365 µs |

Independent recounts matched each phase's byte range and the complete log.
The phase boundaries are QMP pause points, not instruction-exact boundaries.
The stock-QEMU phase sample retained ordinary event counts but reported
patched code-size and duration metrics as unavailable. The patched, untraced
controls averaged 8,761 ms readiness and 299,099 KiB PSS; the traced samples
averaged 8,518 ms readiness and 302,994 KiB PSS. The overlapping ranges and
sequential n=3 samples do not establish overhead or a startup benefit.

### Two-vCPU restore

Two repeated `--vcpus 2` KVM runs used `-smp 2` for every baseline VM, source
template, and restored clone. Both passed RAM isolation, disk isolation,
backing verification, unique MAC/machine identity/vmgenid, vCPU0 register
sampling, and sibling survival after QEMU A was killed. They establish that
this two-vCPU migration configuration restored and ran. The saved HMP output
contains `CPU#0` only, so it does not independently inspect vCPU1 registers.
The probe now validates live vCPU indexes with `query-cpus-fast` and records
them per clone. Optional register capture uses HMP `info registers -a`, checks
the reported CPU-index set, and stores registers under per-vCPU keys. These
changes have unit and TCG interface smoke coverage, but no successful
post-change KVM restore run yet. One- and two-vCPU conventional baselines and
a generic-CPU-model one-vCPU guest all stopped before guest readiness with
`KVM: entry failed, hardware error 0x0`; the underlying host cause remains
unknown.

## Reproducing measurements

Start with host preflight:

```sh
cargo run -- doctor
```

Conventional KVM baseline:

```sh
cargo run -- benchmark baseline --out /path/to/kvm-2 --count 2 --accel kvm
```

TCG baseline with normal stock events:

```sh
cargo run -- benchmark baseline --out /path/to/tcg-trace \
  --count 1 --accel tcg --trace-translations
```

For generated code bytes and durations, build the versioned QEMU patches using
[`patches/qemu/README.md`](patches/qemu/README.md), then put the custom QEMU
binary first in `PATH`. Preserve a fresh output directory for every run.

Two-clone template proof or bounded multi-vCPU probe:

```sh
cargo run -- probe --out /path/to/two-clone --count 2 --skip-guest-oom
cargo run -- probe --out /path/to/two-vcpu --count 2 --vcpus 2 --skip-guest-oom
```

Do not run high VM counts without checking host `MemAvailable`, process/cgroup
limits, and free disk. DenseVM preflight may refuse unsafe counts. A refusal is
not a failed optimization result; it is a host-capacity limit.


## RTC alarm clock-mode comparison

A two-clone QMP stop/resume experiment compared QEMU RTC `host`, `rt`, and
`vm` clocks. Running controls fired in every mode. Both alarms armed before a
seven-second stop missed in `host` and `rt`, while both `vm` alarms fired after
resume. Two sequential samples per mode repeated this result on QEMU 11.1.2 / Linux
7.2.11-arch1-1 and one synthetic guest; host/rt event loss is not attributed to a specific QEMU
or guest layer. See [`docs/rtc-clock-experiment.md`](docs/rtc-clock-experiment.md)
and the machine-readable [comparison](results/kvm-recovery-2026-10-30/rtc-clock-comparison.json).

## RAM mapping page estimates

`densevm memory-map PID RAM_FILE` now adds estimated resident, shared,
private, and private-dirty page counts derived from matching `/proc/PID/smaps`
KiB totals and the host page size. A two-process 8 MiB MAP_PRIVATE smoke
measured 1,920 shared and 128 private-dirty pages after one process changed
128 pages. This validates the reporting command, not guest COW. The
[method and caveats](docs/memory-map.md) explain why this is an aggregate
classification rather than a guest-physical page map.

The same page estimates are now attached to main probe RAM-mapping snapshots.
In a 100%-of-test-region two-clone KVM divergence run, the writer gained an
estimated 8,208 private-dirty pages and the untouched sibling gained 52.
[Raw data](results/kvm-recovery-2026-10-30/page-estimates-cow.json) and
[summary](results/kvm-recovery-2026-10-30/page-estimates-cow-summary.json).

For persistent clones, the same command can take a clone ID and state directory:
`densevm memory-map clone ID --state-dir DIR`. It reads the clone registry
without mutating it, verifies the recorded QEMU process fingerprint, and finds
the RAM path in that process's command line. A synthesized-record QEMU smoke
validated this lookup path only; it did not run a guest. See
[the method](docs/memory-map.md) and
[the smoke output](results/memory-map-live-qemu-smoke.json).

## Persistent clone memory-map lookup

A live run started two persistent clones from the same template-tree generation and sampled each using `densevm memory-map clone ID --state-dir DIR`. The command resolved separate QEMU PIDs and the common RAM backing file. Both process mappings reported 3,515 shared pages; estimated private/private-dirty pages were 1,501/1,395 for A and 260/260 for B. The clones stopped cleanly. These are `/proc/smaps` categories, not a page-by-page guest memory map or a complete host footprint. The source template was temporary, so only the measurements are retained. See [method and caveats](docs/memory-map.md) and [comparison JSON](results/kvm-recovery-2026-10-30/memory-map-persistent-comparison.json).

## Persistent template-tree RAM mapping by clone ID

The persistent-tree experiment runner now records `densevm memory-map clone ID` for each clone before divergence. It checks that each report matches the started clone ID and generation. A live two-clone run from the same generation reported 3,598 shared pages in each mapping, used separate QEMU PIDs, and stopped both clones cleanly. These are `/proc/smaps` classifications and do not identify guest-physical page numbers. The result file includes commands, measurements, and cleanup state: [JSON](results/kvm-recovery-2026-10-30/persistent-tree-memory-map-by-id.json). Reproduction steps are in [the method page](docs/memory-map.md).
