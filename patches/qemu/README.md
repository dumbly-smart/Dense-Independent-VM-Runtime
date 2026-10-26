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

The patch is not built into the system QEMU. It was applied to upstream QEMU
v11.1.2 and built as a separate x86_64-softmmu TCG binary. DenseVM detects
`translate_block_complete` at runtime and enables it only when present. The
patched experiment is saved under `results/tcg/2026-10-26/codegen3/`; a stock
QEMU fallback run is under `results/tcg/2026-10-26/stocktrace/`.

The experiment used:

```sh
../qemu-11.1.2/configure --target-list=x86_64-softmmu --disable-docs \
  --disable-tools --disable-kvm --enable-tcg --prefix=/tmp/qemu-install \
  --disable-werror
ninja -C /tmp/qemu-build -j4 qemu-system-x86_64
PATH=/tmp/qemu-build:$PATH cargo run -- benchmark baseline --count 1 \
  --accel tcg --trace-translations --out results/tcg/2026-10-26/codegen
```

The source archive was the QEMU `v11.1.2` tag. No upstream QEMU files or
compiled binary are vendored here.
