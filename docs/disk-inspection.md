# Inspecting a persistent clone's disk

The path form accepts any qcow2 image and reports the QEMU image metadata and
backing chain:

```sh
densevm inspect /var/lib/densevm/vm01/disk.qcow2 disk
```

For a persistent clone, use its registry ID instead:

```sh
densevm inspect clone vm01 disk --state-dir /var/lib/densevm
```

The command opens the clone registry without modifying it, resolves the
recorded overlay, and requires that it be the regular `disk.qcow2` file inside
that clone's own instance directory. It reports the clone ID, generation,
lifecycle state, overlay path, and `qemu-img info --backing-chain` output. The
backing chain helps check that sibling overlays derive from the same base.

`actual-size` describes each image file in the returned chain. Do not add every
clone's full chain together as though all backing images were private: shared
backing files appear repeatedly. Compare the overlay's own `actual-size` and
measure filesystem allocation separately. `virtual-size` is guest-visible
capacity, not host storage consumed.

When a guest is running, its QEMU process may update qcow2 metadata while
`qemu-img info` samples the image; the returned metadata is not an atomic
snapshot. Prefer stopped clones for repeatable measurements. This command
inspects image metadata and does not prove guest disk-write isolation; use the
controlled COW experiments for that.

A 2026-10-30 sample of two stopped persistent clones resolved distinct overlay
paths from the same template generation. Each overlay reported 200,704 bytes
`actual-size` against a 64 MiB `virtual-size`, and both backing chains named
the same parent overlay and base. This is one metadata sample, not a write
divergence benchmark. Raw JSON: [`persistent-clone-disk-inspect.json`](../results/kvm-recovery-2026-10-30/persistent-clone-disk-inspect.json).
