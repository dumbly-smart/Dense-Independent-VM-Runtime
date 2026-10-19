# Controlled divergence experiment

`experiments/divergence/run.sh` runs the two-clone COW probe at 0%, 5%, 10%,
25%, 50%, and 100% of the 32 MiB guest test region. Each run uses a fresh output
directory and records the full `result.json`; the script also writes a compact
`divergence.json` containing the effective percentage and exact page count,
PSS, private RAM mapping, and isolation fields. The `--divergence-mib` option
remains available for whole-MiB experiments; `--divergence-percent` provides
the percentage sweep, including an undirtied 0% control.

Run it after building the binary:

```sh
cargo build
experiments/divergence/run.sh /tmp/densevm-divergence target/debug/densevm
```

This is a measurement harness, not a fabricated prediction. The host must
have enough memory for each sequential QEMU run, and the resulting values must
be interpreted together with the kernel/QEMU version and unmeasured KVM kernel
metadata noted in `docs/semantic-differences.md`.

When run with `--ksm`, each result also records before/after host-global KSM
counters and `ksmd` CPU ticks. Other host workloads can affect these values.

Baseline and clone results also sample each QEMU process's `/proc/<pid>/stat`
CPU ticks over an approximately one-second idle interval. The JSON reports
raw scheduler ticks and interval duration, not a CPU percentage. This captures
time charged to the QEMU process; kernel-side KVM work is not separately
accounted here.
