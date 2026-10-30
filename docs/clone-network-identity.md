# Persistent clone network identity ordering

## Why the order matters

QEMU migration restores device state as well as CPU state. A template NIC can
therefore restore its saved MAC after DenseVM supplied a different MAC on the
QEMU command line. The existing probe demonstrated this behavior: changing the
command-line MAC alone did not change the migrated guest NIC.

A clone also receives fresh kernel entropy and a new machine ID only after its
guest agent starts. If the restored NIC were already connected, a guest could
send DHCP or other traffic while it still had the template's identity.

## Clone startup sequence

Persistent clone startup follows this order:

1. Generate a host-random machine ID, locally administered unicast MAC, and
   per-clone entropy payload. Entropy payload bytes are not written to the
   registry or result metadata.
2. Build QEMU with `-S`, the private RAM mapping, a private qcow2 overlay, the
   template's restricted user-network backend, and its hotpluggable PCIe root
   port. Reject a template command that contains a saved NIC.
3. Restore the migration state while the clone is paused. Verify QMP reports
   the paused state and that `query-rx-filter` reports no NIC after migration.
   Then resume the guest without a virtual network adapter.
4. Send fresh entropy to the guest, force its kernel CRNG reseed, set and read
   back its machine ID and hostname, and rotate existing OpenSSH host keys when
   the guest agent supports them.
5. Use QMP `device_add` to attach a new `virtio-net-pci` NIC to the template's
   PCIe root port and restricted network backend, with the new MAC.
6. Check that QMP reports exactly one adapter with that MAC and that the guest
   reads the same address from `/sys/class/net/eth0/address`. Only then mark
   the clone `running`.

Any failed check makes clone startup fail and triggers cleanup of that clone's
QEMU process. QMP's [`device_add` command](https://qemu.readthedocs.io/en/master/interop/qemu-qmp-ref.html)
is used to create the independent adapter after the migrated machine state is
loaded.

## Template requirements and limits

The template must have one `-netdev` user backend with an explicit ID and a
`pcie-root-port` device with an explicit ID. The saved migration state must not
contain a NIC. DenseVM rejects a saved NIC rather than trying to change its
MAC after migration. The runtime currently adds one `virtio-net-pci` adapter;
other NIC models, backends, and PCI topologies are unsupported by this path.

The network backend is QEMU user networking with `restrict=on`; this provides
per-QEMU isolation for the supported runtime path. Range experiments that use
a shared virtual segment have their own separately configured NIC/hotplug
sequence.

This ordering proves the tested guest had no virtual NIC before its identity
was prepared and had the generated MAC after hotplug. It does not prove that
every distribution's DHCP client derives a unique client identifier from the
new machine ID. DHCP option 61 remains verified only for the tested BusyBox
`udhcpc` profile. Arbitrary userspace RNG caches, application identity caches,
and SSH implementations remain unknown.

## Live evidence

On QEMU 11.1.2 and Linux 7.2.11, a generated schema-v4 template and two
persistent clones completed this sequence. Both clone starts succeeded, both
guest-visible MACs differed, both machine IDs differed, their memory maps
resolved to the same template RAM file, and both stopped cleanly. The run
reported 2,307.88 ms to start both clones. Raw command and measurement data:
[`persistent-nic-identity-gating.json`](../results/kvm-recovery-2026-10-30/persistent-nic-identity-gating.json).

A subsequent eight-clone run using the same startup path verified eight
distinct MACs and machine IDs. Clone 1 dirtied all of the 32 MiB test region;
its QEMU PSS rose 32,718 KiB while sibling changes were small measurement
noise. After isolation checks, stopping clone 1 left all seven siblings
responsive with their own RAM and disk markers. Starting all eight took
9,236.98 ms. Aggregate QEMU-process PSS was 305,209 KiB before divergence and
338,986 KiB afterward; this excludes KVM/kernel memory and has no matched
conventional baseline, so it is not a host-wide density claim. The 2 GiB
configured total is eight guests configured with 256 MiB each, not measured
physical use. Raw run: [`persistent-eight-sibling-stop.json`](../results/kvm-recovery-2026-10-30/persistent-eight-sibling-stop.json).
