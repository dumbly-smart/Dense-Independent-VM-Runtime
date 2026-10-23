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
