# Eight persistent clones from a nested template

The repeatable [`persistent_tree.py`](../../../../experiments/runtime-clone/persistent_tree.py)
driver started eight independent QEMU processes from the relocated schema-v4
leaf generation. Each guest was configured with 256 MiB, for 2 GiB total
configured guest RAM. All eight initially read the parent RAM and disk markers
as `73`. Clone 001 then wrote `91`; every other guest continued to read `73`
from both RAM and disk. All eight MAC addresses and machine IDs were unique,
the base template RAM/disk hashes stayed unchanged, and all eight stop commands
returned after their QEMU PIDs disappeared.

## Measurements

| Measurement | Before A's writes | After A's writes |
| --- | ---: | ---: |
| Aggregate QEMU process PSS | 314,013 KiB | 347,220 KiB |
| Aggregate process `Private_Dirty` | 272,252 KiB | 305,540 KiB |
| Aggregate template RAM mapping PSS | 43,238 KiB | 76,194 KiB |
| Aggregate template mapping `Private_Dirty` | 10,240 KiB | 43,276 KiB |

Clone 001's process PSS increased by 32,471 KiB and its `Private_Dirty` by
32,876 KiB. Other clones changed by tens to a few hundred KiB during the same
interval. The guest write dirtied the synthetic guest's 32 MiB test region.
PSS and `smaps` are per-process kernel readings; these numbers exclude KVM
allocations not mapped into the QEMU processes and are not total host physical
memory accounting. This is one scale sample, not a density limit.

Eight sequential starts took about 5.1 seconds in the initial scale run; this
repeat also records per-clone startup and total time in `result.json`. Timing
includes the clone's restore and guest identity preparation.
