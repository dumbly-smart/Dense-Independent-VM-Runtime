# Paired KSM experiment

`experiments/ksm/run.sh` runs the same DenseVM probe twice: first with template
copy-on-write only, then with QEMU KSM eligibility enabled while Linux KSM is
temporarily active. The default experiment uses two clones, zero requested
divergence, and skips the guest OOM workload to keep the comparison bounded.
Both runs use the same clone count, guest, divergence percentage, and workload
options.

Build DenseVM and run the read-only preflight:

```sh
cargo build
experiments/ksm/run.sh --check-only
```

The preflight reports whether `/sys/kernel/mm/ksm/run` is readable, initially
disabled (`0`), and writable by the current process. If it is not writable,
the explicit `--sudo-ksm` option uses `sudo` only for the KSM `run` switch;
DenseVM and its QEMU children remain under the invoking user. The script asks
for sudo credentials before beginning the control run, then uses noninteractive
`sudo tee` to enable KSM for the second probe and restore the original value.
Without that option, the process itself must be allowed to write the control.
The read-only `--check-only` command does not authenticate sudo; its `ready`
status means the requested write mode and prerequisites are available, not
that sudo credentials have been verified. The actual run verifies credentials
before starting either probe.

When using sudo for only the sysfs writes, run:

```sh
experiments/ksm/run.sh \
  --out /path/to/new-output-directory \
  --count 2 \
  --divergence-percent 0 \
  --sudo-ksm
```

If the current user already has write access, omit `--sudo-ksm`. The regular
invocation is:

```sh
experiments/ksm/run.sh \
  --out /path/to/new-output-directory \
  --count 2 \
  --divergence-percent 0
```

Pass `--guest DIR` to compare using a prepared guest. Guest OOM testing is
skipped by default; `--include-guest-oom` opts into that workload. The harness
refuses to start when KSM is already enabled or the output directory exists.
It checks the `run` value between phases and after the KSM probe. In direct
mode, DenseVM's `KsmGuard` enables KSM and restores it. In `--sudo-ksm` mode,
the harness toggles only the `run` control while DenseVM observes the already
enabled state; an exit trap attempts to restore the original `0` after errors,
INT, or TERM. `run-status.json` records the final observed value and whether it
matches the initial state. SIGKILL, power loss, or a sudo policy that prevents
the final write can bypass or defeat cleanup; inspect the recorded status and
host setting after any interrupted run.

The output directory contains:

* `preflight.json`: host permissions, initial KSM state, and run configuration.
  `ksm_write_mode` records whether the harness uses direct writes or the
  explicitly requested sudo-only control path.
* `template-cow/result.json`: control probe.
* `template-cow-plus-ksm/result.json`: KSM-enabled probe.
* `comparison.json`: paired clone PSS, RAM-mapping PSS, KSM counter deltas, and
  `ksmd` CPU ticks.
* `run-status.json`: completion/failure stage and KSM restoration check.

The summary rejects pairs with different clone counts, guest RAM sizes,
divergence sizes or scopes, guest image provenance, QEMU/kernel versions, or
host page-size/KVM configuration. It also verifies that the control result
observed KSM `run=0` and the KSM result observed `run=1`; canonical numeric
strings from sysfs are normalized to integers. Missing measurements stay
`null`.
KSM counters and `ksmd` CPU ticks are host-global; other processes can affect
them. The PSS values are summed QEMU process measurements and do not include
all kernel/KVM allocations. A positive `pages_shared` delta alone does not
prove those pages came from DenseVM.

The reference host cannot perform the positive run: KSM is disabled and its
`run` control is not writable by the current user; noninteractive sudo is also
unavailable. The refusal is saved in
`results/2026-10-24/ksm-comparison-blocked/`; a fresh permission check is saved
at `results/2026-11-14/ksm-permission-check.json`. No host setting was changed.
