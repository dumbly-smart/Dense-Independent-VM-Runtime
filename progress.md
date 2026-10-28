# DenseVM project status

**Status date:** 2026-10-29

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
- Two-vCPU source/template/clone restore runs passed twice. Register capture now
  requests all monitor CPUs and keeps GPRs under CPU-indexed keys; this update
  has TCG interface evidence but still needs a successful post-change KVM run.
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
  GPR blocks separately by vCPU, but post-change capture has not yet run in a
  successful KVM restore experiment. Other MSRs, detailed APIC registers,
  pending interrupts, and precise interrupt timing remain unverified.
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
  include their verified directory and manifest paths. Deletion, a
  managed catalog, and power-loss recovery remain deferred. A regression test
  now exercises the planner against a hash-verified schema-v3 generation and
  a persisted running-clone record, confirming the referenced generation is
  protected in the command's real report path.
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
new check still needs a successful live KVM run.
See the [`results/vcpu-topology-2026-10-29/`](results/vcpu-topology-2026-10-29/),
[`results/kvm-entry-2026-10-29/`](results/kvm-entry-2026-10-29/), and
[`results/vcpu-qmp-smoke-2026-10-29/`](results/vcpu-qmp-smoke-2026-10-29/)
artifacts, plus the [`results/kvm-cpu-model-smoke-2026-10-29/`](results/kvm-cpu-model-smoke-2026-10-29/) CPU-model comparison.

The optional register-state check now uses HMP `info registers -a`, checks for
exactly the configured CPU index set, and stores GPRs under keys such as
`CPU#0:RAX` and `CPU#1:RAX`. QEMU 11.1.2 returned separate blocks for both
vCPUs in a paused TCG interface smoke. This is not a substitute for a live KVM
restore and independent-state experiment.

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

