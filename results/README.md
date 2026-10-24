# Saved results

`probe-2026-09-22-qemu-11.1.1.json` is the raw output of the two-clone probe
on the inspected host. It includes the two ordinary VM baseline, QMP migration
statuses, whole-process and RAM-mapping `smaps` measurements, template RAM
file cache residency, disk allocations, MAC and vmgenid values, and isolation
checks. PIDs and timing are run-specific. Large RAM and disk artifacts remain
in the probe output directory outside the repository; the JSON is enough to
audit the recorded measurements but not to restart those exact guests.

Current-host capabilities are recorded in
[`2026-10-24/host-capabilities.json`](2026-10-24/host-capabilities.json). The
2026-10-24 run directory contains CPU, disk latency, network, parking, OOM, and
identity samples. The matched synthetic clone-count comparison is in
[`scale/2026-10-24/`](scale/2026-10-24/), and the controlled 8-clone divergence
curve and plot are in [`divergence/2026-10-24/`](divergence/2026-10-24/).
Guest kernel crash evidence is in [`isolation/2026-10-24/`](isolation/2026-10-24/).
All process-PSS comparisons exclude kernel/KVM memory unless stated otherwise.
