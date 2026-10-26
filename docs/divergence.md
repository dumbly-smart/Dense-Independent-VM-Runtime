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
the guests answer again. Repeated samples on QEMU 11.1.2 / Linux 7.2.11 record
guest realtime, uptime, timerfd expirations, aggregate local-timer (`LOC`)
interrupts, and the RTC epoch around a pause. Realtime advanced 7–8 ms and
uptime 0–10 ms around a one-second host pause; RTC epoch advanced one second.
The post-resume reads include activity after the pause and do not reveal when
timer interrupts were delivered. RTC alarm behavior remains unmeasured. Parking
changes timing semantics. The control is explicit; there is no automatic idle
detector or packet-triggered wakeup. Raw samples:
`results/2026-10-24/parking-clock-devices-2-run1.json` and
`parking-clock-devices-2-run2.json`.

## Single-clone divergence and Alpine range measurements

The sweep above uses `--divergence-all`, so every clone dirties the requested
fraction of its test region. For the control case where only clone A diverges,
use the sequential driver:

```sh
python3 experiments/runtime-clone/sweep_driver.py \
  --densevm target/debug/densevm \
  --out /tmp/densevm-single-clone-sweep \
  --count 8
```

The driver uses 0%, 5%, 10%, 25%, 50%, and 100% divergence by default,
validates RAM and disk isolation in each raw result, and writes a machine-
readable `sweep.json` plus a `densevm stats` summary per run. A measured
QEMU 11.1.2 / Linux 7.2.11 sweep is recorded in
`results/runtime-clone/2026-10-26/eight-clone-divergence-sweep-run1/`; its
table shows clone process PSS rising from 462,997 KiB at 0% to 495,980 KiB at
100%, against per-run conventional baseline PSS values near 1.6 million KiB.
The six runs are single samples, and the entire 256 MiB template RAM file was
resident in cache. Kernel/KVM memory is not included.

A second full sweep using the same synthetic initramfs probe completed on the
same host. The clone process-PSS increase from 0% to 100% was 28,112 KiB in
that sweep, compared with 32,983 KiB in the first; clone-private memory rose
by 27,840 KiB and 33,236 KiB respectively. These two samples confirm the
direction and approximate scale of the COW cost, while also showing several
MiB of measurement variation. Per-point data and the second chart are in
`results/runtime-clone/2026-10-26/eight-clone-divergence-sweep-run2/`.

The larger Alpine range run created 20 targets plus an attacker (21 guests)
and passed guest RAM/disk isolation, guest OOM, crash, process, reboot, and
range connectivity checks. The attacker reached all targets before one target
was killed, then reached a surviving target afterward. Its evidence is in
`results/runtime-clone/2026-10-26/alpine-twenty-target-range-run1/`. That run
used a tmpfs output directory because the persistent filesystem was nearly
full; its `st_blocks` values describe tmpfs allocation and are not a
persistent-storage benchmark.
