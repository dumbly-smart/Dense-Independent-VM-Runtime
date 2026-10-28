# vCPU register state sampling

`densevm probe` includes an opt-in integration test that checks whether two
restored QEMU/KVM clones expose independent general-purpose register state.
Each clone runs a different seeded arithmetic workload and reports that the
workload is active. The host then pauses each QEMU process through QMP and asks
the QEMU HMP command `info registers` for the saved x86-64 CPU state. Both
clones resume and must finish their workloads.

The experiment reads QEMU's reported register state; it does not inspect KVM
kernel structures. The default machine has one vCPU per guest; the probe now
accepts `--vcpus 1..4` and applies that count to conventional baselines, the
frozen template, and restored clones. Existing GS-base and GPR sampling still
captures only QEMU's selected `CPU#0`. Each guest sets a different user GS base
(`0x11110000` or `0x22220000`) in its workload thread. QEMU reported both
values exactly in the paused snapshots. Four of the 16 reported
general-purpose registers also differed. CR0, CR2, CR3, CR4, and EFER were
reported and matched in this pair. The distinct GS bases and GPR values show
sampled per-vCPU state; matching control-register values do not establish
whether independent writes to them would remain isolated.
Sampling/resume did not prevent the workloads or ordinary RAM/disk isolation
checks from completing.

Two repeated two-vCPU KVM runs saved and restored a template into two clones
with `-smp 2` on every process. Both runs passed RAM/disk isolation, unique
identity, vCPU0 register sampling, and sibling-survival checks. The raw records
are `results/vcpu-multi2/result.json` and
`results/vcpu-multi2-run2/result.json`. QEMU's HMP output contained only
`CPU#0`, so this proves the multi-vCPU restore path ran, not independent
register state for vCPU1. The sample does not verify every part of architectural
CPU state. Independent control-register writes, other MSR writes, APIC
registers beyond TPR, pending interrupts, and direct vCPU1 state remain
unmeasured. The result preserves
the full HMP output for inspection, while the structured parser compares
x86-64 general-purpose registers and the GS base. QEMU's output format can
vary by version and architecture. Do not treat these sampled states as
exhaustive proof of all CPU-state isolation.

The opt-in register probe also requests QEMU HMP `info lapic` for each paused
clone. Two repeated live two-clone runs captured LAPIC state from both guests;
within each run, the dumps were identical and both reported `TPR 0x10`. This
confirms that QEMU returned consistent LAPIC snapshots for both paused clones,
but alone does not prove independent APIC state.

## LAPIC task-priority register divergence

The opt-in `--lapic-tpr-state` experiment writes each guest's LAPIC task
priority register and captures it with QEMU `info lapic`. It tries the guest
x2APIC TPR MSR first and falls back to xAPIC MMIO. Both successful runs used
the MSR path. Each guest began at TPR `0x10`. DenseVM set clone A's TPR to
`0x11`; QMP then reported A at `0x11` and B still at `0x10`. It next set B to
`0x12`; QMP reported B at `0x12`, while A had returned to `0x10` during the
intervening guest execution. This is consistent with Linux rewriting TPR as
the guest runs. It did not show B's value appearing in A. Both clones were
then restored to `0x10`, verified through QMP. The transitions repeated in
two runs; RAM/disk isolation and sibling survival passed in each.

The experiment changes only TPR's subpriority bits. Intel documents TPR bits
7:4 as the task-priority class and bits 3:0 as the sub-class; the sub-class
does not control interrupt delivery. This keeps the priority class unchanged
while giving each guest a distinct observable TPR value ([Intel SDM, APIC
TPR/PPR description](https://cdrdv2-public.intel.com/874240/325462-090-sdm-vol-1-2abcd-3abcd-4.pdf)).
These results demonstrate independent TPR writes for the tested one-vCPU
QEMU/KVM configuration. They do not establish isolation for other LAPIC
registers, pending interrupts, other MSRs, or multi-vCPU machines.

An earlier `/dev/mem`-only attempt read back the value from its mapping but
QEMU continued to report TPR `0x10`; that raw failed observation is retained
separately. A first task-priority-class attempt also timed out on the guest
serial console, so the final experiment changes only subpriority bits.

Raw results: [`results/2026-10-24/vcpu-register-state-2.json`](../results/2026-10-24/vcpu-register-state-2.json),
[`results/2026-10-24/vcpu-register-state-lapic-2.json`](../results/2026-10-24/vcpu-register-state-lapic-2.json),
[`results/2026-10-24/vcpu-register-state-lapic-2-run2.json`](../results/2026-10-24/vcpu-register-state-lapic-2-run2.json),
[`results/2026-10-24/lapic-tpr-divergence-2.json`](../results/2026-10-24/lapic-tpr-divergence-2.json),
[`results/2026-10-24/lapic-tpr-divergence-2-run2.json`](../results/2026-10-24/lapic-tpr-divergence-2-run2.json),
and [`results/2026-10-24/lapic-tpr-divergence-mmio-noeffect-2.json`](../results/2026-10-24/lapic-tpr-divergence-mmio-noeffect-2.json).
