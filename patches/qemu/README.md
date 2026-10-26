# QEMU patch experiments

## `0001-trace-tcg-generated-block-size.patch`

This patch targets QEMU 11.1.2. QEMU's existing `translate_block` trace event
is emitted before code generation. The new `translate_block_complete` event
fires after QEMU assigns `tb->tc.size`, exposing the generated host-code size
for each successful `tb_gen_code` call without emitting a full disassembly.

The byte counts are cumulative emitted code. They are not current TCG cache
residency: a block may later be discarded, including after a duplicate-block
race. This patch does not measure translation duration. Failed or restarted
translations can have a start event without a completion event; consumers must
pair records by TB pointer and report unmatched starts separately.

The patch is not yet built into or selected by DenseVM. It must be applied to
the matching QEMU source and built as a separate binary before the runtime can
collect this event. No upstream QEMU files are vendored here.
