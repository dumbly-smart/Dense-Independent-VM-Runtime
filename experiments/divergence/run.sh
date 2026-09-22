#!/bin/sh
set -eu

out=${1:?usage: run.sh OUTPUT_DIR [DENSEVM_BIN]}
bin=${2:-target/debug/densevm}
mkdir -p "$out"

results=""
for mib in 1 2 4 8 16 32; do
    run="$out/divergence-${mib}mib"
    "$bin" probe --out "$run" --count 2 --divergence-mib "$mib"
    results="$results $run/result.json"
done

jq -s '[.[] | {
  requested_ram_divergence_mib,
  clone_a_pss_kib: .clones.a.after_ram_write_memory.pss_kib,
  clone_b_pss_kib: .clones.b.after_a_ram_write_memory.pss_kib,
  clone_a_private_ram_kib: .clones.a.after_ram_write_ram_mapping.private_dirty_kib,
  clone_b_private_ram_kib: .clones.b.after_a_ram_write_ram_mapping.private_dirty_kib,
  ram_isolation,
  disk_isolation
}] | sort_by(.requested_ram_divergence_mib)' $results > "$out/divergence.json"
printf '%s\n' "$out/divergence.json"
