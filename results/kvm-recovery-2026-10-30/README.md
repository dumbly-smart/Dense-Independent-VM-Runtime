# KVM COW probe recovery

On 2026-10-30, the existing ignored two-clone isolation test passed on QEMU
11.1.2 / Linux 7.2.11-arch1-1. The same host had previously failed before
guest readiness with `KVM: entry failed, hardware error 0x0`; this successful
run establishes that the failure is no longer reproducible for this probe.
It does not explain the earlier failures or establish that every KVM workload
will run.

Run command:

```sh
cargo test --test probe two_clones_resume_and_isolate_ram_and_disk -- --ignored --nocapture
```

The raw machine-readable output is [`two-clone-kvm-isolation.json`](two-clone-kvm-isolation.json).
The probe used two 256 MiB guests and a 64 MiB qcow2 disk. It verified shared
template backing, independent clone MAC and machine IDs, private RAM writes,
private disk writes including a full-disk write, unchanged template RAM,
reboot isolation, and survival of clone B after clone A was killed.

Measured values from this one run:

| Measurement | Result |
| --- | ---: |
| Conventional two-VM aggregate QEMU PSS | 414,602 KiB |
| Clone aggregate QEMU PSS before divergence | 247,816 KiB |
| Clone aggregate QEMU PSS after A-only divergence | 280,626 KiB |
| A template-mapping private-dirty increase | 32,780 KiB |
| B template-mapping private-dirty increase | 160 KiB |
| Baseline time until both guests ready | 3,010 ms |
| Time until both clones resumed | 478 ms |
| Shared base image allocation | 200,704 bytes |
| Both clone overlay allocations after the full-disk write | 67,706,880 bytes |

These are process PSS and allocated-file measurements, not total host physical
cost. KVM metadata, kernel page tables, and cgroup memory were not measured in
this run. It is one sample with a small synthetic guest; it is not evidence
for a particular density at larger guest sizes.

The same host also passed the opt-in register capture with one vCPU per clone;
its raw result is [`one-vcpu-register-capture.json`](one-vcpu-register-capture.json).
A two-vCPU attempt first exposed a capture-gate bug: it checked CPU 0's
privilege level, while the paused output showed CPU 0 at CPL 0 and CPU 1 at
CPL 3. The corrected gate accepts a userspace vCPU and checks the expected GS
base on that same CPU. The successful result is
[`two-vcpu-register-capture.json`](two-vcpu-register-capture.json); both clones
reported CPU indexes `[0, 1]`, the guest-specific GS bases matched on CPUs 0
and 1 respectively, and the snapshots contained differing GPRs for both CPU
indexes. The initial failed attempt remains recorded as
[`two-vcpu-register-capture-initial-failure.json`](two-vcpu-register-capture-initial-failure.json)
and the follow-up diagnostic
[`two-vcpu-register-capture-diagnostic.json`](two-vcpu-register-capture-diagnostic.json).
This is one sample and still covers GPR/GS state only.

The dedicated guest LAPIC TPR mutation test also passed on this host. It used
one vCPU per clone and changed A's TPR from `0x10` to `0x11` while B remained
at `0x10`; it then changed B to `0x12` while A remained at `0x10`. Both values
were restored to `0x10`. RAM and disk isolation and sibling survival after
killing A were verified in the same run. This exercises the x2APIC TPR MSR
path only; it does not validate other APIC registers or interrupt behavior.
Raw result: [`lapic-tpr-isolation.json`](lapic-tpr-isolation.json).

The test was then tightened to require a two-vCPU guest. The old default first
failed the new topology assertion (`guest_vcpus` was 1); after adding
`--vcpus 2`, it passed and the same TPR values were observed. The guest helper
accesses `/dev/cpu/0/msr`, so this validates CPU 0's TPR inside a two-vCPU VM;
the raw result is:
[`lapic-tpr-isolation-2vcpu.json`](lapic-tpr-isolation-2vcpu.json).

The follow-up now accesses `/dev/cpu/N/msr` and uses QMP's `cpu-index` argument
to inspect each CPU's LAPIC. A two-vCPU run verified the independent TPR
sequence on CPU 0 and CPU 1; each original value was restored. The test rejects
a snapshot that does not name the requested CPU. xAPIC MMIO fallback is still
limited to CPU 0. Other APIC registers and pending interrupts remain untested.
Raw result: [`lapic-tpr-per-vcpu.json`](lapic-tpr-per-vcpu.json).

