#!/usr/bin/env python3
"""Exercise persistent clones from a relocated nested template generation."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any

from probe import GuestSerial, process_memory, ram_mapping


def cgroup_memory_snapshot(path: pathlib.Path | None = None) -> dict[str, Any]:
    if path is None:
        configured_path = os.environ.get("DENSEVM_CGROUP_PATH")
        if configured_path:
            path = pathlib.Path(configured_path)
        else:
            try:
                membership = pathlib.Path("/proc/self/cgroup").read_text().splitlines()
            except OSError as error:
                return {"status": "unavailable", "reason": f"read cgroup membership: {error}"}
            relative = next(
                (line.split("::", 1)[1] for line in membership if line.startswith("0::")), None
            )
            if relative is None:
                return {"status": "not-requested", "reason": "no cgroup-v2 membership found"}
            path = pathlib.Path("/sys/fs/cgroup") / relative.lstrip("/")

    if not path.is_absolute():
        return {"status": "unavailable", "path": str(path), "reason": "cgroup path is not absolute"}
    try:
        current_bytes = int((path / "memory.current").read_text().strip())
        try:
            memory_limit = (path / "memory.max").read_text().strip()
            memory_max_bytes = None if memory_limit == "max" else int(memory_limit)
        except OSError:
            memory_limit = None
            memory_max_bytes = None
        memory_stat = {}
        for line in (path / "memory.stat").read_text().splitlines():
            fields = line.split()
            if len(fields) != 2:
                raise ValueError(f"malformed memory.stat line: {line!r}")
            key, value = fields
            if key in memory_stat:
                raise ValueError(f"duplicate memory.stat key: {key}")
            memory_stat[key] = int(value)
        if not memory_stat:
            raise ValueError("memory.stat is empty")
    except (OSError, ValueError) as error:
        return {"status": "unavailable", "path": str(path), "reason": str(error)}
    try:
        memory_peak_bytes = int((path / "memory.peak").read_text().strip())
        if memory_peak_bytes < 0:
            raise ValueError("memory.peak must be non-negative")
        memory_peak_error = None
    except (OSError, ValueError) as error:
        memory_peak_bytes = None
        memory_peak_error = str(error)
    try:
        memory_events = {}
        for line in (path / "memory.events").read_text().splitlines():
            fields = line.split()
            if len(fields) != 2:
                raise ValueError(f"malformed memory.events line: {line!r}")
            key, value = fields
            if key in memory_events:
                raise ValueError(f"duplicate memory.events key: {key}")
            counter = int(value)
            if counter < 0:
                raise ValueError(f"memory.events counter must be non-negative: {key}")
            memory_events[key] = counter
        if not memory_events:
            raise ValueError("memory.events is empty")
        memory_events_error = None
    except (OSError, ValueError) as error:
        memory_events = None
        memory_events_error = str(error)
    return {
        "status": "measured",
        "path": str(path),
        "current_bytes": current_bytes,
        "memory_limit": memory_limit,
        "memory_max_bytes": memory_max_bytes,
        "memory_stat": memory_stat,
        "memory_peak_bytes": memory_peak_bytes,
        "memory_peak_error": memory_peak_error,
        "memory_events": memory_events,
        "memory_events_error": memory_events_error,
        "scope": "cgroup-v2 charges for this scope; other tasks in the same cgroup, file cache, and charged kernel memory are included",
    }


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run_command(
    result: dict[str, Any], binary: pathlib.Path, arguments: list[str], timeout: int = 90
) -> tuple[dict[str, Any], str]:
    completed = subprocess.run(
        [str(binary), *arguments], capture_output=True, text=True, timeout=timeout
    )
    step = {
        "command": [binary.name, *arguments],
        "exit_code": completed.returncode,
        "stdout": completed.stdout.strip(),
        "stderr": completed.stderr.strip(),
    }
    result["steps"].append(step)
    if completed.returncode:
        raise RuntimeError(f"command failed: {step}")
    return step, completed.stdout


def capture_clone_memory_maps(
    result: dict[str, Any],
    binary: pathlib.Path,
    state_dir: pathlib.Path,
    clones: list[dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    reports = {}
    for clone in clones:
        clone_id = clone["id"]
        _, stdout = run_command(
            result,
            binary,
            ["memory-map", "clone", clone_id, "--state-dir", str(state_dir)],
        )
        report = json.loads(stdout)
        identity = report.get("clone")
        if not isinstance(identity, dict) or identity.get("id") != clone_id:
            raise RuntimeError(f"memory-map reported clone ID different from {clone_id}")
        if identity.get("generation_id") != clone.get("generation_id"):
            raise RuntimeError(f"memory-map reported a different generation for {clone_id}")
        estimates = report.get("page_estimates")
        if not isinstance(estimates, dict) or not isinstance(
            estimates.get("shared_pages_estimate"), int
        ):
            raise RuntimeError(f"memory-map omitted page estimates for {clone_id}")
        reports[clone_id] = report
    return reports


def validate_sibling_survival_after_stop(
    stop_result: dict[str, Any],
    stopped_clone: dict[str, Any],
    siblings: list[dict[str, Any]],
    serials: dict[str, GuestSerial],
    expected_state: dict[str, dict[str, int]],
) -> dict[str, Any]:
    clone_id = stopped_clone.get("id")
    stopped_pid = stopped_clone.get("qemu_pid")
    if stop_result.get("state") != "stopped":
        raise RuntimeError(f"clone {clone_id} stop did not report stopped state")
    if not isinstance(stopped_pid, int) or stop_result.get("pid_gone_on_return") is not True:
        raise RuntimeError(f"clone {clone_id} QEMU process did not exit after stop")
    if pathlib.Path(f"/proc/{stopped_pid}").exists():
        raise RuntimeError(f"stopped clone {clone_id} QEMU PID {stopped_pid} is still present")

    survivors = {}
    for sibling in siblings:
        sibling_id = sibling.get("id")
        sibling_pid = sibling.get("qemu_pid")
        if not isinstance(sibling_id, str) or not isinstance(sibling_pid, int):
            raise RuntimeError("sibling clone is missing its ID or QEMU PID")
        if not pathlib.Path(f"/proc/{sibling_pid}").exists():
            raise RuntimeError(f"sibling QEMU process for clone {sibling_id} exited")
        serial = serials.get(sibling_id)
        if serial is None:
            raise RuntimeError(f"sibling serial connection for clone {sibling_id} is unavailable")
        expected = expected_state.get(sibling_id)
        if (
            not isinstance(expected, dict)
            or not isinstance(expected.get("ram"), int)
            or not isinstance(expected.get("disk"), int)
        ):
            raise RuntimeError(f"expected guest markers for sibling {sibling_id} are unavailable")
        ram = serial.command("ram-read", "RAM ")
        disk = serial.command("disk-read", "DISK ")
        if ram != f"RAM {expected['ram']}" or disk != f"DISK {expected['disk']}":
            raise RuntimeError(
                f"sibling {sibling_id} guest stopped responding correctly after clone {clone_id} stopped: RAM={ram}, DISK={disk}"
            )
        survivors[sibling_id] = {
            "qemu_pid": sibling_pid,
            "process_present": True,
            "ram_reply": ram,
            "disk_reply": disk,
        }
    if not survivors:
        raise RuntimeError("sibling survival validation requires at least one sibling")
    return {"stopped_clone": clone_id, "stopped_qemu_pid": stopped_pid, "survivors": survivors}


def run(args: argparse.Namespace) -> pathlib.Path:
    binary = args.densevm.resolve()
    manifest = args.manifest.resolve()
    output = args.out.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"densevm executable not found or not executable: {binary}")
    if not manifest.is_file():
        raise ValueError(f"template manifest does not exist: {manifest}")
    if output.exists():
        raise ValueError(f"output already exists: {output}")

    metadata = json.loads(manifest.read_text())
    ram_path = (manifest.parent / metadata["ram"]["path"]).resolve()
    disk_path = (manifest.parent / metadata["disk"]["path"]).resolve()
    state_dir = pathlib.Path(tempfile.mkdtemp(prefix="densevm-persistent-tree-"))
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result: dict[str, Any] = {
        "experiment": "persistent clones from a relocated nested template generation",
        "densevm": str(binary),
        "manifest": str(manifest),
        "generation_id": metadata.get("generation_id"),
        "state_dir": str(state_dir),
        "template_artifact_hashes_before": {
            "ram": sha256(ram_path),
            "disk": sha256(disk_path),
        },
        "steps": [],
        "status": "running",
    }
    clones: list[dict[str, Any]] = []
    serials: dict[str, GuestSerial] = {}

    try:
        _, validation_text = run_command(
            result, binary, ["validate", "template", str(manifest)]
        )
        result["validation"] = json.loads(validation_text)
        if result["validation"].get("status") != "verified":
            raise RuntimeError(f"template validation failed: {result['validation']}")

        result["cgroup_memory"] = {"before_clones": cgroup_memory_snapshot()}
        startup_started = time.monotonic()
        startup_samples = []
        for index in range(args.count):
            clone_id = f"tree-child-{index + 1:03d}"
            clone_started = time.monotonic()
            _, stdout = run_command(
                result,
                binary,
                [
                    "clone",
                    "start",
                    clone_id,
                    "--template",
                    str(manifest),
                    "--state-dir",
                    str(state_dir),
                ],
            )
            record = json.loads(stdout)
            clones.append(record)
            startup_samples.append(
                {"id": clone_id, "startup_ms": round((time.monotonic() - clone_started) * 1000, 3)}
            )
            serials[clone_id] = GuestSerial(
                state_dir / clone_id / f"{clone_id}.serial.sock"
            )
        result["clone_id_memory_maps"] = capture_clone_memory_maps(
            result, binary, state_dir, clones
        )
        result["clone_count"] = args.count
        result["configured_guest_ram_bytes"] = (
            metadata["ram"]["logical_size_bytes"] * args.count
        )
        result["startup"] = {
            "clone_samples": startup_samples,
            "time_to_n_clones_ms": round((time.monotonic() - startup_started) * 1000, 3),
        }
        result["cgroup_memory"]["after_clone_start"] = cgroup_memory_snapshot()

        for field in ("mac", "machine_id"):
            values = [clone[field] for clone in clones]
            if len(set(values)) != len(values):
                raise RuntimeError(f"clones received duplicate {field}: {values}")
        result["identities"] = {
            "macs": [clone["mac"] for clone in clones],
            "machine_ids": [clone["machine_id"] for clone in clones],
            "unique": True,
        }

        names = [clone["id"] for clone in clones]
        guest_pages = args.region_mib * 1024 * 1024 // 4096
        dirty_pages = guest_pages * args.divergence_percent // 100
        if args.divergence_mode == "all-clones":
            dirty_names = names if dirty_pages else []
        else:
            dirty_names = names[:1] if dirty_pages else []
        dirty_values = {
            name: (args.write_marker + index) % 256
            for index, name in enumerate(names)
            if name in dirty_names
        }
        result["divergence"] = {
            "mode": args.divergence_mode,
            "percent_of_guest_probe_region": args.divergence_percent,
            "probe_region_mib": args.region_mib,
            "pages_per_dirty_clone": dirty_pages,
            "targets": dirty_names,
            "byte_values": dirty_values,
            "disk_write": "one marker byte per target when divergence is nonzero",
        }
        result["initial_guest_state"] = {
            name: {
                "ram": serials[name].command("ram-read", "RAM "),
                "disk": serials[name].command("disk-read", "DISK "),
            }
            for name in names
        }
        for name in names:
            state = result["initial_guest_state"][name]
            if state != {"ram": f"RAM {args.expected_marker}", "disk": f"DISK {args.expected_marker}"}:
                raise RuntimeError(f"{name} did not inherit the expected template markers: {state}")

        def memory_snapshot() -> dict[str, Any]:
            return {
                clone["id"]: {
                    "process": process_memory(clone["qemu_pid"]),
                    "template_ram_mapping": ram_mapping(clone["qemu_pid"], ram_path),
                }
                for clone in clones
            }

        result["memory_before_divergence"] = memory_snapshot()

        for name, value in dirty_values.items():
            if dirty_pages:
                serials[name].command(
                    f"ram-write-pages {dirty_pages} {value}",
                    f"RAM-WROTE-PAGES {dirty_pages} {value}",
                )
                serials[name].command(f"disk-write {value}", f"DISK-WROTE {value}")
        result["guest_state_after_divergence"] = {
            name: {
                "ram": serials[name].command("ram-read", "RAM "),
                "disk": serials[name].command("disk-read", "DISK "),
            }
            for name in names
        }
        expected = {
            name: {
                "ram": f"RAM {dirty_values.get(name, args.expected_marker)}",
                "disk": f"DISK {dirty_values.get(name, args.expected_marker)}",
            }
            for name in names
        }
        if result["guest_state_after_divergence"] != expected:
            raise RuntimeError(
                f"RAM or disk write leaked across clones: {result['guest_state_after_divergence']}"
            )

        result["memory_after_divergence"] = memory_snapshot()
        result["cgroup_memory"]["after_divergence"] = cgroup_memory_snapshot()
        result["aggregate_memory"] = {
            phase: {
                section: {
                    field: sum(sample[section][field] for sample in snapshot.values())
                    for field in fields
                }
                for section, fields in (
                    ("process", ("pss_kib", "private_dirty_kib")),
                    (
                        "template_ram_mapping",
                        ("pss_kib", "shared_clean_kib", "shared_dirty_kib", "private_dirty_kib"),
                    ),
                )
            }
            for phase, snapshot in (
                ("before_divergence", result["memory_before_divergence"]),
                ("after_divergence", result["memory_after_divergence"]),
            )
        }
        result["memory_delta"] = {
            clone_id: {
                field: (
                    result["memory_after_divergence"][clone_id][section][field]
                    - result["memory_before_divergence"][clone_id][section][field]
                )
                for section, field in (
                    ("process", "pss_kib"),
                    ("process", "private_dirty_kib"),
                    ("template_ram_mapping", "pss_kib"),
                    ("template_ram_mapping", "private_dirty_kib"),
                )
            }
            for clone_id in names
        }
        result["template_artifact_hashes_after"] = {
            "ram": sha256(ram_path),
            "disk": sha256(disk_path),
        }
        if result["template_artifact_hashes_before"] != result["template_artifact_hashes_after"]:
            raise RuntimeError("a persistent clone changed an immutable template artifact")
        if len(clones) > 1:
            stopped_clone = clones[0]
            _, stop_text = run_command(
                result,
                binary,
                ["clone", "stop", stopped_clone["id"], "--state-dir", str(state_dir)],
            )
            stop_result = json.loads(stop_text)
            stop_result["pid_gone_on_return"] = not pathlib.Path(
                f"/proc/{stopped_clone['qemu_pid']}"
            ).exists()
            result.setdefault("stops", []).append(
                {
                    "id": stopped_clone["id"],
                    "state": stop_result.get("state"),
                    "pid_gone_on_return": stop_result["pid_gone_on_return"],
                }
            )
            expected_state = {
                name: {
                    "ram": dirty_values.get(name, args.expected_marker),
                    "disk": dirty_values.get(name, args.expected_marker),
                }
                for name in names
            }
            result["sibling_survival_after_stop"] = validate_sibling_survival_after_stop(
                stop_result,
                stopped_clone,
                clones[1:],
                serials,
                expected_state,
            )
            result["cgroup_memory"]["after_sibling_stop"] = cgroup_memory_snapshot()
        result["status"] = "verified"
    except Exception as error:  # Preserve failure evidence for interrupted experiments.
        result["status"] = "failed"
        result["error"] = repr(error)
    finally:
        for serial in serials.values():
            try:
                serial.close()
            except OSError as error:
                result.setdefault("cleanup_errors", []).append(repr(error))
        for clone in clones:
            pid = clone.get("qemu_pid")
            if not isinstance(pid, int) or not pathlib.Path(f"/proc/{pid}").exists():
                continue
            try:
                _, stdout = run_command(
                    result,
                    binary,
                    ["clone", "stop", clone["id"], "--state-dir", str(state_dir)],
                )
                stopped = json.loads(stdout)
                result.setdefault("stops", []).append(
                    {
                        "id": clone["id"],
                        "state": stopped["state"],
                        "pid_gone_on_return": not pathlib.Path(f"/proc/{pid}").exists(),
                    }
                )
            except Exception as error:
                result.setdefault("cleanup_errors", []).append(repr(error))
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

        pids = [clone.get("qemu_pid") for clone in clones if isinstance(clone.get("qemu_pid"), int)]
        for _ in range(200):
            result["cleanup_leftover_pids"] = [
                pid for pid in pids if pathlib.Path(f"/proc/{pid}").exists()
            ]
            if not result["cleanup_leftover_pids"]:
                break
            time.sleep(0.01)
        if not result.get("cleanup_leftover_pids"):
            shutil.rmtree(state_dir, ignore_errors=True)
        if result.get("cleanup_errors") or any(
            not stop["pid_gone_on_return"] for stop in result.get("stops", [])
        ):
            result["status"] = "failed"
            result.setdefault("error", "one or more clone cleanup operations did not complete cleanly")
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")

    if result["status"] != "verified" or result.get("cleanup_leftover_pids"):
        raise RuntimeError(f"persistent template experiment {result['status']}: {output / 'result.json'}")
    return output / "result.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--densevm", type=pathlib.Path, default=pathlib.Path("target/debug/densevm"))
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--expected-marker", type=int, default=73)
    parser.add_argument("--write-marker", type=int, default=91)
    parser.add_argument("--count", type=int, default=2)
    parser.add_argument("--divergence-percent", type=int, default=100)
    parser.add_argument(
        "--divergence-mode", choices=("first-clone", "all-clones"), default="first-clone"
    )
    parser.add_argument("--region-mib", type=int, default=32)
    args = parser.parse_args()
    if not 0 <= args.expected_marker <= 255 or not 0 <= args.write_marker <= 255:
        parser.error("markers must be bytes in the range 0..255")
    if not 2 <= args.count <= 32:
        parser.error("clone count must be between 2 and 32")
    if not 0 <= args.divergence_percent <= 100:
        parser.error("divergence percentage must be between 0 and 100")
    if not 1 <= args.region_mib <= 32:
        parser.error("probe region must be between 1 and 32 MiB")
    try:
        print(run(args))
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"persistent template experiment failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
