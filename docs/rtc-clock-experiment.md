# RTC alarm behavior across QMP stop/resume

## Question

When a guest's emulated RTC alarm expires while QEMU is stopped with QMP,
does the guest receive the alarm after QEMU resumes? Does QEMU's selected RTC
clock change that outcome?

This probes one timing-related guest-visible behavior. It does not establish
general timer fidelity, nor does it isolate a lost event to QEMU timer
processing, interrupt injection, or the guest RTC handler.

## Mechanism

The opt-in command is:

```sh
densevm probe --out /path/to/run --skip-guest-oom --rtc-alarm --rtc-clock host
```

`--rtc-clock` accepts QEMU's `host`, `rt`, and `vm` values and is only valid
with `--rtc-alarm`. If omitted, QEMU's default is preserved. When set, the
selected `-rtc clock=...` argument is applied to the conventional baseline,
template, and every restored clone. The result records the requested clock and
the actual QEMU argument vectors.

The guest arms an alarm five seconds in the future while running, waits for
the event, then arms another alarm and is stopped via QMP for seven seconds.
After resuming, the guest waits up to 22 seconds. Both clones are tested in
each phase. The alarm event requires `RTC_AF` in the guest RTC event flags.

QEMU's manual says the RTC defaults to host system time, `rt` uses a host
monotonic clock where available, and `vm` uses virtual time that does not
advance during suspension. See the official [QEMU 11.1.2 `-rtc` documentation](https://qemu.readthedocs.io/en/v11.1.2/system/invocation.html#rtc-clock).

## Result

One run per mode was collected on QEMU 11.1.2 / Linux 7.2.11-arch1-1. In every
mode, the running control alarms fired once in both clones with flags 160
(`RTC_IRQF | RTC_AF`).

| QEMU RTC clock | Running controls | Alarms after 7-second QMP stop | Result |
| --- | --- | --- | --- |
| `host` | A and B delivered | A and B missed | RTC was past the programmed alarm; `alarm_IRQ: yes`, `alrm_pending: no` |
| `rt` | A and B delivered | A and B missed | RTC was past the programmed alarm; `alarm_IRQ: yes`, `alrm_pending: no` |
| `vm` | A and B delivered | A and B delivered | One event per clone with `RTC_AF` |

The virtual-clock result is consistent with pausing RTC progress during QMP
stop: the alarm remains ahead of guest RTC time and fires after resume. The
host and monotonic clock results reproduce the missed-alarm behavior. This is
an observation from one machine and one trial per mode, not a general claim
that QEMU loses RTC alarms whenever it is paused. The exact loss point for the
host and monotonic trials remains unknown.

## Reproduction and raw data

Run each command separately, using a fresh short output path:

```sh
cargo run -- probe --out /tmp/rtc-host --skip-guest-oom --rtc-alarm --rtc-clock host --divergence-percent 0
cargo run -- probe --out /tmp/rtc-rt --skip-guest-oom --rtc-alarm --rtc-clock rt --divergence-percent 0
cargo run -- probe --out /tmp/rtc-vm --skip-guest-oom --rtc-alarm --rtc-clock vm --divergence-percent 0
```

The captured raw QEMU command for the template and both clones must contain
the requested `-rtc clock=...` value. Results from this host are in
[`rtc-clock-comparison.json`](../results/kvm-recovery-2026-10-30/rtc-clock-comparison.json),
with the full per-run results beside it:
[`host`](../results/kvm-recovery-2026-10-30/rtc-clock-host.json),
[`rt`](../results/kvm-recovery-2026-10-30/rtc-clock-rt.json), and
[`vm`](../results/kvm-recovery-2026-10-30/rtc-clock-vm.json).

The ignored live integration test
`probe_records_guest_rtc_alarm_delivery_with_virtual_clock` checks the
template/clone command configuration and alarm report on a host with working
QEMU/KVM.
