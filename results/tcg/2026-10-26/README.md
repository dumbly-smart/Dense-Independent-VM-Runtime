# Initial TCG baseline

These are first real-QEMU runs of the conventional baseline with the QEMU TCG
accelerator. The `baseline.json` files preserve the exact QEMU commands, host
metadata, PSS/RSS, startup-to-ready time, idle ticks, and active CPU samples.

| Accelerator | VM count | Ready time | Aggregate process PSS | Active CPU per process |
| --- | ---: | ---: | ---: | ---: |
| TCG | 1 | 7,728 ms | 285,434 KiB | 103% of one host core |
| TCG | 2 | 13,543 ms | 554,488 KiB | 102–103% of one host core |
| KVM | 2 | 2,281 ms | 421,467 KiB | 103–104% of one host core |

The TCG two-VM sample reached readiness about 5.9 times later than the KVM
two-VM sample. This is one run per configuration, not a statistical result.
The TCG guest used QEMU's `max` CPU model while KVM used `host`; treat the
timing as an initial path check, not a controlled CPU-model comparison. Both
active workloads consumed about one host core per QEMU process, consistent
with separate guests still executing their own work.

The untraced runs establish that the local QEMU 11.1.2 TCG path boots this
project guest and provide a basic process baseline. They do not count
translation-block events. Generated-code size and time spent translating
remain unmeasured. The KVM PSS values are QEMU process totals and do not
include KVM or host page-table memory.

An opt-in one-VM trace run is saved in [`trace1/`](trace1/). It recorded
108,834 `translate_block` events through the active-workload sample in an
8,759,535-byte trace log. Its ready time was 8,063 ms and process PSS was
288,175 KiB. The trace is instrumentation, so these timing and memory values
are not directly comparable to the untraced runs. Event count is not a count
of unique TBs and does not measure translation duration or generated-code
size.

Reproduce with:

```sh
cargo run -- benchmark baseline --count 1 --accel tcg --out /path/to/tcg-1
cargo run -- benchmark baseline --count 2 --accel tcg --out /path/to/tcg-2
cargo run -- benchmark baseline --count 2 --accel kvm --out /path/to/kvm-2
```
