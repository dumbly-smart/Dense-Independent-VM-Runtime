# Synthetic guest clone scale

Four matched runs were collected on 2026-10-24 with QEMU 11.1.2, Linux 7.2.11, and the same 256 MiB synthetic guest. Each run used 0% explicit divergence of the 32 MiB test buffer. The raw records are `synthetic-{2,4,8,16}-zero-divergence.json`; `summary.json` contains the extracted measurements.

The requested one-VM conventional baseline is recorded separately in `conventional-baseline-1.json`: it reached ready in 763 ms with 216,824 KiB process PSS, 229,252 KiB RSS, and 200,704 bytes allocated disk. There is no one-clone templated VM run because the restore probe requires at least two independent clones.

| Clones | Configured guest RAM | Conventional QEMU process PSS | Clone process PSS | Clone RAM mapping PSS | Clone ready | Overlay allocation |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 512 MiB | 406 MiB | 241 MiB | 165 MiB | 429 ms | 64.5 MiB |
| 4 | 1,024 MiB | 805 MiB | 323 MiB | 181 MiB | 999 ms | 65.2 MiB |
| 8 | 2,048 MiB | 1,584 MiB | 456 MiB | 184 MiB | 1,436 ms | 66.5 MiB |
| 16 | 4,096 MiB | 3,165 MiB | 734 MiB | 202 MiB | 4,558 ms | 69.0 MiB |

The observations show clone process PSS increasing more slowly than the conventional baseline over these counts. With 16 clones, summed clone process PSS was 23.2% of the conventional baseline process PSS. Adding the full 254 MiB resident template RAM file to clone PSS gives a deliberately conservative, overlapping user-space bound of 988 MiB (31.2% of baseline PSS). Neither comparison includes complete host physical memory; KVM metadata, host page tables, and other kernel allocations are unknown. The 32 MiB zero-divergence test only means the explicit experiment buffer was not intentionally changed; guests still perform normal work.

A 32-clone request was refused before creating an output directory or launching QEMU: the probe required 8,912,896 KiB available and found 6,814,724 KiB. See `preflight-refusal-32.json`; 32-clone support is not claimed as measured.

All four matched runs passed RAM/disk isolation, unique identity, sibling survival, parking, and divergence checks. The separate one-VM baseline reached ready successfully. The scale values are process PSS and storage allocation, not configured RAM treated as physical use. QEMU launch latency was measured as time until the requested clone count resumed and is workload/host specific.
