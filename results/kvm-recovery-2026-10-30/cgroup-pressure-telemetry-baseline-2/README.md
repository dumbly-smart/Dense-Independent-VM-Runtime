# Cgroup pressure telemetry smoke

A conventional two-VM KVM baseline was run inside a transient user systemd scope with `MemoryAccounting=yes` and `MemoryMax=3G` on QEMU 11.1.2 / Linux 7.2.11-arch1-1. DenseVM captured the cgroup sample when both guests reached readiness.

At that point `memory.current` and cgroup-lifetime `memory.peak` were both 427,380,736 bytes. `memory.events` reported zero for `max`, `oom`, and `oom_kill` (all counters in the raw file were zero). The configured limit was 3,221,225,472 bytes. The raw record includes QEMU PSS and `memory.stat` as separate, overlapping accounting views.

This run validates that the optional peak and event fields are read from a live cgroup. It is not a matched template-clone comparison and does not establish a host-wide no-pressure claim. See [`docs/cgroup-memory-accounting.md`](../../../docs/cgroup-memory-accounting.md).
