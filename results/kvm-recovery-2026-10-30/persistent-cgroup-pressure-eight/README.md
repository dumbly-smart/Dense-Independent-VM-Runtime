# Eight-clone cgroup pressure telemetry run

This live persistent-clone run used QEMU 11.1.2 / Linux 7.2.11-arch1-1, eight guests with 256 MiB configured RAM each, and a transient user systemd cgroup limited to 2 GiB. The seed/template preparation and persistent clones ran in that same scope. Every clone dirtied 100% of its 32 MiB probe region.

| Stage | `memory.current` | cgroup-lifetime `memory.peak` |
| --- | ---: | ---: |
| Before clone creation | 646,848,512 bytes | 817,307,648 bytes |
| After clone startup | 948,772,864 bytes | 952,688,640 bytes |
| After divergence | 1,224,478,720 bytes | 1,224,916,992 bytes |
| After clone 1 stopped | 1,151,705,088 bytes | 1,226,051,584 bytes |

The `memory.current` increase from clone startup to post-divergence was 275,705,856 bytes. The summary's startup-to-divergence delta for every `memory.events` counter was zero, including `max`, `oom`, and `oom_kill`. Peak is a cgroup-lifetime high-water counter, so the stage rows repeat a cumulative value rather than measuring separate stage maxima.

Clone 1 stopped and its QEMU PID exited. All seven remaining clones returned their own RAM and disk markers; cleanup reported 0 leftover QEMU processes. The 2 GiB scope did not reach its limit in this run. This is one host sample and does not show total host headroom or guarantee all KVM/kernel memory was charged to this cgroup.

For reproduction, follow the full-scope setup in [`docs/cgroup-memory-accounting.md`](../../../docs/cgroup-memory-accounting.md), then replace the sweep step with `persistent_tree.py --count 8 --region-mib 32 --divergence-percent 100 --divergence-mode all-clones`. Keep the seed, template, and clones in the same scope and use a fresh short output path.

Raw stage data: [`result.json`](result.json). Derived cgroup-stage summary: [`cgroup-summary.json`](cgroup-summary.json).
