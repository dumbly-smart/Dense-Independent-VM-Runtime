# Repeated TCG and KVM baseline

These are two real-QEMU samples per configuration of the conventional baseline.
Each `baseline.json` preserves the exact QEMU commands, host metadata, PSS/RSS,
startup-to-ready time, idle ticks, and active CPU samples. The `baseline-*`
directories contain the first samples; `tcg1b`, `tcg2b`, and `kvm2b` contain
the repeats.

| Accelerator | VM count | Ready time, mean (range) | Aggregate process PSS, mean (range) | Active CPU per process, range |
| --- | ---: | ---: | ---: | ---: |
| TCG | 1 | 8,408 ms (7,728–9,087) | 284,412 KiB (283,390–285,434) | 99–103% of one host core |
| TCG | 2 | 14,509 ms (13,543–15,475) | 553,016 KiB (551,543–554,488) | 102–104% of one host core |
| KVM | 2 | 2,229 ms (2,176–2,281) | 419,144 KiB (416,820–421,467) | 99–104% of one host core |

These two samples per configuration show TCG two-VM readiness taking about
6.5 times as long as KVM two-VM readiness on average. This is a directional
observation, not a statistical result: `n=2`, the runs were sequential rather
than randomized, and TCG uses QEMU's `max` CPU model while KVM uses `host`.
Both active workloads consumed about one host core per QEMU process, consistent
with separate guests still executing their own work.

The untraced runs establish that the local QEMU 11.1.2 TCG path boots this
project guest and provide a basic process baseline. They do not count
translation-block events. Generated-code size and time spent translating
remain unmeasured. The KVM PSS values are QEMU process totals and do not
include KVM or host page-table memory.

Two opt-in one-VM trace runs are saved in [`trace1/`](trace1/) and
[`trace1b/`](trace1b/). They recorded 108,834 and 109,377 `translate_block`
events through the active-workload sample in 8,759,535-byte and 8,803,502-byte
logs. Ready time was 8,063 ms and 7,548 ms; process PSS was 288,175 KiB and
287,784 KiB. The trace is instrumentation, so these timing and memory values
are not directly comparable to the untraced runs. Event count is not a count
of unique TBs and does not measure translation duration or generated-code
size. The traces contained 92,654–92,677 distinct guest PC values; 28,410–
28,999 events fell at 12,230–12,299 PCs observed multiple times. A PC can map
to multiple TBs under different translation state, so these are PC reuse
counts, not unique TB or retranslation counts.

QEMU's `-d out_asm` output includes emitted host-code block sizes, but a manual
probe generated a 395 MB log and did not yield a guest-ready checkpoint within
the observation window. It is not treated as a result. See
[`tcg-sharing-feasibility.md`](../../../docs/tcg-sharing-feasibility.md) for
the limits and next measurement gate.

Reproduce with:

```sh
cargo run -- benchmark baseline --count 1 --accel tcg --out /path/to/tcg-1
cargo run -- benchmark baseline --count 2 --accel tcg --out /path/to/tcg-2
cargo run -- benchmark baseline --count 2 --accel kvm --out /path/to/kvm-2
```

Repeat samples were collected with the same commands and distinct output paths:

```sh
cargo run -- benchmark baseline --count 1 --accel tcg --out /path/to/tcg-1b
cargo run -- benchmark baseline --count 2 --accel tcg --out /path/to/tcg-2b
cargo run -- benchmark baseline --count 2 --accel kvm --out /path/to/kvm-2b
```
