#!/usr/bin/env python3
"""Plot measured data only; requires matplotlib (for example uv run --with matplotlib)."""
import json
import sys
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

source = Path(sys.argv[1])
rows = sorted(json.loads(source.read_text()), key=lambda row: row["divergence_percent"])
if not rows or len({row["clone_count"] for row in rows}) != 1:
    raise SystemExit("expected nonempty measurements with one clone count")
percent = [row["divergence_percent"] for row in rows]
fig, ax = plt.subplots(figsize=(8, 4.8))
ax.plot(percent, [row["baseline_process_pss_kib"] / 1024 for row in rows], "o--", label="Conventional QEMU process PSS")
ax.plot(percent, [row["after_divergence"]["process_pss_kib"] / 1024 for row in rows], "o-", label="Clone process PSS")
ax.plot(percent, [row["after_divergence"]["process_pss_plus_full_template_cache_kib"] / 1024 for row in rows], "s-", label="Clone PSS + full template cache (overcount)")
region_mib = rows[0]["divergence_region_bytes"] / (1024 * 1024)
ax.set(xlabel=f"Divergence within each {region_mib:g} MiB test buffer (%)", ylabel="Measured resident memory (MiB)", title=f"DenseVM: {rows[0]['clone_count']} synthetic Linux clones")
ax.set_xticks(percent)
ax.grid(alpha=0.25)
ax.legend(fontsize=8)
fig.text(0.5, 0.01, "Kernel/KVM metadata excluded. 100% of this buffer is not 100% of guest RAM.", ha="center", fontsize=8)
fig.tight_layout(rect=(0, 0.04, 1, 1))
output = Path(sys.argv[2]) if len(sys.argv) > 2 else source.with_suffix(".svg")
if output.exists():
    raise SystemExit(f"refusing to overwrite {output}")
fig.savefig(output)
print(output)
