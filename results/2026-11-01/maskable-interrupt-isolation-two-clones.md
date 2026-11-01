# Two-clone maskable interrupt isolation probe

Date: 2026-11-01 (Asia/Kolkata)

Host: Linux 7.2.11-arch1-1, QEMU and qemu-img 11.1.3, 4 KiB pages, `/dev/kvm` accessible.

Run the ignored KVM integration test:

```sh
cargo test --test probe qmp_keyboard_interrupt_is_observed_only_by_the_target_clone -- --ignored --nocapture
```

The test passed. QMP `send-key` delivered a key event to clone A. A's guest
i8042 line in `/proc/interrupts` changed from 9 to 10; B remained at 9.

This demonstrates isolation for the tested emulated PS/2 keyboard interrupt
path in one-vCPU synthetic Linux guests. It does not inspect pending APIC
IRR/ISR state, establish exact interrupt timing, cover virtio or other device
interrupt paths, or generalize to other QEMU versions and guest distributions.
The complete machine-readable probe is in the adjacent JSON file.
