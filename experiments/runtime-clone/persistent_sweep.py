#!/usr/bin/env python3
"""Measure all-clone RAM divergence for persistent template clones."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
from typing import Any


DEFAULT_PERCENTAGES = (0, 5, 10, 25, 50, 100)


def write_index(path: pathlib.Path, index: dict[str, Any]) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(index, indent=2) + "\n")
    temporary.replace(path)


def write_plot(path: pathlib.Path, rows: list[dict[str, Any]]) -> None:
    width, height = 1000, 700
    left, right = 120, 940
    plot_width = right - left
    x_values = [row["percent"] for row in rows]

    def panel(
        title: str,
        unit: str,
        series: list[tuple[str, list[float], str]],
        top: int,
        panel_height: int,
    ) -> list[str]:
        values = [value for _, data, _ in series for value in data]
        low, high = min(values), max(values)
        spread = max(high - low, 1.0)
        low = max(0.0, low - spread * 0.08)
        high += spread * 0.08
        bottom = top + panel_height
        parts = [f'<text x="{left}" y="{top - 24}" class="title">{title}</text>']
        for tick in range(5):
            fraction = tick / 4
            y = bottom - fraction * panel_height
            value = low + fraction * (high - low)
            parts.append(f'<line x1="{left}" y1="{y:.1f}" x2="{right}" y2="{y:.1f}" class="grid"/>')
            parts.append(f'<text x="{left - 12}" y="{y + 4:.1f}" text-anchor="end" class="axis">{value:.0f} {unit}</text>')
        for percent in (0, 25, 50, 75, 100):
            x = left + plot_width * percent / 100
            parts.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{bottom}" class="grid"/>')
            parts.append(f'<text x="{x:.1f}" y="{bottom + 22}" text-anchor="middle" class="axis">{percent}%</text>')
        for line_index, (label, data, color) in enumerate(series):
            points = []
            for percent, value in zip(x_values, data):
                x = left + plot_width * percent / 100
                y = bottom - (value - low) / (high - low) * panel_height
                points.append(f"{x:.1f},{y:.1f}")
                parts.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="5" fill="{color}"/>')
            parts.append(f'<polyline points="{" ".join(points)}" fill="none" stroke="{color}" stroke-width="3"/>')
            legend_x = right - 230 + line_index * 125
            parts.append(f'<line x1="{legend_x}" y1="{top - 10}" x2="{legend_x + 22}" y2="{top - 10}" stroke="{color}" stroke-width="3"/>')
            parts.append(f'<text x="{legend_x + 28}" y="{top - 6}" class="axis">{label}</text>')
        return parts

    pss_before = [row["process_pss_before_kib"] / 1024 for row in rows]
    pss_after = [row["process_pss_after_kib"] / 1024 for row in rows]
    private_delta = [row["template_mapping_private_dirty_delta_kib"] / 1024 for row in rows]
    pieces = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<style>text{font-family:Arial,sans-serif;fill:#172033}.title{font-size:18px;font-weight:bold}.axis{font-size:12px}.grid{stroke:#d8dee9;stroke-width:1}.foot{font-size:13px;fill:#475569}</style>',
        '<text x="500" y="35" text-anchor="middle" class="title">Eight persistent clones: RAM divergence</text>',
    ]
    pieces.extend(
        panel(
            "Aggregate QEMU process PSS",
            "MiB",
            [("Before writes", pss_before, "#2563eb"), ("After writes", pss_after, "#dc2626")],
            105,
            190,
        )
    )
    pieces.extend(
        panel(
            "Template RAM mapping Private_Dirty increase",
            "MiB",
            [("All clones", private_delta, "#059669")],
            430,
            160,
        )
    )
    pieces.append('<text x="500" y="665" text-anchor="middle" class="foot">PSS and smaps only; KVM/kernel memory is excluded. Each clone dirties the stated fraction of a 32 MiB test region.</text>')
    pieces.append("</svg>")
    path.write_text("\n".join(pieces) + "\n")


def run_sweep(args: argparse.Namespace) -> pathlib.Path:
    binary = args.densevm.resolve()
    manifest = args.manifest.resolve()
    output = args.out.resolve()
    experiment = pathlib.Path(__file__).with_name("persistent_tree.py")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"densevm executable not found or not executable: {binary}")
    if not manifest.is_file():
        raise ValueError(f"template manifest does not exist: {manifest}")
    if not 2 <= args.count <= 32:
        raise ValueError("clone count must be between 2 and 32")
    if not args.percentages or any(not 0 <= value <= 100 for value in args.percentages):
        raise ValueError("divergence percentages must be between 0 and 100")
    if tuple(sorted(set(args.percentages))) != tuple(args.percentages):
        raise ValueError("divergence percentages must be unique and ascending")
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    index_path = output / "sweep.json"
    index: dict[str, Any] = {
        "experiment": "persistent-clone-all-guest-ram-divergence",
        "densevm": str(binary),
        "manifest": str(manifest),
        "clone_count": args.count,
        "divergence_percentages": list(args.percentages),
        "probe_region_mib": args.region_mib,
        "status": "running",
        "runs": [],
    }
    write_index(index_path, index)

    for percentage in args.percentages:
        run_dir = output / f"{percentage:03d}-percent"
        command = [
            sys.executable,
            str(experiment),
            "--densevm",
            str(binary),
            "--manifest",
            str(manifest),
            "--out",
            str(run_dir),
            "--count",
            str(args.count),
            "--region-mib",
            str(args.region_mib),
            "--divergence-mode",
            "all-clones",
            "--divergence-percent",
            str(percentage),
        ]
        completed = subprocess.run(command, capture_output=True, text=True, timeout=300)
        command_record = {
            "command": command,
            "exit_code": completed.returncode,
            "stdout": completed.stdout.strip(),
            "stderr": completed.stderr.strip(),
        }
        if completed.returncode:
            index["status"] = "failed"
            index["failed_run"] = command_record
            write_index(index_path, index)
            raise RuntimeError(f"divergence run failed at {percentage}%: {command_record}")

        result_path = run_dir / "result.json"
        result = json.loads(result_path.read_text())
        if result.get("status") != "verified":
            index["status"] = "failed"
            index["failed_run"] = command_record
            write_index(index_path, index)
            raise RuntimeError(f"divergence run was not verified at {percentage}%")
        before = result["aggregate_memory"]["before_divergence"]
        after = result["aggregate_memory"]["after_divergence"]
        row = {
            "percent": percentage,
            "pages_dirtied_per_clone": result["divergence"]["pages_per_dirty_clone"],
            "all_clones_verified": len(result["divergence"]["targets"]) == args.count
            if percentage
            else True,
            "process_pss_before_kib": before["process"]["pss_kib"],
            "process_pss_after_kib": after["process"]["pss_kib"],
            "process_pss_delta_kib": after["process"]["pss_kib"]
            - before["process"]["pss_kib"],
            "process_private_dirty_before_kib": before["process"]["private_dirty_kib"],
            "process_private_dirty_after_kib": after["process"]["private_dirty_kib"],
            "process_private_dirty_delta_kib": after["process"]["private_dirty_kib"]
            - before["process"]["private_dirty_kib"],
            "template_mapping_private_dirty_before_kib": before["template_ram_mapping"][
                "private_dirty_kib"
            ],
            "template_mapping_private_dirty_after_kib": after["template_ram_mapping"][
                "private_dirty_kib"
            ],
            "template_mapping_private_dirty_delta_kib": after["template_ram_mapping"][
                "private_dirty_kib"
            ]
            - before["template_ram_mapping"]["private_dirty_kib"],
            "time_to_n_clones_ms": result["startup"]["time_to_n_clones_ms"],
            "result": str(result_path.relative_to(output)),
            "command": command_record,
        }
        if not row["all_clones_verified"]:
            raise RuntimeError(f"not all clones diverged at {percentage}%")
        index["runs"].append(row)
        write_index(index_path, index)

    index["status"] = "verified"
    write_index(index_path, index)
    write_plot(output / "divergence.svg", index["runs"])
    return index_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--densevm", type=pathlib.Path, default=pathlib.Path("target/debug/densevm"))
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--count", type=int, default=8)
    parser.add_argument("--region-mib", type=int, default=32)
    parser.add_argument("--percentages", type=int, nargs="+", default=DEFAULT_PERCENTAGES)
    args = parser.parse_args()
    try:
        print(run_sweep(args))
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"persistent divergence sweep failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
