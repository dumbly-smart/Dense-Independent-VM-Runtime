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

The QEMU COW experiments provide the actual guest evidence. For example, the
two-clone run's template mappings showed A gaining about 32,780 KiB of
`Private_Dirty` while B gained 160 KiB after A dirtied its test region; see
[`the KVM result`](../results/kvm-recovery-2026-10-30/two-clone-kvm-isolation.json).