The cgroup-v2 wrapper also measured the two-VM conventional baseline and clone
phase inside a delegated memory cgroup. The raw JSON, command, summary, and
limits are recorded in [`cg/`](cg/). The temporary guest disk/RAM artifacts
were removed after collection; cgroup `memory.current` includes file-cache
charges and is not a complete host-memory inventory.

A separate cgroup run intentionally dirtied all 32 MiB of the probe region in
clone A. Its PSS and private-mapping changes were about 32 MiB while clone B
remained nearly stable; cgroup `memory.current` rose by 28,975,104 bytes. This
does not mean 100% of the 256 MiB guest RAM was dirtied. The exact table and
limits are in [`cg100/`](cg100/).

A repeated RTC alarm trial was run on the same host with:

```sh
cargo test --test probe probe_records_guest_rtc_alarm_delivery_during_running_and_paused_trials -- --ignored --nocapture
```

Both running controls received one RTC alarm interrupt (flags 160). Both
alarms armed before a seven-second QMP `stop` timed out during the 20-second
post-resume wait. After the timeout, guest RTC time was 22 seconds beyond the
programmed alarm; `/proc/driver/rtc` reported `alarm_IRQ: yes` and
`alrm_pending: no`. This repeats the previously observed behavior and still
does not locate the lost event in QEMU's timer, interrupt injection, or the
Linux guest handler. Raw output: [`rtc-alarm-pause-repeat.json`](rtc-alarm-pause-repeat.json).

The captured QEMU command does not specify `-rtc`, so QEMU's documented default
clock applies. QEMU documentation says the RTC defaults to host system time;
`clock=vm` instead stops RTC progress during suspension. This result therefore
only characterizes the default clock configuration of this probe. It does not
establish behavior under `-rtc clock=vm` or `clock=rt`.

The RTC clock comparison used one live run each with QEMU `host`, `rt`, and
`vm` RTC clocks. Running controls fired in all three modes. Both paused alarms
missed in `host` and `rt`, while both fired after resume in `vm`. Each template
and clone command contained the selected `-rtc clock=...` value. Full raw
records and reproduction details are in [`docs/rtc-clock-experiment.md`](../../docs/rtc-clock-experiment.md)
and [`rtc-clock-comparison.json`](rtc-clock-comparison.json).

A fresh two-clone 100%-of-test-region divergence run now includes shared,
private, resident, and private-dirty page estimates for each RAM mapping. A's
private-dirty estimate rose by 8,208 pages (32,832 KiB); B rose by 52 pages
(208 KiB). These are `/proc/smaps` classifications, not guest-physical page
identities. Raw result: [`page-estimates-cow.json`](page-estimates-cow.json);
extracted values: [`page-estimates-cow-summary.json`](page-estimates-cow-summary.json).

The RTC mode comparison was repeated sequentially once per mode. Running
controls again delivered in all six guest trials; both paused alarms missed
in host and `rt`, and both delivered in `vm`. Summary data includes these
second samples, with separate raw JSON records linked from
[`docs/rtc-clock-experiment.md`](../../docs/rtc-clock-experiment.md).

The persistent clone-ID memory-map command was also tested against two real
DenseVM-managed clones from one template-tree generation. Each lookup resolved
a distinct QEMU PID and the same RAM path. Both mappings reported 3,515 shared
pages; private/private-dirty estimates were 1,501/1,395 pages for `mapclone`
and 260/260 pages for `mapclone2`. Both clones were stopped cleanly. This
validates the lookup against live persistent records and records smaps sharing
classifications only; it does not establish guest-physical page identity. The
source template was temporary. See [`memory-map-persistent-comparison.json`](memory-map-persistent-comparison.json)
and its [raw A](memory-map-persistent-a.json) / [raw B](memory-map-persistent-b.json)
outputs.

The persistent-tree runner now exercises the clone-ID memory-map interface as
part of its standard two-clone workflow. On this run both reports matched the
clone IDs and generation, each showed 3,598 shared pages, and the separate
QEMU processes were stopped cleanly. The exact test output, commands, and
cleanup results are recorded in
[`persistent-tree-memory-map-by-id.json`](persistent-tree-memory-map-by-id.json).
