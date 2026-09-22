# Controlled divergence experiment

`experiments/divergence/run.sh` runs the two-clone COW probe at 1, 2, 4, 8,
16, and 32 MiB of intentional RAM divergence. Each run uses a fresh output
directory and records the full `result.json`; the script also writes a compact
`divergence.json` containing PSS, private RAM mapping, and isolation fields.

Run it after building the binary:

```sh
cargo build
experiments/divergence/run.sh /tmp/densevm-divergence target/debug/densevm
```

This is a measurement harness, not a fabricated prediction. The host must
have enough memory for each sequential QEMU run, and the resulting values must
be interpreted together with the kernel/QEMU version and unmeasured KVM kernel
metadata noted in `docs/semantic-differences.md`.
