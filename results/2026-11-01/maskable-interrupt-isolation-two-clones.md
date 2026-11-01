# Two-clone maskable interrupt isolation probe

Date: 2026-11-01 (Asia/Kolkata)

Host: Linux 7.2.11-arch1-1, QEMU and qemu-img 11.1.3, 4 KiB pages, `/dev/kvm` accessible.

Run the ignored KVM integration test:

```sh
cargo test --test probe qmp_keyboard_interrupt_is_observed_only_by_the_target_clone -- --ignored --nocapture
```

The test passed. A running QMP `send-key` advanced clone A's guest i8042
`/proc/interrupts` count from 9 to 10; B stayed at 9.

The probe also paused both clones and sent a `ping` through A's serial
connection. QEMU `info lapic` and `info pic` showed no LAPIC or IOAPIC IRR in
either clone before or after the write; `info irq` counters were unchanged
while paused. After resume, A returned `PONG`. This shows the guest handled the
serial input after resume, but the probe did not observe a pending APIC/IOAPIC
interrupt while paused. Pending queue isolation remains UNKNOWN.

These measurements cover one-vCPU synthetic Linux guests on this QEMU version.
They do not establish exact interrupt timing, cover virtio or other virtual
device interrupt paths, or generalize to other QEMU versions and guest
distributions. The complete machine-readable probe is in the adjacent JSON
file.
