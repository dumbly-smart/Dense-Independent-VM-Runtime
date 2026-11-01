# Two-clone IA32_TSC_AUX isolation probe

Date: 2026-11-01 (Asia/Kolkata)

Host: Linux 7.2.11-arch1-1, QEMU and qemu-img 11.1.3, 4 KiB pages, `/dev/kvm` accessible.

Run the ignored two-vCPU KVM test:

```sh
cargo test --test probe guest_tsc_aux_writes_are_private_for_every_vcpu -- --ignored --nocapture
```

The test passed on vCPU 0 and vCPU 1. For each vCPU, clone A wrote a tagged
value and clone B retained its original; B then wrote another tag and A
retained its value. Both clones restored their original TSC_AUX values (0 on
vCPU 0 and 1 on vCPU 1).

This demonstrates independent writes and readback for IA32_TSC_AUX
(`0xc0000103`) in this two-vCPU synthetic Linux configuration. It does not
establish behavior for other MSRs, other CPU models, or arbitrary guests. The
complete machine-readable probe is in the adjacent JSON file.
