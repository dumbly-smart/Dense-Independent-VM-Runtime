#!/bin/sh
set -eu

out=${1:?usage: run.sh OUTPUT_DIR [DENSEVM_BIN]}
bin=${2:-target/debug/densevm}
mkdir -p "$out"

for percent in 0 5 10 25 50 100; do
    run="$out/divergence-${percent}pct"
    "$bin" probe --out "$run" --count 2 --divergence-percent "$percent" >/dev/null
done

jq -s '[.[] | {
  ram_divergence_pages,
  ram_divergence_bytes,
  effective_ram_divergence_percent,
  clone_a_pss_kib: .clones.a.after_ram_write_memory.pss_kib,
  clone_b_pss_kib: .clones.b.after_a_ram_write_memory.pss_kib,
  clone_a_private_ram_kib: .clones.a.after_ram_write_ram_mapping.private_dirty_kib,
  clone_b_private_ram_kib: .clones.b.after_a_ram_write_ram_mapping.private_dirty_kib,
  ram_isolation,
  disk_isolation
}] | sort_by(.effective_ram_divergence_percent)' "$out"/divergence-*/result.json > "$out/divergence.json"
printf '%s\n' "$out/divergence.json"
