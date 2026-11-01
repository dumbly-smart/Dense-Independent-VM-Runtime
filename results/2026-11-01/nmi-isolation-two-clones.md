# Two-clone NMI isolation probe

Date: 2026-11-01 (Asia/Kolkata)

Host: Linux 7.2.11-arch1-1, QEMU and qemu-img 11.1.3, 4 KiB pages, `/dev/kvm` accessible.

Run the ignored KVM integration test:

```sh
cargo test --test probe qmp_nmi_delivery_is_observed_only_by_the_target_clone -- --ignored --nocapture
```

The test passed. QMP `inject-nmi` returned successfully for clone A. A's guest
`/proc/interrupts` NMI total changed from 0 to 1; B's total stayed at 0. The
raw `result.json` contains the full probe and host metadata.

This demonstrates one QEMU process's NMI injection was observed by its target
guest and not the sibling guest in this setup. It does not test queued maskable
interrupts, per-vCPU APIC IRR/ISR state, exact interrupt timing, other QEMU
versions, or other guests/distributions. The run used one vCPU per clone.
