# Inspecting a clone's RAM mapping

`densevm memory-map PID RAM_FILE` reads the matching file-backed entries in
`/proc/PID/smaps`. It reports the resident, proportional, shared, and private
KiB totals already used by DenseVM's measurements, plus page-count estimates
for resident, shared, private, and private-dirty RAM.

The estimates use the host's base page size from `getconf PAGESIZE`. Each KiB
total is converted to bytes and rounded to the nearest page. PSS is deliberately
not converted to pages: PSS apportions shared pages among mappings and is not a
physical-page count.

## Example

```sh
cargo run -- memory-map 12345 /var/lib/densevm/template/memory.img
```

Use the PID of a live QEMU process and the exact RAM backing-file path shown in
that QEMU command. The command only reads `/proc`; it does not read the guest
RAM file's contents.

For a persistent DenseVM clone, the clone ID form resolves the PID and RAM
backing file from the read-only clone registry and the live QEMU command line:

```sh
cargo run -- memory-map clone vm01 --state-dir /var/lib/densevm
```

This form only accepts a `running` record with a matching QEMU boot ID and
process start time. It samples the process, then checks the start time again
to catch exit or PID reuse during the sample. It requires same-user access to
the state directory and `/proc/<pid>`.

## Reading the classifications

- `shared_pages_estimate` is derived from the matching mapping's
  `Shared_Clean` plus `Shared_Dirty` fields in `smaps`.
- `private_pages_estimate` is derived from `Private_Clean` plus
  `Private_Dirty`.
- `private_dirty_pages_estimate` is the dirty subset of private pages. It is a
  useful indicator of divergence, but not a precise count of guest writes:
  QEMU can dirty guest RAM for other reasons, and some guest changes need not
  appear as a private dirty page at the instant sampled.
- A `Private_Clean` page may still refer to file-backed template contents; it
  is called private because only this process maps it at sample time. A single
  process cannot demonstrate inter-process page sharing by itself.
- These are per-process, per-file mapping aggregates. They do not show guest
  physical page numbers, associate a host page with a guest address, or expose
  guest contents. They also do not account for KVM allocations, kernel page
  tables, or other QEMU mappings.

`smaps` reports KiB values, so conversion to page counts is an estimate. The
result labels this limit in its `page_estimates.accounting` field. For a
clone-to-clone comparison, sample each live QEMU process against the same RAM
file and compare its shared/private categories at the same workload point.

## Command smoke evidence

The real CLI was exercised against two processes mapping the same 8 MiB file
with private writable mappings; one process wrote 128 pages while both kept
the file pages resident. On the tested 4 KiB page-size host, the sampled writer
reported 2,048 resident pages, 1,920 shared pages, and 128 private-dirty pages.
This validates `/proc/smaps` parsing and page conversion, not QEMU/KVM behavior.
Raw output: [`results/memory-map-smoke.json`](../results/memory-map-smoke.json).

A separate live-QEMU smoke validated clone-ID resolution using a synthesized
registry record and a QEMU process paused before guest execution. The command
found its RAM path and process identity; no guest pages were resident, so this
is lookup coverage only, not COW evidence:
[`results/memory-map-live-qemu-smoke.json`](../results/memory-map-live-qemu-smoke.json).

The QEMU COW experiment provides guest evidence in the same schema. In a
fresh two-clone run that dirtied 100% of the 32 MiB test region in A, its
private-dirty estimate rose by 8,208 pages (32,832 KiB), while B rose by 52
pages (208 KiB). This is one sample; the complete result and extracted
comparison are [`page-estimates-cow.json`](../results/kvm-recovery-2026-10-30/page-estimates-cow.json)
and [`page-estimates-cow-summary.json`](../results/kvm-recovery-2026-10-30/page-estimates-cow-summary.json).
