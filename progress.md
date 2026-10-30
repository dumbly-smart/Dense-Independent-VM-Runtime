# DenseVM project status

**Status date:** 2026-10-30

**Working branch:** `research/remaining-runtime`  
**Detailed research diary:** [`docs/progress.md`](docs/progress.md)  
**Evidence index:** [`reports.md`](reports.md)  
**Semantic status:** [`docs/semantic-differences.md`](docs/semantic-differences.md)

## Goal

Determine how much RAM and storage a collection of independent QEMU/KVM guests
can share when they start from a common frozen Linux state, while retaining
private CPU/device state, guest-visible identity, RAM writes, disk writes, and
process lifetimes. The runtime is a research prototype; results are scoped to
the tested QEMU, kernel, guest, devices, and workload.

## Demonstrated

- Separate QEMU processes restore from a frozen RAM/device state and run as
  independent guests.
- Unchanged file-backed template RAM is shared through private writable
  mappings; a guest write creates private pages. Sibling RAM and disk markers
  remain unchanged in recorded isolation checks.
- Each guest has a qcow2 overlay over an immutable base; disk writes stay in
  the guest overlay.
- Unique MAC, machine-id, hostname, vmgenid, entropy preparation, and tested
  Alpine OpenSSH/DHCP client identity flows are implemented and measured.
- Two-vCPU source/template/clone restore runs passed twice. The all-vCPU
  register capture was exercised on KVM: both clone processes reported indexes
  `[0, 1]` and separate GPR snapshots, with the guest workload's user GS base
  observed on its active CPU. This samples GPR/GS state only; other MSRs,
  detailed APIC state, pending interrupts, and precise interrupt timing remain
  unverified. See `docs/vcpu-register-state.md` and the 2026-10-30 recovery data.
- Persistent clone start/stop/reconcile/delete, schema-v4 relocated template
  lineage, and guarded staging recovery have evidence-backed implementations.
- Conventional KVM and TCG baselines, memory/disk/CPU/network workloads,
  bounded OOM/crash/reboot/process-kill checks, and range experiments have
  saved JSON and raw evidence.
- TCG traces now split boot/readiness, idle, and active-workload phases; patched
  and stock-QEMU behavior has been checked against raw log ranges.

## Partial, blocked, or deferred

- **Positive KSM comparison: blocked by host permission.** KSM is host-global;
  `/sys/kernel/mm/ksm/run` is not writable by the current account. The paired
  harness and read-only preflight exist, but no positive merge result is
  claimed.
- **Whole-host memory cost: partial.** Process PSS and selected cgroup-v2
  accounting are measured. Some KVM allocations, kernel page tables, and
  allocations outside the cgroup remain outside the reported totals.
- **CPU/device semantics: partial.** Previous KVM samples captured vCPU0 GPR/GS
  and selected LAPIC TPR state. The probe now parses all `info registers -a`
  GPR blocks separately by vCPU, and a two-clone KVM restore succeeded on
  2026-10-30. One- and two-vCPU register capture passed; both vCPU indexes had
  differing GPR snapshots and the guest-specific user GS bases were observed
  on CPU 0 and CPU 1 respectively. Other MSRs, detailed APIC registers,
  pending interrupts, and precise interrupt timing remain unverified.
- A fresh QEMU 11.1.2 / Linux 7.2.11-arch1-1 run of the ignored LAPIC TPR
  isolation test passed on 2026-10-30. Clone A changed TPR from `0x10` to
  `0x11` while B stayed at `0x10`; B then changed to `0x12` while A read
  `0x10`; both restored to `0x10`. RAM/disk isolation and B's survival after
  A's process kill also passed. This is a third TPR sample, not evidence for
  other APIC registers or interrupt-delivery semantics. Raw result:
  [`lapic-tpr-isolation.json`](results/kvm-recovery-2026-10-30/lapic-tpr-isolation.json).
- The TPR test now runs with two vCPUs per guest and asserts the reported
  topology. It writes guest CPU 0's x2APIC TPR: the same isolation sequence
  passed. The test first failed at the
  new assertion with the old one-vCPU launch, then passed after adding
  `--vcpus 2`. Raw result:
  [`lapic-tpr-isolation-2vcpu.json`](results/kvm-recovery-2026-10-30/lapic-tpr-isolation-2vcpu.json).
