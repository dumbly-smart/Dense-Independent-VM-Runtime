# DenseVM Remaining Research Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the DenseVM prototype's remaining correctness, realistic-guest, and reproducible-evaluation work without claiming unmeasured semantics or savings.

**Architecture:** Keep one QEMU/KVM process per guest and preserve the current immutable shared-RAM plus per-clone qcow2-overlay model. Extend the probe and guest tooling in independently testable slices, separating host capability reporting, synthetic mechanism tests, and distribution-specific integration. Every optimization remains optional until its correctness and cost are measured.

**Tech Stack:** Rust, QEMU/QMP, KVM, Linux procfs/sysfs, qcow2, shell experiment harnesses, JSON results, Markdown research records.

**Spec:** `docs/design.md`, `docs/milestone1.md`, `docs/semantic-differences.md`, and the original DenseVM project requirements.

## Global Constraints

- Each clone remains a separate QEMU process with independent vCPU and device state.
- The template RAM and base disk are immutable while clones exist; each clone has a private RAM mapping and qcow2 overlay.
- Never alter global KSM settings without preserving and restoring the prior value.
- Never infer physical savings from configured RAM or RSS alone; report PSS and disclose unmeasured kernel overhead.
- Any unverified semantic property remains `UNKNOWN` or `PARTIAL`.
- Experiments must preflight host memory and stop before uncontrolled OOM risk.
- Guest identity must be unique before clones share a network; deterministic probe tokens are not cryptographic entropy.

## Review Focus

- Missing or incompatible QEMU/KVM/KSM features should yield actionable capability errors, not partial-success claims.
- Failed restore or guest probe must clean up only the QEMU processes/artifacts created by that run.
- Disk overlays must not silently depend on a moved or replaced backing path.
- A clone crash, reboot, memory pressure event, or disk write must not mutate sibling state.
- Results from global KSM counters or old QEMU/kernel versions must not be attributed solely to DenseVM.

---

### Task 1: Refresh host capability and reproducibility record

**Files:**
- Modify: `src/inspect.rs`, `src/cli.rs`
- Modify: `docs/design.md`, `results/README.md`
- Test: existing Rust unit/integration tests

**Interfaces:**
- Consumes: existing QEMU/QMP and host inspection functions.
- Produces: explicit machine-readable host capability/version metadata used by every experiment result.

- [ ] Add tests for unavailable `/dev/kvm`, missing `x-ignore-shared`, and unavailable KSM counters; assert each is reported as unsupported rather than accepted.
- [ ] Run those tests and confirm they fail before implementation.
- [ ] Record current kernel, QEMU, qemu-img, page size, KVM, QMP capabilities, and KSM availability in result metadata.
- [ ] Run `cargo test` and save a capability report without changing global host configuration.
- [ ] Commit the capability-reporting change.

### Task 2: Make clone identity rotation real and network-safe

**Files:**
- Modify: `experiments/cow-memory/guest-init.rs`, `src/probe.rs`, `src/qmp.rs`
- Modify: `docs/semantic-differences.md`
- Test: guest protocol and identity tests

**Interfaces:**
- Consumes: per-clone metadata and existing vmgenid/MAC assignment.
- Produces: a pre-network identity preparation protocol with explicit supported identity fields and failure status.

- [ ] Add tests proving cloned identity requests differ and network attachment is rejected until preparation succeeds.
- [ ] Define guest-side identity actions for machine-id and host keys where present; unsupported DHCP/RNG operations must report `UNKNOWN`, not fake success.
- [ ] Implement the protocol and ensure entropy comes from a documented guest entropy source rather than the deterministic test token.
- [ ] Validate with a disposable supported Linux guest image; compare machine-id, SSH host keys, vmgenid reaction, and DHCP client identity before enabling network access.
- [ ] Document exact guest/image prerequisites and commit.

### Task 3: Re-run and extend controlled RAM divergence measurements

**Files:**
- Modify: `experiments/divergence/run.sh`, `src/probe.rs`, `src/metrics.rs`
- Create: `results/divergence/` machine-readable run outputs and summary
- Test: divergence argument and aggregation tests

**Interfaces:**
- Consumes: `--divergence-percent`, clone PSS/smaps, host metadata.
- Produces: reproducible 0/5/10/25/50/100% divergence dataset, including baseline and run environment.

- [ ] Add unit tests for percent-to-page rounding and reject out-of-range values.
- [ ] Run the test suite and verify invalid percentages fail before launching VMs.
- [ ] Execute the sweep under a conservative memory preflight on the current host, recording PSS, private/shared RAM mapping pages, latency, and host versions.
- [ ] Generate a summary/plot from recorded data only and mark failed or unavailable points explicitly.
- [ ] Commit harness and results separately from any generated large artifacts.

### Task 4: Validate scale, multi-clone isolation, and disk lifecycle

**Files:**
- Modify: `src/probe.rs`, `src/inspect.rs`, `docs/milestone1.md`
- Create: scale/isolation result records
- Test: adversarial clone and backing-path tests

**Interfaces:**
- Consumes: clone-count probe and per-overlay inspection.
- Produces: live 2/4/8 clone measurements and an auditable isolation matrix.

