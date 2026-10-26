# Persistent clones from a relocated nested template

This run used the third generation in a relocated schema-v4 template tree:
`tree-reloc-run21`. Its parent RAM and disk markers were both `73`. DenseVM
validated the manifest and started two independent persistent QEMU clones from
it using the ordinary `clone start` lifecycle command.

Both guests initially read `RAM 73` and `DISK 73`. Clone A then wrote `91` to
its test RAM region and virtual disk. A read back `91` from both; B continued
to read `73` from both. The clones had distinct MAC addresses and machine IDs,
and both normal stops returned after their QEMU PIDs disappeared.

The JSON file includes the manifest integrity result, command outputs, guest
reads, process PSS, and `/proc/<pid>/smaps` counters for the actual template RAM
file mapping. At the recorded snapshot, A had 34,112 KiB `Private_Dirty` in
that mapping versus 920 KiB for B. These process-level readings are not a
complete host memory total; kernel/KVM memory is excluded.

This demonstrates runtime cloning from a relocated, nested v4 generation and
guest-visible RAM/disk write isolation for this synthetic guest and QEMU
configuration. It does not establish other machine types, network backends, or
distribution behavior.
