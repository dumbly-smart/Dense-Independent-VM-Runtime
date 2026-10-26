#!/usr/bin/env python3
"""Measure live DenseVM clones and verify RAM/disk isolation."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")
PROBE_REGION_MIB = 32


def command_json(command: list[str]) -> Any:
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {command}: {result.stderr}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"command returned invalid JSON: {command}: {error}") from error


def read_metrics(path: Path, tracked: set[str]) -> dict[str, int]:
    values: dict[str, int] = {}
    for line in path.read_text().splitlines():
        key, separator, value = line.partition(":")
        if separator and key in tracked:
            values[key.lower() + "_kib"] = int(value.strip().split()[0])
    present = {key.removesuffix("_kib").replace("_", " ").title().replace(" ", "_") for key in values}
    missing = tracked - present
    if missing:
        raise RuntimeError(f"{path} lacks memory metrics: {sorted(missing)}")
    return values


def process_memory(pid: int) -> dict[str, int]:
    return read_metrics(
        Path(f"/proc/{pid}/smaps_rollup"),
        {"Rss", "Pss", "Shared_Clean", "Shared_Dirty", "Private_Clean", "Private_Dirty"},
    )


def ram_mapping(pid: int, ram_path: Path) -> dict[str, int]:
    records: list[dict[str, int]] = []
    fields: dict[str, int] | None = None
    for line in Path(f"/proc/{pid}/smaps").read_text().splitlines():
        if re.match(r"^[0-9a-f]+-[0-9a-f]+\s", line):
            if fields is not None:
                records.append(fields)
            header = line.split(maxsplit=5)
            fields = {} if len(header) == 6 and header[5] == str(ram_path) else None
            if fields is not None and header[1] != "rw-p":
                raise RuntimeError(f"clone RAM mapping is not writable MAP_PRIVATE: {line}")
        elif fields is not None:
            key, separator, value = line.partition(":")
            if separator and key in {"Pss", "Shared_Clean", "Shared_Dirty", "Private_Clean", "Private_Dirty"}:
                fields[key.lower() + "_kib"] = int(value.strip().split()[0])
    if fields is not None:
        records.append(fields)
    if not records:
        raise RuntimeError(f"no template RAM mapping found for PID {pid}: {ram_path}")
    return {
        key: sum(record.get(key, 0) for record in records)
        for key in ("pss_kib", "shared_clean_kib", "shared_dirty_kib", "private_clean_kib", "private_dirty_kib")
    }


class GuestSerial:
    def __init__(self, path: Path, timeout: float = 15.0):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(0.2)
        deadline = time.monotonic() + timeout
        while True:
            try:
                self.socket.connect(str(path))
                break
            except OSError:
                if time.monotonic() >= deadline:
                    self.socket.close()
                    raise
                time.sleep(0.05)
        self.buffer = b""

    def read_line(self, deadline: float) -> str:
        while b"\n" not in self.buffer:
            if time.monotonic() >= deadline:
                raise TimeoutError("guest serial line timed out")
            try:
                chunk = self.socket.recv(4096)
            except TimeoutError:
                continue
            if not chunk:
                raise RuntimeError("guest serial disconnected")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return line.decode(errors="replace").rstrip("\r")

    def command(self, command: str, expected: str, timeout: float = 20.0) -> str:
        self.socket.sendall((command + "\n").encode())
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = self.read_line(deadline)
            if not line.startswith("DENSEVM "):
                continue
            reply = line.removeprefix("DENSEVM ")
            if reply.startswith("ERROR"):
                raise RuntimeError(f"guest command {command!r} failed: {reply}")
            if reply.startswith(expected):
                return reply
        raise TimeoutError(f"guest serial did not report {expected!r}")

    def close(self) -> None:
        self.socket.close()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def start_clone(binary: Path, manifest: Path, state_dir: Path, name: str) -> dict[str, Any]:
    return command_json(
        [str(binary), "clone", "start", name, "--template", str(manifest), "--state-dir", str(state_dir)]
    )


def stop_clone(binary: Path, state_dir: Path, name: str) -> None:
    subprocess.run(
        [str(binary), "clone", "stop", name, "--state-dir", str(state_dir)],
        check=True,
        capture_output=True,
        text=True,
    )


def run(binary: Path, manifest_path: Path, state_dir: Path, output_path: Path) -> dict[str, Any]:
    manifest_path = manifest_path.resolve()
    state_dir = state_dir.resolve()
    output_path = output_path.resolve()
    if output_path.exists():
        raise ValueError(f"output already exists: {output_path}")
    if state_dir.exists() and any((state_dir / name).exists() for name in ("cow-a", "cow-b")):
        raise ValueError("state directory already contains this experiment's clone IDs")

    manifest = json.loads(manifest_path.read_text())
    template_ram = manifest_path.parent / manifest["ram"]["path"]
    template_disk = manifest_path.parent / manifest["disk"]["path"]
    template_hashes_before = {"ram": sha256(template_ram), "disk": sha256(template_disk)}
    output_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    clones: list[dict[str, Any]] = []
    serials: dict[str, GuestSerial] = {}
    try:
        for name in ("cow-a", "cow-b"):
            clone = start_clone(binary, manifest_path, state_dir, name)
            clones.append(clone)
            serials[name] = GuestSerial(state_dir / name / f"{name}.serial.sock")

        for field in ("mac", "machine_id"):
            values = [clone[field] for clone in clones]
            if len(set(values)) != len(values):
                raise RuntimeError(f"clones received duplicate {field} values: {values}")

        baseline = {
            name: {
                "ram": serials[name].command("ram-read", "RAM "),
                "disk": serials[name].command("disk-read", "DISK "),
            }
            for name in ("cow-a", "cow-b")
        }
        if baseline["cow-a"] != baseline["cow-b"]:
            raise RuntimeError(f"clones did not restore equivalent guest state: {baseline}")

        def snapshot() -> dict[str, Any]:
            by_name = {clone["id"]: clone for clone in clones}
            return {
                name: {
                    "process": process_memory(by_name[name]["qemu_pid"]),
                    "template_ram_mapping": ram_mapping(by_name[name]["qemu_pid"], template_ram),
                    "overlay_allocated_bytes": (
                        state_dir / name / "disk.qcow2"
                    ).stat().st_blocks * 512,
                }
                for name in ("cow-a", "cow-b")
            }

        before = snapshot()
        if sum(
            value["template_ram_mapping"]["shared_clean_kib"]
            + value["template_ram_mapping"]["shared_dirty_kib"]
            for value in before.values()
        ) == 0:
            raise RuntimeError(f"clones have no shared template RAM pages: {before}")

        divergence = []
        probe_pages = PROBE_REGION_MIB * 1024 * 1024 // PAGE_SIZE
        for percent in (0, 5, 10, 25, 50, 100):
            pages = probe_pages * percent // 100
            if pages:
                serials["cow-a"].command(
                    f"ram-write-pages {pages} 173", f"RAM-WROTE-PAGES {pages} 173"
                )
            ram_a = serials["cow-a"].command("ram-read", "RAM ")
            ram_b = serials["cow-b"].command("ram-read", "RAM ")
            expected_a = "RAM 173" if pages else baseline["cow-a"]["ram"]
            if ram_a != expected_a:
                raise RuntimeError(f"cow-a did not observe its RAM write at {percent}% divergence: {ram_a}")
            if ram_b != baseline["cow-b"]["ram"]:
                raise RuntimeError(f"RAM write leaked from cow-a to cow-b at {percent}% divergence")
            divergence.append(
                {
                    "percent_of_guest_probe_region": percent,
                    "pages": pages,
                    "guest_ram_read": {"cow-a": ram_a, "cow-b": ram_b},
                    "snapshot": snapshot(),
                }
            )

        overlay_before_disk_write = {
            name: (state_dir / name / "disk.qcow2").stat().st_blocks * 512
            for name in ("cow-a", "cow-b")
        }
        serials["cow-a"].command("disk-write 211", "DISK-WROTE 211")
        disk_a = serials["cow-a"].command("disk-read", "DISK ")
        disk_b = serials["cow-b"].command("disk-read", "DISK ")
        if disk_a != "DISK 211" or disk_b != baseline["cow-b"]["disk"]:
            raise RuntimeError(f"disk write was not private: A={disk_a}, B={disk_b}")
        overlay_after_disk_write = {
            name: (state_dir / name / "disk.qcow2").stat().st_blocks * 512
            for name in ("cow-a", "cow-b")
        }

        stop_clone(binary, state_dir, "cow-a")
        serials["cow-a"].close()
        del serials["cow-a"]
        survivor = serials["cow-b"].command("disk-read", "DISK ")
        if survivor != baseline["cow-b"]["disk"]:
            raise RuntimeError("cow-b changed after cow-a stopped")

        template_hashes_after = {"ram": sha256(template_ram), "disk": sha256(template_disk)}
        if template_hashes_after != template_hashes_before:
            raise RuntimeError("clone activity changed an immutable template artifact")
        report = {
            "experiment": "densevm-live-clone-cow",
            "status": "verified",
            "host": {
                "kernel": os.uname().release,
                "page_size_bytes": PAGE_SIZE,
                "qemu_version": subprocess.run(
                    ["qemu-system-x86_64", "--version"], capture_output=True, text=True, check=True
                ).stdout.splitlines()[0],
            },
            "template": {
                "manifest": str(manifest_path),
                "generation_id": manifest["generation_id"],
                "logical_ram_bytes": manifest["ram"]["logical_size_bytes"],
                "ram": str(template_ram),
                "disk": str(template_disk),
                "sha256_before": template_hashes_before,
                "sha256_after": template_hashes_after,
            },
            "clone_identity": {
                "unique_mac_addresses": len({clone["mac"] for clone in clones}) == len(clones),
                "unique_machine_ids": len({clone["machine_id"] for clone in clones}) == len(clones),
            },
            "clones_at_start": clones,
            "guest_baseline": baseline,
            "shared_pages_before_divergence": before,
            "divergence": divergence,
            "disk_isolation": {
                "cow_a_after_write": disk_a,
                "cow_b_after_a_write": disk_b,
                "overlay_allocated_bytes_before_write": overlay_before_disk_write,
                "overlay_allocated_bytes_after_write": overlay_after_disk_write,
            },
            "survivor_after_sibling_stop": survivor,
            "accounting_note": "PSS and smaps mappings are process metrics; they exclude KVM metadata and host page tables.",
        }
        output_path.write_text(json.dumps(report, indent=2) + "\n")
        return report
    finally:
        for serial in serials.values():
            serial.close()
        for clone in clones:
            if clone["id"] != "cow-a" or clone["state"] == "running":
                try:
                    stop_clone(binary, state_dir, clone["id"])
                except Exception:
                    pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--densevm", type=Path, default=Path("target/debug/densevm"))
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    arguments = parser.parse_args()
    report = run(arguments.densevm.resolve(), arguments.template, arguments.state_dir, arguments.out)
    print(json.dumps({"status": report["status"], "result": str(arguments.out.resolve())}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"runtime-clone probe failed: {error}", file=sys.stderr)
        raise SystemExit(1)