- [ ] Add tests for stale backing paths and unique overlay ownership.
- [ ] Run them to confirm current failure modes are caught.
- [ ] Exercise RAM, disk, MAC, reboot, QEMU-process-kill, and guest-crash isolation across all clones at safe counts.
- [ ] Save 2/4/8 clone PSS, private/shared pages, storage, clone latency, and cleanup outcomes; do not imply 16/32 support was measured unless run.
- [ ] Update milestone acceptance statuses from live evidence and commit.

### Task 5: Measure KSM as a separate optional experiment

**Files:**
- Modify: `src/ksm.rs`, `src/cli.rs`, `docs/semantic-differences.md`
- Create: KSM experiment output/summary
- Test: KSM prior-state restoration and telemetry parsing tests

**Interfaces:**
- Consumes: current KSM status/counter sampler.
- Produces: isolated template-COW versus template-COW-plus-KSM comparison with configuration provenance.

- [ ] Add tests that unsupported counters and failed writes preserve the previous global KSM state.
- [ ] Verify failures before any live experiment.
- [ ] Run only when explicitly requested and restore the exact original KSM setting on success, error, and interruption.
- [ ] Collect PSS, pages_shared/pages_sharing/pages_unshared/pages_volatile, ksmd CPU ticks, and a warning that counters are host-global.
- [ ] Document side-channel and operational trade-offs and commit.

### Task 6: Validate idle CPU sampling and explicit parking

**Files:**
- Modify: `src/probe.rs`, `src/qmp.rs`, `docs/semantic-differences.md`
- Create: parking experiment results
- Test: state-transition and resume-after-failure tests

**Interfaces:**
- Consumes: explicit stop/resume mode and process CPU tick sampling.
- Produces: live idle-vs-parked CPU data and documented guest-visible pause semantics.

- [ ] Test state transitions, including a failed stop and failed resume, ensuring cleanup attempts to resume or terminate only owned VMs.
- [ ] Run tests and verify unexpected states are surfaced.
- [ ] Measure idle and explicitly parked clones over equal intervals, plus resume latency and guest clock/timer behavior.
- [ ] State clearly that parking is not semantically equivalent to continuous execution; automatic detection and packet wake remain out of scope until separately validated.
- [ ] Commit code, data, and documentation.

### Task 7: Add an ordinary Linux guest integration and security-range topology

**Files:**
- Create: `examples/` guest provisioning and isolated network configuration
- Modify: `src/probe.rs`, `docs/design.md`, `docs/semantic-differences.md`
- Test: integration tests requiring an explicitly configured guest image

**Interfaces:**
- Consumes: identity protocol, template/clone flow, qcow2 backing model.
- Produces: documented supported distro image workflow and independent target NICs behind a controlled range network.

- [ ] Specify guest image inputs and reject absent/untrusted image paths without downloading or modifying host images implicitly.
- [ ] Test that targets cannot share a network until identity preparation succeeds.
- [ ] Boot a distribution guest to a stable service/login checkpoint, clone multiple targets, and verify machine identity, keys, disk, RAM, processes, and network isolation.
- [ ] Record image hashes, package/service configuration, host capabilities, and all semantic unknowns.
- [ ] Commit only reproducible scripts/configuration and bounded result summaries.

### Task 8: Complete workload, resource, and adversarial evaluation

**Files:**
- Modify: `src/metrics.rs`, `src/probe.rs`, `docs/semantic-differences.md`
- Create: benchmark workload harnesses and results
- Test: aggregation/parser tests plus opt-in live integration tests

**Interfaces:**
- Consumes: baseline, clone, host metadata, disk and memory metrics.
- Produces: comparable measurements for idle/active CPU, I/O latency, network throughput, OOM/crash/reboot behavior, and declared unmeasured kernel overhead.

- [ ] Add deterministic unit tests for metric aggregation and missing samples.
- [ ] Verify the tests distinguish unavailable values from zero.
- [ ] Run conventional and DenseVM cases with matched guest count/configuration and safe host limits; collect active workload CPU, I/O, network, clone latency, PSS, disk allocation, and failure outcomes.
- [ ] Add bounded OOM/crash/reboot/device/network adversarial tests, ensuring host memory is capped and sibling liveness/state is checked after each action.
- [ ] Publish raw JSON and a concise interpretation; never report configured RAM as physical savings.
- [ ] Commit harness, results, and semantic table updates.

### Task 9: Close project status and defer unsupported research branches honestly

**Files:**
- Modify: `README.md`, `docs/milestone1.md`, `docs/semantic-differences.md`, `docs/tcg-sharing-feasibility.md`

**Interfaces:**
- Consumes: all verified result files and test outcomes.
- Produces: status matrix separating implemented, experimentally demonstrated, unknown, and deferred work.

- [ ] Cross-check every project requirement against a live result, automated test, or explicit deferral.
- [ ] Mark template trees and TCG execution-code sharing deferred unless measurements justify implementation.
- [ ] Run formatting, unit tests, and all available safe integration tests; list unavailable host/image prerequisites.
- [ ] Commit the final evidence-backed status.
