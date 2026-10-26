"""Sequential driver for the DenseVM divergence experiment."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


DEFAULT_PERCENTAGES = (0, 5, 10, 25, 50, 100)


def _integer(mapping: dict[str, Any], key: str, location: str) -> int:
    value = mapping.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"missing or invalid {location}.{key}")
    return value


def summarize_run(
    result: dict[str, Any], stats: dict[str, Any], *, percentage: int, count: int
) -> dict[str, int]:
    """Validate run identity/isolation and extract comparable process metrics."""
    if result.get("baseline", {}).get("count") != count:
        raise ValueError(f"baseline count does not match requested count {count}")
    if result.get("clones", {}).get("count") != count:
        raise ValueError(f"clone count does not match requested count {count}")
    if result.get("effective_ram_divergence_percent") != percentage or stats.get(
        "divergence_percent"
    ) != percentage:
        raise ValueError(f"result divergence does not match requested {percentage}% divergence")
    if stats.get("clone_count") != count:
        raise ValueError(f"stats clone count does not match requested count {count}")
    if result.get("ram_isolation") is not True:
        raise ValueError("RAM isolation check did not pass")
    if result.get("disk_isolation") is not True:
        raise ValueError("disk isolation check did not pass")

    before = stats.get("before_divergence", {})
    after = stats.get("after_divergence", {})
    return {
        "divergence_percent": percentage,
        "baseline_process_pss_kib": _integer(stats, "baseline_process_pss_kib", "stats"),
        "clone_pss_before_kib": _integer(before, "process_pss_kib", "before_divergence"),
        "clone_pss_after_kib": _integer(after, "process_pss_kib", "after_divergence"),
        "clone_private_after_kib": _integer(after, "process_private_kib", "after_divergence"),
        "clone_pss_plus_full_cache_kib": _integer(
            after, "process_pss_plus_full_template_cache_kib", "after_divergence"
        ),
    }


def _run(command: list[str]) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(command, capture_output=True, text=True)
    if completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"command failed ({completed.returncode}): {command[0]}: {detail}")
    return completed


def _write_index(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    os.replace(temporary, path)


def run_sweep(
    densevm: Path, output: Path, *, count: int, percentages: tuple[int, ...]
) -> Path:
    densevm = densevm.resolve()
    output = output.resolve()
    if not densevm.is_file() or not os.access(densevm, os.X_OK):
        raise ValueError(f"densevm executable not found or not executable: {densevm}")
    if not 1 <= count <= 32:
        raise ValueError("clone count must be between 1 and 32")
    if not percentages or any(not 0 <= value <= 100 for value in percentages):
        raise ValueError("divergence percentages must be between 0 and 100")
    if tuple(sorted(set(percentages))) != percentages:
        raise ValueError("divergence percentages must be unique and ascending")

    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    index_path = output / "sweep.json"
    index: dict[str, Any] = {
        "experiment": "densevm-ram-divergence-sweep",
        "densevm": str(densevm),
        "clone_count": count,
        "divergence_percentages": list(percentages),
        "status": "running",
        "runs": [],
    }
    _write_index(index_path, index)

    for percentage in percentages:
        run_dir = output / f"{percentage:03d}-percent"
        _run(
            [
                str(densevm),
                "probe",
                "--out",
                str(run_dir),
                "--count",
                str(count),
                "--divergence-percent",
                str(percentage),
            ]
        )
        result_path = run_dir / "result.json"
        if not result_path.is_file():
            raise RuntimeError(f"probe did not write expected result: {result_path}")
        result = json.loads(result_path.read_text())
        stats_process = _run([str(densevm), "stats", str(result_path)])
        stats = json.loads(stats_process.stdout)
        (run_dir / "summary.json").write_text(json.dumps(stats, indent=2) + "\n")
        row = summarize_run(result, stats, percentage=percentage, count=count)
        row["result"] = str(result_path.relative_to(output))
        row["summary"] = str((run_dir / "summary.json").relative_to(output))
        index["runs"].append(row)
        _write_index(index_path, index)

    index["status"] = "verified"
    _write_index(index_path, index)
    return index_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--densevm", type=Path, default=Path("target/debug/densevm"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--count", type=int, default=8)
    parser.add_argument("--percentages", type=int, nargs="+", default=DEFAULT_PERCENTAGES)
    arguments = parser.parse_args()
    try:
        index_path = run_sweep(
            arguments.densevm,
            arguments.out,
            count=arguments.count,
            percentages=tuple(arguments.percentages),
        )
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"divergence sweep failed: {error}", file=sys.stderr)
        return 1
    print(index_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
