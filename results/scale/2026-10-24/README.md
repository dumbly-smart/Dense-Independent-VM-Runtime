# Synthetic guest clone scale

Three matched runs were collected on 2026-10-24 with QEMU 11.1.2, Linux 7.2.11, and the same 256 MiB synthetic guest. Each run used 0% explicit divergence of the 32 MiB test buffer. The raw records are `synthetic-{2,4,8}-zero-divergence.json`; `summary.json` contains the extracted measurements.

| Clones | Configured guest RAM | Conventional QEMU process PSS | Clone process PSS | Clone RAM mapping PSS | Clone ready | Overlay allocation |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 512 MiB | 406 MiB | 241 MiB | 165 MiB | 429 ms | 64.5 MiB |
| 4 | 1,024 MiB | 805 MiB | 323 MiB | 181 MiB | 999 ms | 65.2 MiB |
| 8 | 2,048 MiB | 1,584 MiB | 456 MiB | 184 MiB | 1,436 ms | 66.5 MiB |

The observations show clone process PSS increasing more slowly than the conventional baseline over these counts. With eight clones, summed clone process PSS was 28.7% of the conventional baseline process PSS. Adding the full 260 MiB resident template RAM file to clone PSS gives a deliberately conservative, overlapping user-space bound of 709 MiB (44.7% of baseline PSS). Neither comparison includes complete host physical memory; KVM metadata, host page tables, and other kernel allocations are unknown. The 32 MiB zero-divergence test only means the explicit experiment buffer was not intentionally changed; guests still perform normal work.

All three runs passed RAM/disk isolation, unique identity, sibling survival, parking, and divergence checks. The scale values are process PSS and storage allocation, not configured RAM treated as physical use. QEMU launch latency was measured as time until the requested clone count resumed and is workload/host specific.
