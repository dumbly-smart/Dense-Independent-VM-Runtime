# vCPU register state sampling

`densevm probe` includes an opt-in integration test that checks whether two
restored QEMU/KVM clones expose independent general-purpose register state.
Each clone runs a different seeded arithmetic workload and reports that the
workload is active. The host then pauses each QEMU process through QMP and asks
the QEMU HMP command `info registers` for the saved x86-64 CPU state. Both
clones resume and must finish their workloads.

The experiment reads QEMU's reported register state; it does not inspect KVM
kernel structures. The current QEMU machine has one vCPU per guest. A successful
saved sample showed different values in 15 of the 16 reported general-purpose
registers. This is evidence that the measured clones retain distinct
sampled vCPU state while paused, and that sampling/resume did not prevent the
workloads or ordinary RAM/disk isolation checks from completing.

The sample does not verify every part of architectural CPU state. Control
registers, MSRs, APIC registers, pending interrupts, and multi-vCPU behavior
remain unmeasured. QEMU's HMP output format can vary by version and architecture;
the parser accepts x86-64 general-purpose registers only. Do not treat one pair
of distinct snapshots as exhaustive proof of all CPU-state isolation.

Raw result: [`results/2026-10-24/vcpu-register-state-2.json`](../results/2026-10-24/vcpu-register-state-2.json).
