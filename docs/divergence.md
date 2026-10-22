# Controlled divergence experiment

`experiments/divergence/run.sh` runs an eight-clone COW probe at 0%, 5%, 10%,
25%, 50%, and 100% of each clone's 32 MiB guest test region. Each run uses a fresh output
directory and records the full `result.json`; the script also writes a compact
`divergence.json` containing the effective percentage and exact page count,
PSS, private RAM mapping, and isolation fields. The `--divergence-mib` option
remains available for whole-MiB experiments; `--divergence-percent` provides
the percentage sweep, including an undirtied 0% control.

Run it after building the binary:

```sh
cargo build
experiments/divergence/run.sh /tmp/densevm-divergence target/debug/densevm
# Optional third argument selects the clone count (default 8).
uv run --with matplotlib experiments/divergence/plot.py /tmp/densevm-divergence/divergence.json
```

This is a measurement harness, not a fabricated prediction. The host must
have enough memory for each sequential QEMU run, and the resulting values must
be interpreted together with the kernel/QEMU version and unmeasured KVM kernel
metadata noted in `docs/semantic-differences.md`.

`--divergence-all` fills the requested pages with independently generated guest
random bytes in every clone. Without it only A diverges. The 32 MiB region is
one eighth of configured guest RAM: 100% here does not mean 100% of the VM.
Results include snapshots of every live clone at matching before/after points.
`densevm stats RESULT_JSON` rejects missing PSS measurements and reports process
PSS separately from PSS plus the full resident template cache. The latter
double-counts mapped file pages and is explicitly an overcount, not total host
memory. Use disk-backed artifact storage for long sweeps: tmpfs artifacts also
consume host RAM after QEMU exits.

When run with `--ksm`, each result also records before/after host-global KSM
counters and `ksmd` CPU ticks. Other host workloads can affect these values.

Baseline and clone results also sample each QEMU process's `/proc/<pid>/stat`
CPU ticks over an approximately one-second idle interval. The JSON reports
raw scheduler ticks and interval duration, not a CPU percentage. This captures
time charged to the QEMU process; kernel-side KVM work is not separately
accounted here.

Pass `--park-idle` to stop every restored guest through QMP, sample QEMU
process CPU ticks while paused, explicitly resume each guest, and check that
the guests answer again. This is an experiment: guest timers stop while
paused, so it changes timing semantics. The control is explicit; there is no
automatic idle detector or packet-triggered wakeup.
