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

- [x] Add tests for unavailable `/dev/kvm`, missing `x-ignore-shared`, and unavailable KSM counters; assert each is reported as unsupported rather than accepted.
- [x] Run the capability tests; synthetic negative cases cover unavailable features without changing host state.
- [x] Record current kernel, QEMU, qemu-img, page size, KVM, QMP capabilities, and KSM availability in result metadata.
- [x] Run `cargo test --all-targets` and save a capability report without changing global host configuration.
- [x] Commit the capability-reporting change (`c2a7ff0`).

### Task 2: Make clone identity rotation real and network-safe

**Files:**
- Modify: `experiments/cow-memory/guest-init.rs`, `src/probe.rs`, `src/qmp.rs`
- Modify: `docs/semantic-differences.md`
- Test: guest protocol and identity tests

**Interfaces:**
- Consumes: per-clone metadata and existing vmgenid/MAC assignment.
- Produces: a pre-network identity preparation protocol with explicit supported identity fields and failure status.

- [x] Add tests for distinct clone identities and guest rejection of identity/network setup before entropy preparation.
- [x] Define guest-side machine-id and OpenSSH host-key actions; DHCP remains explicitly UNKNOWN because the fixture has no DHCP client.
- [x] Implement the protocol with fresh host-kernel entropy credited/reseeded in the guest before key generation and NIC hotplug.
- [ ] **Blocked by fixture coverage:** Validate with a disposable supported Linux guest image; compare machine-id, SSH host keys, vmgenid reaction, and DHCP client identity before enabling network access. Current synthetic and prepared Alpine fixtures do not contain OpenSSH host keys or run a DHCP client, so only machine-id/hostname/MAC/vmgenid assignment is measured. The live guest protocol fails closed until reseeding and identity preparation complete.
- [x] Document image prerequisites and remaining SSH/DHCP validation gaps.

### Task 3: Re-run and extend controlled RAM divergence measurements

**Files:**
- Modify: `experiments/divergence/run.sh`, `src/probe.rs`, `src/metrics.rs`
- Create: `results/divergence/` machine-readable run outputs and summary
- Test: divergence argument and aggregation tests

**Interfaces:**
- Consumes: `--divergence-percent`, clone PSS/smaps, host metadata.
- Produces: reproducible 0/5/10/25/50/100% divergence dataset, including baseline and run environment.

- [x] Add unit/CLI coverage for percentage parsing and page-count selection; reject out-of-range values.
- [x] Run tests and verify invalid percentages fail before launching VMs.
- [x] Execute the six-point sweep under the probe's conservative memory preflight, recording PSS, RAM mapping pages, clone-ready latency, and host versions.
- [x] Generate a measured summary and SVG plot; all six points completed.
- [x] Commit harness/results separately from large runtime artifacts (`47ca274`).

### Task 4: Validate scale, multi-clone isolation, and disk lifecycle

**Files:**
- Modify: `src/probe.rs`, `src/inspect.rs`, `docs/milestone1.md`
- Create: scale/isolation result records
- Test: adversarial clone and backing-path tests

**Interfaces:**
- Consumes: clone-count probe and per-overlay inspection.
- Produces: live 2/4/8 clone measurements and an auditable isolation matrix.

- [x] Add tests for stale backing paths and unique overlay ownership.
- [x] Run them to confirm invalid chains are rejected.
- [x] Exercise RAM, disk, MAC, reboot, QEMU-process-kill, OOM, and guest-crash isolation at safe counts; crash is tested with three clones.
- [x] Save matched synthetic 2/4/8/16 clone PSS, private/shared pages, storage, clone latency, and cleanup outcomes. A 32-clone request was refused before QEMU launch by memory preflight and is recorded as unmeasured.
- [x] Update milestone acceptance status from live evidence.

### Task 5: Measure KSM as a separate optional experiment

**Files:**
- Modify: `src/ksm.rs`, `src/cli.rs`, `docs/semantic-differences.md`
- Create: KSM experiment output/summary
- Test: KSM prior-state restoration and telemetry parsing tests

**Interfaces:**
- Consumes: current KSM status/counter sampler.
- Produces: isolated template-COW versus template-COW-plus-KSM comparison with configuration provenance.

