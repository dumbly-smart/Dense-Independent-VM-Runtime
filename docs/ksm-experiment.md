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
disabled (`0`), and writable by the current process. A measurement requires an
administrator to grant that process write access. The harness never invokes
`sudo` and never writes KSM sysfs itself. When preflight succeeds, run:

```sh
experiments/ksm/run.sh \
  --out /path/to/new-output-directory \
  --count 2 \
  --divergence-percent 0
```

Pass `--guest DIR` to compare using a prepared guest. Guest OOM testing is
skipped by default; `--include-guest-oom` opts into that workload. The harness
refuses to start when KSM is already enabled or the output directory exists.
It checks the `run` value between phases and after the KSM probe. Only the
DenseVM `KsmGuard` changes the sysfs value, and the harness records whether it
returned to the original setting. It reports an unexpected value and leaves
host settings alone instead of trying to rewrite them. DenseVM handles
SIGINT/SIGTERM cleanup; SIGKILL and host failure can bypass cleanup.

The output directory contains:

* `preflight.json`: host permissions, initial KSM state, and run configuration.
* `template-cow/result.json`: control probe.
* `template-cow-plus-ksm/result.json`: KSM-enabled probe.
* `comparison.json`: paired clone PSS, RAM-mapping PSS, KSM counter deltas, and
  `ksmd` CPU ticks.
* `run-status.json`: completion/failure stage and KSM restoration check.

The summary rejects pairs with different clone counts, guest RAM sizes,
divergence sizes, or divergence scopes. Missing measurements stay `null`.
KSM counters and `ksmd` CPU ticks are host-global; other processes can affect
them. The PSS values are summed QEMU process measurements and do not include
all kernel/KVM allocations. A positive `pages_shared` delta alone does not
prove those pages came from DenseVM.

The reference host cannot perform the positive run: KSM is disabled and its
`run` control is not writable by the current user. The refusal is saved in
`results/2026-10-24/ksm-comparison-blocked/`; no host setting was changed.
