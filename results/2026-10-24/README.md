# Six-guest range run

Run on 2026-10-24 with QEMU 11.1.2 and Linux 7.2.11, using the locally prepared Alpine 3.24.2 ext4 fixture. Reproduce with:

```sh
cargo run -- probe --guest /path/to/prepared-alpine --range --count 6 --divergence-all --park-idle --out /tmp/densevm-range
```

The full machine-readable measurement is in [`security-range-6.json`](security-range-6.json). The experiment launched six conventional baseline guests and six templated clones; each guest had 256 MiB configured RAM. The range used one UDP client and five targets on a loopback-bound QEMU multicast socket network.

| Measurement | Result |
| --- | ---: |
| Baseline: six QEMU processes, summed PSS | 1,214,875 KiB |
| Baseline time until six guests ready | 5,299 ms |
| Templated clones: summed PSS before test divergence | 272,865 KiB |
| Templated clones: summed PSS after each clone dirtied its 32 MiB test region | 469,747 KiB |
| Clone RAM mapping PSS, before → after divergence | 59,435 → 256,193 KiB |
| Shared base disk allocated | 12,128,256 bytes |
| Six overlay files allocated | 70,082,560 bytes total |

The initial clone process PSS was about 22.5% of the six-process conventional baseline PSS for this fixture and measurement point. After the private RAM writes it was about 38.7%. These are process-PSS comparisons, not complete host physical-memory totals: QEMU PSS omits KVM metadata, host page tables, and other kernel allocations. The 32 MiB divergence region is not the full 256 MiB guest RAM.

The run verified RAM and disk isolation, whole-test-disk isolation, unique guest/MAC/vmgenid identities, isolated process state, per-clone parking and resume, guest reboot of target A, survival of its sibling, and client connectivity to all five targets both before and after killing target T1's QEMU process. `KSM run` remained 0; this run measures template COW without KSM.

[`security-range-6-network.json`](security-range-6-network.json) is a follow-up with the same six-guest topology and a bounded UDP request burst. It sent 64 one-kilobyte datagrams from the client to T1, received all 64 identity-verified replies, and measured 1,241 microseconds end to end (about 51,571 requests/s and 422.5 Mbps of request payload). The socket multicast backend is local to this host and the sample is short, unpaced guest-to-guest traffic. Treat it as a diagnostic of this path, not as host NIC line rate or a general network benchmark.

## Active CPU sample

[`active-cpu-2.json`](active-cpu-2.json) records a separate matched two-VM synthetic-guest run. Each guest executes `cpu-burn 1500`; QEMU process CPU ticks are sampled during the same one-second window.

| Mode | Idle ticks per guest | Active CPU (% of one host core) | Summed process PSS |
| --- | ---: | ---: | ---: |
| Conventional baseline | 2, 3 | 101%, 102% | 421,471 KiB |
| Templated clones | 2, 2 | 102%, 102% | 109,078 KiB |

This indicates both modes consume about one host core per busy guest. DenseVM does not deduplicate guest instructions. The PSS comparison is specific to this synthetic guest and omits kernel memory overhead.

## Initial disk sync latency sample

[`disk-io-2.json`](disk-io-2.json) records a matched two-VM Alpine ext4 probe. Each guest writes 16 × 4 KiB to its own test file and calls `fdatasync` after each write. The values are per guest:

| Mode | Mean write+sync latency | p50 | p95 |
| --- | ---: | ---: | ---: |
| Conventional baseline 0 | 366 µs | 192 µs | 2,255 µs |
| Conventional baseline 1 | 444 µs | 217 µs | 3,102 µs |
| Templated clone 0 | 154 µs | 102 µs | 735 µs |
| Templated clone 1 | 207 µs | 146 µs | 1,089 µs |

The p95 spread and tiny 16-write sample make these initial observations only; they do not establish a disk performance advantage. The test measures guest-visible file write+sync latency over qcow2, including host cache and scheduling effects.
