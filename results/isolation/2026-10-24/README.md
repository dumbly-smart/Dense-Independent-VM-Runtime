# Guest kernel crash isolation

`kernel-crash-3.json` records a live three-clone synthetic KVM run on QEMU 11.1.2 / Linux 7.2.11. The probe enabled SysRq in one clone and triggered the guest kernel crash action. It observed the kernel panic text on that clone's serial console, verified its QEMU process and QMP remained available, and confirmed the other clones remained responsive. The run also passed its RAM, disk, and identity checks.

Reproduce with:

```sh
cargo run -- probe --count 3 --divergence-percent 0 --out /tmp/densevm-crash-3
```

This verifies guest kernel crash isolation for the synthetic guest and this host kernel. It does not test recovery, ordinary distribution crash dumps, or application behavior after a panic.