- The LAPIC probe now targets a selected guest CPU through `/dev/cpu/N/msr`
  and QMP `human-monitor-command`'s local `cpu-index` argument. On QEMU 11.1.2,
  the two-clone isolation sequence passed independently on vCPU 0 and vCPU 1;
  each clone's original TPR was restored. The first attempt showed that HMP's
  `cpu 1` selection does not persist across QMP command calls, so snapshots
  verify that QEMU reports the requested CPU index. xAPIC MMIO fallback remains
  limited to vCPU 0. Other APIC state and pending interrupts remain untested.
  Raw result: [`lapic-tpr-per-vcpu.json`](results/kvm-recovery-2026-10-30/lapic-tpr-per-vcpu.json).
- The recovery run now has a delegated cgroup-v2 measurement alongside PSS:
  conventional `memory.current` was 416,010,240 bytes versus 356,093,952 bytes
  for clones before divergence. Baseline/clone `sec_pagetables` charges were
  405,504/409,600 bytes. This includes cache charges and remains partial
  accounting. See [`cgroup sample`](results/kvm-recovery-2026-10-30/cg/).
- In the matched 100%-of-probe-region divergence run, clone A's template
  mapping `Private_Dirty` rose 32,864 KiB and PSS rose 32,813 KiB; B's
  corresponding changes were 16 KiB and 181 KiB. Cgroup `memory.current`
  increased 28,975,104 bytes, with `sec_pagetables` up 45,056 bytes. This is
  32 MiB of one guest's RAM, not 100% of the guest allocation. See
  [`divergence cgroup sample`](results/kvm-recovery-2026-10-30/cg100/).
- **Timers while parked: partial.** QMP pause nearly eliminates host CPU use,
  but guest time and alarms differ. Parking is not semantically equivalent to
  continuous execution.
- **Distribution coverage: partial.** Alpine/BusyBox profiles were tested;
  other distributions' entropy, machine-id, DHCP, SSH, and service identity
  behavior remain unknown.
- **Hierarchical templates: experimental.** Materialization, validation, and
  relocation proofs exist, and `densevm validate template-tree ROOT` now
  verifies all discovered generation artifacts and reports duplicate IDs.
  `densevm template gc-plan ROOT --state-dir DIR [--keep ID]` provides a
  read-only reachability plan; it blocks on unverified or ambiguous catalogs
  and protects active clone generations and their ancestors. Candidate entries
  include their verified directory and manifest paths; active clone records
  include lifecycle state. A blocked plan prints its report and exits nonzero.
  Deletion, a managed catalog, and power-loss recovery remain deferred. A regression test
  now exercises the planner against a hash-verified schema-v3 generation and
  persisted clone records in all four lifecycle states. Starting/running
  generations are protected; stopped/failed records do not pin unknown
  generations. The active references are confirmed in the planner report.
- **TCG code sharing: deferred.** Translation is measured, but sharing mutable
  QEMU translation blocks across processes is not justified by the current
  safety analysis. This does not affect the KVM COW design.
- **32-VM capacity: not demonstrated.** Safe preflight rejected that count on
  the measured host. Do not extrapolate from configured guest RAM.

## Latest focused increment

`densevm probe --vcpus 1..4` now applies one vCPU topology consistently to
conventional baseline VMs, the frozen source, and incoming clones. The memory
preflight includes extra vCPU overhead. Two two-vCPU runs passed restore,
RAM/disk isolation, unique identities, and sibling survival. See
[`results/vcpu-multi2/`](results/vcpu-multi2/) and
[`results/vcpu-multi2-run2/`](results/vcpu-multi2-run2/).

The probe now also queries `query-cpus-fast` after every clone restore, requires
the expected contiguous vCPU index set, and writes those indexes to each
clone's result. Unit tests pass, and a two-vCPU TCG smoke test confirmed the
installed QEMU 11.1.2 reply shape. The attempted post-change KVM run failed in
the first baseline with `KVM: entry failed, hardware error 0x0`, before
template creation. Separate one-vCPU runs failed with the same error under
both `-cpu host` and `-cpu qemu64`; the underlying host cause is unknown. The
at that time, the new check still needed a successful live KVM run.
See the [`results/vcpu-topology-2026-10-29/`](results/vcpu-topology-2026-10-29/),
[`results/kvm-entry-2026-10-29/`](results/kvm-entry-2026-10-29/), and
[`results/vcpu-qmp-smoke-2026-10-29/`](results/vcpu-qmp-smoke-2026-10-29/)
artifacts, plus the [`results/kvm-cpu-model-smoke-2026-10-29/`](results/kvm-cpu-model-smoke-2026-10-29/) CPU-model comparison.

On 2026-10-30, the existing two-clone KVM isolation probe and the dedicated
LAPIC TPR mutation/isolation probe succeeded on the same reported QEMU/Linux
versions. They verified clone resume, RAM/disk isolation, identity separation,
reboot and process-kill survival; separate live register captures also passed
for one and two vCPUs. This shows guest entry and these targeted state probes
work on this host; it does not explain the earlier hardware error or establish
complete CPU/device semantics. Raw measurements:
[`results/kvm-recovery-2026-10-30/`](results/kvm-recovery-2026-10-30/).

