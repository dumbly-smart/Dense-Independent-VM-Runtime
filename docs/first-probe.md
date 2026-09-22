# First two-clone result

The first complete run used QEMU 11.1.1, Linux 7.2.6, 256 MiB configured
guest RAM and a 64 MiB virtual disk per guest. The Linux guest was a static
Rust initramfs probe, not a full distribution. The raw machine-readable result
is [the reference JSON](../results/probe-2026-09-22-qemu-11.1.1.json); rerun with
`cargo test --test probe -- --ignored --nocapture`.

| Measurement | Two ordinary VMs | Two restored clones |
| --- | ---: | ---: |
| Sum of QEMU process PSS at ready | 408,515 KiB | 104,900 KiB |
| Time to two ready/resumed guests | 1,441 ms | 270 ms |
| Template RAM file resident in page cache | n/a | 160,612 KiB |
| Clone A RAM mapping, shared before write | n/a | 22,536 KiB |
| Clone A RAM mapping, private dirty before write | n/a | 2,020 KiB |
| Clone A RAM mapping, private dirty after 32 MiB write | n/a | 34,912 KiB |
| Clone B RAM mapping, private dirty after A write | n/a | 2,236 KiB |
| Each overlay allocated before a disk write | n/a | 200,704 bytes |
| Each overlay allocated after its disk write | n/a | 331,776 bytes |

The two clone processes' PSS is 74% below the two baseline processes' PSS.
That percentage is **not** a complete host physical memory saving: the frozen
RAM file also had about 157 MiB resident in the host page cache. Some of those
cache pages are included in clone PSS, while other resident pages are not
mapped by either clone. Counting the file cache and subtracting the mapped
shared-file PSS gives a rough 237 MiB user-space-plus-template-cache estimate
for the clones, versus 399 MiB process PSS for the baseline. The estimate
excludes KVM page tables and other kernel memory, and the baseline measure
does not include all of its file cache. It is evidence of a meaningful saving
in this tiny workload, not a general density factor.

The RAM-file mapping result is more direct evidence of COW: before divergence,
each clone had about 22 MiB of shared mapped RAM and about 2 MiB private.
After only A changed one byte per 4 KiB page in a 32 MiB guest buffer, A's
private mapping grew by about 32 MiB while B's changed by 216 KiB. Both guests
still read their own expected bytes. `x-ignore-shared` produced a migration
state file of about 304 KiB while the source RAM file allocated about 157 MiB;
the file size alone is not proof of correctness, so restore and isolation were
also tested.

Both overlays pointed to the same immutable base, their allocation grew only
after guest writes, and A's disk data did not appear in B. B kept responding
after A's QEMU process was killed. Guest-visible MACs and QMP vmgenids were
distinct. Full Linux identity, timing, entropy, device state, and adversarial
isolation remain unverified; see the semantic status table.
