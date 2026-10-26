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

        for clone_id in ("tree-child-a", "tree-child-b"):
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
            serials[clone_id] = GuestSerial(
                state_dir / clone_id / f"{clone_id}.serial.sock"
            )

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

        serials[names[0]].command(
            f"ram-write {args.write_marker}", f"RAM-WROTE {args.write_marker}"
        )
        serials[names[0]].command(
            f"disk-write {args.write_marker}", f"DISK-WROTE {args.write_marker}"
        )
        result["after_first_clone_writes"] = {
            name: {
                "ram": serials[name].command("ram-read", "RAM "),
                "disk": serials[name].command("disk-read", "DISK "),
            }
            for name in names
        }
        expected = {
            names[0]: {"ram": f"RAM {args.write_marker}", "disk": f"DISK {args.write_marker}"},
            names[1]: {"ram": f"RAM {args.expected_marker}", "disk": f"DISK {args.expected_marker}"},
        }
        if result["after_first_clone_writes"] != expected:
            raise RuntimeError(
                f"RAM or disk write leaked across clones: {result['after_first_clone_writes']}"
            )

        result["memory"] = {
            clone["id"]: {
                "process": process_memory(clone["qemu_pid"]),
                "template_ram_mapping": ram_mapping(clone["qemu_pid"], ram_path),
            }
            for clone in clones
        }
        result["template_artifact_hashes_after"] = {
            "ram": sha256(ram_path),
            "disk": sha256(disk_path),
        }
        if result["template_artifact_hashes_before"] != result["template_artifact_hashes_after"]:
            raise RuntimeError("a persistent clone changed an immutable template artifact")
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
    args = parser.parse_args()
    if not 0 <= args.expected_marker <= 255 or not 0 <= args.write_marker <= 255:
        parser.error("markers must be bytes in the range 0..255")
    try:
        print(run(args))
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"persistent template experiment failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
