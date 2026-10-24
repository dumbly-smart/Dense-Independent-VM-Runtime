# vCPU register state sampling

`densevm probe` includes an opt-in integration test that checks whether two
restored QEMU/KVM clones expose independent general-purpose register state.
Each clone runs a different seeded arithmetic workload and reports that the
workload is active. The host then pauses each QEMU process through QMP and asks
the QEMU HMP command `info registers` for the saved x86-64 CPU state. Both
clones resume and must finish their workloads.

The experiment reads QEMU's reported register state; it does not inspect KVM
kernel structures. The current QEMU machine has one vCPU per guest. Each guest
sets a different user GS base (`0x11110000` or `0x22220000`) in its workload
thread. QEMU reported both values exactly in the paused snapshots. Four of the
16 reported general-purpose registers also differed. CR0, CR2, CR3, CR4, and
EFER were reported and matched in this pair. The distinct GS bases and GPR
values show sampled per-vCPU state; matching control-register values do not
establish whether independent writes to them would remain isolated.
Sampling/resume did not prevent the workloads or ordinary RAM/disk isolation
checks from completing.

The sample does not verify every part of architectural CPU state. Independent
control-register writes, other MSR writes, APIC registers, pending interrupts,
and multi-vCPU behavior remain unmeasured. The result preserves the full HMP
output for inspection, while the structured parser compares x86-64
general-purpose registers and the GS base. QEMU's output format can vary by
version and architecture. Do not treat one pair of distinct snapshots as
exhaustive proof of all CPU-state isolation.

The opt-in register probe also requests QEMU HMP `info lapic` for each paused
clone. Two repeated live two-clone runs captured LAPIC state from both guests;
within each run, the dumps were identical and both reported `TPR 0x10`. This
confirms that QEMU returned consistent LAPIC snapshots for both paused clones,
but does not prove that independently changing APIC state in one clone leaves
the other unchanged. APIC register and pending-interrupt independence remain
**unverified**; the next experiment needs controlled per-clone APIC/interrupt
divergence.

Raw results: [`results/2026-10-24/vcpu-register-state-2.json`](../results/2026-10-24/vcpu-register-state-2.json),
[`results/2026-10-24/vcpu-register-state-lapic-2.json`](../results/2026-10-24/vcpu-register-state-lapic-2.json),
and [`results/2026-10-24/vcpu-register-state-lapic-2-run2.json`](../results/2026-10-24/vcpu-register-state-lapic-2-run2.json).
