# KVM COW probe recovery

On 2026-10-30, the existing ignored two-clone isolation test passed on QEMU
11.1.2 / Linux 7.2.11-arch1-1. The same host had previously failed before
guest readiness with `KVM: entry failed, hardware error 0x0`; this successful
run establishes that the failure is no longer reproducible for this probe.
It does not explain the earlier failures or establish that every KVM workload
will run.

Run command:

```sh
cargo test --test probe two_clones_resume_and_isolate_ram_and_disk -- --ignored --nocapture
```

The raw machine-readable output is [`two-clone-kvm-isolation.json`](two-clone-kvm-isolation.json).
The probe used two 256 MiB guests and a 64 MiB qcow2 disk. It verified shared
template backing, independent clone MAC and machine IDs, private RAM writes,
private disk writes including a full-disk write, unchanged template RAM,
reboot isolation, and survival of clone B after clone A was killed.

Measured values from this one run:

| Measurement | Result |
| --- | ---: |
| Conventional two-VM aggregate QEMU PSS | 414,602 KiB |
| Clone aggregate QEMU PSS before divergence | 247,816 KiB |
| Clone aggregate QEMU PSS after A-only divergence | 280,626 KiB |
| A template-mapping private-dirty increase | 32,780 KiB |
| B template-mapping private-dirty increase | 160 KiB |
| Baseline time until both guests ready | 3,010 ms |
| Time until both clones resumed | 478 ms |
| Shared base image allocation | 200,704 bytes |
| Both clone overlay allocations after the full-disk write | 67,706,880 bytes |

These are process PSS and allocated-file measurements, not total host physical
cost. KVM metadata, kernel page tables, and cgroup memory were not measured in
this run. It is one sample with a small synthetic guest; it is not evidence
for a particular density at larger guest sizes.

The same host also passed the opt-in register capture with one vCPU per clone;
its raw result is [`one-vcpu-register-capture.json`](one-vcpu-register-capture.json).
A two-vCPU attempt first exposed a capture-gate bug: it checked CPU 0's
privilege level, while the paused output showed CPU 0 at CPL 0 and CPU 1 at
CPL 3. The corrected gate accepts a userspace vCPU and checks the expected GS
base on that same CPU. The successful result is
[`two-vcpu-register-capture.json`](two-vcpu-register-capture.json); both clones
reported CPU indexes `[0, 1]`, the guest-specific GS bases matched on CPUs 0
and 1 respectively, and the snapshots contained differing GPRs for both CPU
indexes. The initial failed attempt remains recorded as
[`two-vcpu-register-capture-initial-failure.json`](two-vcpu-register-capture-initial-failure.json)
and the follow-up diagnostic
[`two-vcpu-register-capture-diagnostic.json`](two-vcpu-register-capture-diagnostic.json).
This is one sample and still covers GPR/GS state only.
