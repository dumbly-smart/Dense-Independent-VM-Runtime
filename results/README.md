# Saved results

`probe-2026-09-22-qemu-11.1.1.json` is the raw output of the two-clone probe
on the inspected host. It includes the two ordinary VM baseline, QMP migration
statuses, whole-process and RAM-mapping `smaps` measurements, template RAM
file cache residency, disk allocations, MAC and vmgenid values, and isolation
checks. PIDs and timing are run-specific. Large RAM and disk artifacts remain
in the probe output directory outside the repository; the JSON is enough to
audit the recorded measurements but not to restart those exact guests.
