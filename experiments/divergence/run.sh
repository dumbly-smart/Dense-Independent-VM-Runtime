#!/bin/sh
set -eu

out=${1:?usage: run.sh OUTPUT_DIR [DENSEVM_BIN]}
bin=${2:-target/debug/densevm}
count=${3:-8}
mkdir "$out"

for percent in 0 5 10 25 50 100; do
    run="$out/divergence-${percent}pct"
    "$bin" probe --out "$run" --count "$count" --divergence-percent "$percent" --divergence-all >/dev/null
    "$bin" stats "$run/result.json" > "$run/summary.json"
done

jq -s 'sort_by(.divergence_percent)' "$out"/divergence-*/summary.json > "$out/divergence.json"
printf '%s\n' "$out/divergence.json"
