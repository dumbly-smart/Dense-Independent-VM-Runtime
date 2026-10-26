# Persistent nested-template clone run 3

This third run used the automated driver
[`persistent_tree.py`](../../../../experiments/runtime-clone/persistent_tree.py)
with the relocated schema-v4 leaf generation. The driver validates the
manifest, starts two independent persistent QEMU clones, checks inherited and
diverged guest markers, records process and template-RAM mapping metrics,
checks that the template RAM/disk hashes did not change, then stops each clone.

Run it with:

```sh
python3 experiments/runtime-clone/persistent_tree.py \
  --manifest /path/to/relocated/tree-reloc-run21/generation.json \
  --out results/runtime-clone/RUN-NAME
```

This run passed: both guests initially read RAM and disk marker `73`; after A
wrote `91`, only A read `91`. Both generated MACs and machine IDs were unique,
the template artifact hashes stayed unchanged, and both stop commands returned
after their QEMU PIDs disappeared. A's template mapping showed 34,132 KiB
`Private_Dirty`; B showed 976 KiB. Process PSS was 89,692 KiB for A and 55,150
KiB for B at the snapshot. These values exclude kernel/KVM memory and are one
short experiment sample, not a full host-memory accounting result.

Run 2 and the earlier manual run are recorded alongside this one. Across all
three, the inherited state, A-only RAM/disk divergence, unique identity, and
normal-stop checks passed.
