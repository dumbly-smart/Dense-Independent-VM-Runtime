# vCPU register state sampling

`densevm probe` includes an opt-in integration test that checks whether two
restored QEMU/KVM clones expose independent general-purpose register state.
Each clone runs a different seeded arithmetic workload and reports that the
workload is active. The host then pauses each QEMU process through QMP and asks
the QEMU HMP command `info registers` for the saved x86-64 CPU state. Both
clones resume and must finish their workloads.

The experiment reads QEMU's reported register state; it does not inspect KVM
kernel structures. The current QEMU machine has one vCPU per guest. The saved
sample showed different values in 6 of the 16 reported general-purpose
registers. CR0, CR2, CR3, CR4, and EFER were also reported and matched in this
pair. The different GPR values show distinct sampled vCPU state while paused;
matching control-register values do not establish whether independent writes
would remain isolated. Sampling/resume did not prevent the workloads or
ordinary RAM/disk isolation checks from completing.

The sample does not verify every part of architectural CPU state. Independent
control-register or MSR writes, APIC registers, pending interrupts, and
multi-vCPU behavior remain unmeasured. The result preserves the full HMP output
for inspection, while the parser compares x86-64 general-purpose registers
only. QEMU's output format can vary by version and architecture. Do not treat
one pair of distinct snapshots as exhaustive proof of all CPU-state isolation.

Raw result: [`results/2026-10-24/vcpu-register-state-2.json`](../results/2026-10-24/vcpu-register-state-2.json).
