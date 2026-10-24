#!/usr/bin/env python3
"""Summarize paired DenseVM probe JSON without inventing missing measurements."""

import json
import sys
from pathlib import Path
from typing import Any


COUNTERS = (
    "pages_shared",
    "pages_sharing",
    "pages_unshared",
    "pages_volatile",
)
COUNTER_SCOPE_WARNING = (
    "KSM counters and ksmd CPU ticks are host-global; concurrent host workloads "
    "can affect these deltas and they cannot be attributed solely to DenseVM."
)


def read_probe(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read probe JSON {path}: {error}") from error
    if not isinstance(data, dict) or data.get("experiment") != "two-clone-cow-probe":
        raise ValueError(f"{path} must be a two-clone-cow-probe result")
    if not isinstance(data.get("snapshots"), dict) or not isinstance(
        data.get("ksm_telemetry"), dict
    ):
        raise ValueError(f"{path} is missing snapshots or KSM telemetry")
    clones = data.get("clones")
    if (
        not isinstance(clones, dict)
        or type(clones.get("count")) is not int
        or type(data.get("guest_ram_mib")) is not int
        or type(data.get("ram_divergence_pages")) is not int
        or not isinstance(data.get("divergence_scope"), str)
    ):
        raise ValueError(f"{path} is missing comparable probe configuration")
    host = data.get("host")
    guest = data.get("guest")
    if (
        not isinstance(guest, dict)
        or not guest
        or not isinstance(host, dict)
        or not isinstance(data.get("qemu_version"), str)
        or not isinstance(data.get("kernel_version"), str)
        or not isinstance(host.get("kernel_version"), str)
        or type(host.get("page_size_bytes")) is not int
        or not isinstance(host.get("kvm"), dict)
        or type(host["kvm"].get("accessible")) is not bool
    ):
        raise ValueError(f"{path} is missing guest or host provenance")
    ksm_run = data.get("ksm_run")
    if isinstance(ksm_run, str) and ksm_run in ("0", "1", "2"):
        data["ksm_run"] = int(ksm_run)
    elif type(ksm_run) is not int or ksm_run not in (0, 1, 2):
        raise ValueError(f"{path} has missing or invalid ksm_run state")
    return data


def _sum_metric(data: dict[str, Any], snapshot: str, group: str) -> dict[str, int | None]:
    result: dict[str, int | None] = {}
    for phase in ("before_divergence", "after_divergence"):
        clones = data.get("snapshots", {}).get(phase, {}).get("clones")
        values = [
            clone.get(group, {}).get(snapshot)
            for clone in clones
        ] if isinstance(clones, list) else []
        if not values or any(type(value) is not int for value in values):
            result["before" if phase == "before_divergence" else "after"] = None
        else:
            result["before" if phase == "before_divergence" else "after"] = sum(values)
    return result


def _counter_deltas(data: dict[str, Any]) -> dict[str, int | None]:
    telemetry = data["ksm_telemetry"]
    before = telemetry.get("counters_before")
    after = telemetry.get("counters_after")
    before = before if isinstance(before, dict) else {}
    after = after if isinstance(after, dict) else {}
    return {
        name: after[name] - before[name]
        if type(before.get(name)) is int and type(after.get(name)) is int
        else None
        for name in COUNTERS
    }


def _run_summary(data: dict[str, Any], source: Path, ksm_requested: bool) -> dict[str, Any]:
    telemetry = data["ksm_telemetry"]
    return {
        "source_result": str(source),
        "ksm_requested": ksm_requested,
        "clone_pss_kib": _sum_metric(data, "pss_kib", "memory"),
        "ram_mapping_pss_kib": _sum_metric(data, "pss_kib", "ram_mapping"),
        "ksm_counter_deltas": _counter_deltas(data),
        "ksmd_cpu_ticks_delta": telemetry.get("ksmd_cpu_ticks_delta"),
        "ksm_telemetry_scope": telemetry.get("scope"),
        "ksm_run_at_result": data.get("ksm_run"),
    }


def summarize(control_path: Path, ksm_path: Path) -> dict[str, Any]:
    control = read_probe(control_path)
    with_ksm = read_probe(ksm_path)
    comparison_fields = (
        ("clone count", control.get("clones", {}).get("count"), with_ksm.get("clones", {}).get("count")),
        ("guest RAM MiB", control.get("guest_ram_mib"), with_ksm.get("guest_ram_mib")),
        ("divergence pages", control.get("ram_divergence_pages"), with_ksm.get("ram_divergence_pages")),
        ("divergence scope", control.get("divergence_scope"), with_ksm.get("divergence_scope")),
        ("guest provenance", control.get("guest"), with_ksm.get("guest")),
        ("QEMU version", control.get("qemu_version"), with_ksm.get("qemu_version")),
        ("kernel version", control.get("kernel_version"), with_ksm.get("kernel_version")),
    )
    control_host = control["host"]
    ksm_host = with_ksm["host"]
    host_fields = (
        ("kernel version", control_host.get("kernel_version"), ksm_host.get("kernel_version")),
        ("page size", control_host.get("page_size_bytes"), ksm_host.get("page_size_bytes")),
        ("KVM accessibility", control_host.get("kvm", {}).get("accessible"), ksm_host.get("kvm", {}).get("accessible")),
    )
    mismatches = [
        f"{name}: control={left!r}, with_ksm={right!r}"
        for name, left, right in comparison_fields
        if left != right
    ]
    host_mismatches = [
        f"{name}: control={left!r}, with_ksm={right!r}"
        for name, left, right in host_fields
        if left != right
    ]
    if host_mismatches:
        raise ValueError("probe runs have different host configuration: " + "; ".join(host_mismatches))
    if mismatches:
        raise ValueError("probe runs are not comparable: " + "; ".join(mismatches))
    if control["ksm_run"] != 0 or with_ksm["ksm_run"] != 1:
        raise ValueError(
            "probe runs have invalid ksm_run roles: "
            f"control={control['ksm_run']!r} (expected 0), "
            f"with_ksm={with_ksm['ksm_run']!r} (expected 1)"
        )
    return {
        "schema_version": 1,
        "experiment": "template-cow-versus-template-cow-plus-ksm",
        "status": "paired",
        "configuration": {
            "clone_count": control["clones"]["count"],
            "guest_ram_mib": control["guest_ram_mib"],
            "divergence_pages": control["ram_divergence_pages"],
            "divergence_scope": control["divergence_scope"],
            "guest": control["guest"],
            "qemu_version": control["qemu_version"],
            "kernel_version": control["kernel_version"],
            "page_size_bytes": control_host["page_size_bytes"],
            "kvm_accessible": control_host["kvm"]["accessible"],
        },
        "counter_scope_warning": COUNTER_SCOPE_WARNING,
        "control": _run_summary(control, control_path, False),
        "with_ksm": _run_summary(with_ksm, ksm_path, True),
    }


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(f"usage: {argv[0]} CONTROL_RESULT.json KSM_RESULT.json OUTPUT.json", file=sys.stderr)
        return 2
    try:
        result = summarize(Path(argv[1]), Path(argv[2]))
        Path(argv[3]).write_text(json.dumps(result, indent=2) + "\n")
    except (OSError, ValueError) as error:
        print(f"KSM summary error: {error}", file=sys.stderr)
        return 1
    print(argv[3])
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
