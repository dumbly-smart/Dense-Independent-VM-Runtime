# Eight-clone persistent RAM divergence sweep

The [`persistent_sweep.py`](../../../../experiments/runtime-clone/persistent_sweep.py)
driver ran two samples of each divergence point with eight persistent QEMU
clones from one relocated nested schema-v4 template. Each guest had 256 MiB
configured RAM; the test region was 32 MiB per guest. For each nonzero point,
all eight guests dirtied the requested fraction of that region using distinct
byte values. A one-byte disk marker was also written to each target overlay;
disk writes are checked for isolation but do not scale with the RAM percentage.

Every point verified independent RAM and disk reads, eight unique clone
identities, unchanged template artifact hashes, and eight clean stops. The raw
index is `sweep.json`; each row links its raw result. The [graph](divergence.svg)
shows process PSS before and after each run and the measured template-RAM mapping
`Private_Dirty` increase. These are QEMU process measurements; KVM/kernel
memory outside those mappings is excluded.

| RAM divergence per clone | Pages dirtied | Mean process PSS increase | Mean template mapping `Private_Dirty` increase |
| ---: | ---: | ---: | ---: |
| 0% | 0 | 0.3 MiB | 0.3 MiB |
| 5% | 409 | 15.2 MiB | 13.4 MiB |
| 10% | 819 | 28.1 MiB | 26.2 MiB |
| 25% | 2,048 | 66.6 MiB | 64.7 MiB |
| 50% | 4,096 | 130.5 MiB | 128.9 MiB |
| 100% | 8,192 | 259.4 MiB | 257.2 MiB |

At 100%, eight guests each dirtied 32 MiB, or 256 MiB total. The mapping
private-dirty increase was about 257 MiB. At 0%, the roughly 0.3 MiB change is
measurement and runtime noise. The two samples track closely at every point,
but two runs are not enough for a statistical performance claim.

The experiment measures the cost of controlled RAM divergence. It does not
measure total host physical memory, test a full 256 MiB divergence per guest,
or establish a VM density limit. `PSS` at the 100% point was about 556 MiB for
the eight QEMU processes; this figure must not be read as total host memory.