The optional register-state check uses HMP `info registers -a`, checks for
exactly the configured CPU index set, and stores GPRs under keys such as
`CPU#0:RAX` and `CPU#1:RAX`. Live KVM evidence for both vCPU indexes is now
recorded above; the QMP/TCG smoke remains interface validation only.

Probe, baseline, and persistent clone-start commands now use one shared check
for QMP and serial socket path lengths (plus optional login sockets in the
probe) before enabling KSM or creating guest/disk artifacts. An overlong probe
output path returned the measured 161-byte path, the Linux limit of 107 bytes,
and created no output directory.

## Completion criterion

The prototype's main COW and isolation mechanisms are implemented and
experimentally demonstrated. The research question is not “complete” as a
universal claim: KSM, whole-host memory overhead, broader guest/device
semantics, and long-run scale require additional hosts, permissions, or test
profiles. Keep those gaps visible rather than treating them as code defects or
inventing measurements.

For a complete beginner-to-technical explanation, read
[`explanation.md`](explanation.md). For the system map, read
[`architecture.md`](architecture.md).

- 2026-10-30 RTC alarm repeat: the running controls again delivered alarms, and both paused trials again missed. The output now records the same late RTC time and no pending alarm; the QEMU command uses the documented default host clock because it supplies no `-rtc` option. Root cause remains unknown, and `clock=vm` / `clock=rt` are unmeasured. See `results/kvm-recovery-2026-10-30/rtc-alarm-pause-repeat.json`.
- RTC clock comparison: the probe now accepts documented QEMU `host`, `rt`, or `vm` clock selection only for `--rtc-alarm` runs, applies it consistently to the conventional baseline/template/restored clones, and records it in JSON. On the tested QEMU 11.1.2 / Linux 7.2.11 host, running control alarms fired for all three settings; alarms during QMP stop missed with host/rt and fired after resume with vm. One sample per mode only; see `docs/rtc-clock-experiment.md` and `results/kvm-recovery-2026-10-30/rtc-clock-comparison.json`.
- Memory-map estimates: the existing `/proc/PID/smaps` command now derives rounded page counts using `getconf PAGESIZE`. A controlled two-process MAP_PRIVATE smoke reports the expected 1,920 shared and 128 private-dirty pages out of an 8 MiB resident mapping. The output explicitly distinguishes smaps classifications from exact guest-physical page ownership. See `docs/memory-map.md` and `results/memory-map-smoke.json`.
- COW page estimates now appear in QEMU probe snapshots as well as `densevm memory-map`. In a fresh two-clone 32 MiB-region divergence run, A gained an estimated 8,208 private-dirty pages and B gained 52. The existing `densevm stats` summary still accepts the expanded record. See `docs/memory-map.md` and the two `page-estimates-cow*` results.
- `densevm memory-map clone ID --state-dir DIR` now resolves persistent clone metadata to a live QEMU PID and RAM file, with process fingerprint checks around the sample. A live QEMU lookup smoke used a synthesized registry record and paused QEMU; it verifies command-path resolution only. See `docs/memory-map.md` and `results/memory-map-live-qemu-smoke.json`.
- RTC clock-mode experiment now has two sequential samples per mode with identical results on this host; host and `rt` missed after pause, `vm` delivered after resume. Cross-version and guest-profile behavior remains unmeasured. Raw repeats are saved beside the first three mode records in `results/kvm-recovery-2026-10-30/`.

- Persistent clone memory-map command now has a live test using two actual DenseVM-managed clones from one template generation. Each resolved to its own QEMU PID and the same RAM file; `/proc/smaps` classified 3,515 pages as shared in each process. Private/private-dirty estimates were 1,501/1,395 pages in A and 260/260 in B. Both clones were stopped cleanly. This confirms persistent registry integration and reported sharing classifications, not exact page identity or host-wide physical savings. See `docs/memory-map.md` and `results/kvm-recovery-2026-10-30/memory-map-persistent-comparison.json`.

- The persistent template-tree runner now captures RAM-map reports by clone ID and verifies the reported identity/generation against each started clone. A live two-clone rerun reported 3,598 smaps-shared pages per process from the common RAM file; both processes were distinct and stopped cleanly. The runner's Python tests and all Rust tests pass. Reproduction commands and limitations: `docs/memory-map.md`; raw result: `results/kvm-recovery-2026-10-30/persistent-tree-memory-map-by-id.json`.