- [x] Add tests that unsupported counters and failed writes preserve the previous global KSM state.
- [x] Verify failure paths without changing host-global KSM settings.
- [ ] **Blocked by host permission:** Run only when explicitly requested and restore the exact original KSM setting on success, error, and interruption. This host reports KSM `run=0`; `/sys/kernel/mm/ksm/run` is not writable by the current user and noninteractive sudo is unavailable. Do not change host-global KSM state without administrative access.
- [ ] **Blocked by host permission:** Collect PSS, pages_shared/pages_sharing/pages_unshared/pages_volatile, ksmd CPU ticks, and a warning that counters are host-global. Read-only counters and disabled-state telemetry are recorded, but a positive merge sample requires an administrator to enable KSM.
- [x] Document side-channel and operational trade-offs and commit.

Positive KSM measurement remains blocked on this host: `/sys/kernel/mm/ksm/run`
is root-writable only and noninteractive sudo is unavailable. The current
value is `0`; no host-global setting was changed. Existing guard and failure
path tests pass, and side-channel/operational trade-offs are documented.

### Task 6: Validate idle CPU sampling and explicit parking

**Files:**
- Modify: `src/probe.rs`, `src/qmp.rs`, `docs/semantic-differences.md`
- Create: parking experiment results
- Test: state-transition and resume-after-failure tests

**Interfaces:**
- Consumes: explicit stop/resume mode and process CPU tick sampling.
- Produces: live idle-vs-parked CPU data and documented guest-visible pause semantics.

- [x] Test failed stop and failed resume states, plus child cleanup that affects only owned QEMU processes.
- [x] Run state-transition tests and verify unexpected states are surfaced.
- [x] Measure idle and explicitly parked clones over equal intervals, resume latency, guest wall-clock, and `/proc/uptime` around a pause. Periodic timer delivery and hardware RTC remain unknown.
- [x] State clearly that parking is not semantically equivalent to continuous execution; automatic detection and packet wake remain out of scope.
- [x] Commit code, data, and documentation.

### Task 7: Add an ordinary Linux guest integration and security-range topology

**Files:**
- Create: `examples/` guest provisioning and isolated network configuration
- Modify: `src/probe.rs`, `docs/design.md`, `docs/semantic-differences.md`
- Test: integration tests requiring an explicitly configured guest image

**Interfaces:**
- Consumes: identity protocol, template/clone flow, qcow2 backing model.
- Produces: documented supported distro image workflow and independent target NICs behind a controlled range network.

- [x] Specify explicit prepared guest paths and validate image metadata/hashes; the workflow does not download or alter host images implicitly.
- [x] Clone NIC attachment and range network configuration occur only after identity preparation; live range runs verify connectivity after that gate.
- [x] Boot Alpine to a stable service/login checkpoint, clone multiple targets, and verify machine identity, disk, RAM, processes, and range connectivity. SSH keys and DHCP are absent from this image and remain unverified.
- [x] Record image hashes, package/service configuration, host capabilities, and semantic unknowns.
- [x] Commit reproducible scripts/configuration and bounded result summaries.

### Task 8: Complete workload, resource, and adversarial evaluation

**Files:**
- Modify: `src/metrics.rs`, `src/probe.rs`, `docs/semantic-differences.md`
- Create: benchmark workload harnesses and results
- Test: aggregation/parser tests plus opt-in live integration tests

**Interfaces:**
- Consumes: baseline, clone, host metadata, disk and memory metrics.
- Produces: comparable measurements for idle/active CPU, I/O latency, network throughput, OOM/crash/reboot behavior, and declared unmeasured kernel overhead.

- [x] Add deterministic unit tests for metric aggregation and missing samples.
- [x] Verify unavailable values are errors rather than zero.
- [x] Run matched conventional and DenseVM workloads at safe counts; collect CPU, I/O, network, clone latency, PSS, disk allocation, OOM, crash, reboot, and process-kill results.
- [x] Add bounded OOM/crash/reboot/network adversarial checks with guest memory capped and sibling liveness checked. Device/register-level coverage remains partial.
- [x] Publish raw JSON and interpretations without treating configured RAM as physical savings.
- [x] Commit workloads, results, and semantic status updates.

### Task 9: Close project status and defer unsupported research branches honestly

**Files:**
- Modify: `README.md`, `docs/milestone1.md`, `docs/semantic-differences.md`, `docs/tcg-sharing-feasibility.md`

**Interfaces:**
- Consumes: all verified result files and test outcomes.
- Produces: status matrix separating implemented, experimentally demonstrated, unknown, and deferred work.

- [x] Cross-check project requirements against live results, automated tests, or explicit unknown/deferred status.
- [x] Mark template trees and TCG execution-code sharing deferred; no measurement currently justifies implementation.
- [x] Run formatting, the full unit/integration suite, and all three available safe KVM integration tests; list unavailable host/image prerequisites in the status documents.
- [x] Commit the final evidence-backed status.
