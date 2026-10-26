# Eight-clone 5% divergence sample

This sample used the persistent runtime path to start eight clones from the
relocated nested schema-v4 generation. All eight began with RAM and disk marker
`73`. Each clone dirtied 409 pages, equal to 5% of its 32 MiB test region, and
wrote a distinct marker byte to its own RAM and disk overlay. Every clone read
back its own marker. The run recorded before/after process PSS and template RAM
mapping counters and stopped all eight QEMU processes.

This is a single-point sample, not a density benchmark or full-guest divergence
test.
