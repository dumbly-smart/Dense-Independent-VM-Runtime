# Persistent all-clone divergence sweep, sample 1

This is the first eight-clone sample of the persistent runtime RAM-divergence
sweep. It ran all six points from 0% through 100%; every guest's RAM and disk
readback matched its own expected value, and each clone stopped cleanly. The
machine-readable `sweep.json` links the raw result for each point.

The repeated run, SVG graph, summarized values, procedure, and measurement
limits are documented in
[`persistent-divergence-sweep-run2`](../persistent-divergence-sweep-run2/README.md).
