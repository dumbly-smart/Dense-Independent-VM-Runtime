#!/usr/bin/env python3
"""Two-generation QEMU RAM/disk template COW experiment.

The input directory is a completed DenseVM probe output. This intentionally
small experiment snapshots one private clone into a second file-backed RAM
template, then restores two descendants from it.
"""

import argparse
import hashlib
import json
import os
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable


class QMP:
    def __init__(self, path: Path):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(2)
        deadline = time.monotonic() + 15
        while True:
            try:
                self.socket.connect(str(path))
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
        self.reader = self.socket.makefile("r", encoding="utf-8")
        greeting = json.loads(self.reader.readline())
        if "QMP" not in greeting:
            raise RuntimeError(f"invalid QMP greeting: {greeting}")
        self.next_id = 1
        self.execute("qmp_capabilities")

    def execute(self, command: str, arguments: dict[str, Any] | None = None) -> Any:
        request_id = self.next_id
        self.next_id += 1
        message: dict[str, Any] = {"execute": command, "id": request_id}
        if arguments is not None:
            message["arguments"] = arguments
        self.socket.sendall((json.dumps(message) + "\r\n").encode())
        while True:
            line = self.reader.readline()
            if not line:
                raise RuntimeError(f"QMP closed while waiting for {command}")
            response = json.loads(line)
            if "event" in response:
                continue
            if response.get("id") != request_id:
                raise RuntimeError(f"QMP reply mismatch: {response}")
            if "error" in response:
                raise RuntimeError(f"QMP {command}: {response['error']}")
            return response.get("return")

    def close(self) -> None:
        self.reader.close()
        self.socket.close()


class GuestSerial:
    def __init__(self, path: Path):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(1)
        deadline = time.monotonic() + 15
        while True:
            try:
                self.socket.connect(str(path))
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
        self.buffer = b""

    def read_line(self, timeout: float) -> str:
        deadline = time.monotonic() + timeout
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

    def command(self, command: str, expected: str) -> str:
        self.socket.sendall((command + "\n").encode())
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            line = self.read_line(max(0.1, deadline - time.monotonic()))
            result = line.removeprefix("DENSEVM ")
            if result.startswith("ERROR"):
                raise RuntimeError(f"guest command {command!r} returned {result!r}")
            if line.startswith("DENSEVM ") and result.startswith(expected):
                return result
        raise TimeoutError(f"guest serial did not report {expected!r}")

    def close(self) -> None:
        self.socket.close()


class VM:
    def __init__(self, process: subprocess.Popen[bytes], qmp: QMP, serial: GuestSerial, login: socket.socket | None):
        self.process = process
        self.qmp = qmp
        self.serial = serial
        self.login = login

    def close(self) -> None:
        try:
            self.qmp.execute("quit")
        except (OSError, RuntimeError, TimeoutError):
            pass
        self.qmp.close()
        self.serial.close()
        if self.login is not None:
            self.login.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def wait_migration(qmp: QMP, timeout: float = 60) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        state = qmp.execute("query-migrate")
        status = state.get("status")
        if status == "completed":
            return state
        if status in ("failed", "cancelled"):
            raise RuntimeError(f"QEMU migration {status}: {state}")
        time.sleep(0.1)
    raise TimeoutError(f"QEMU migration timed out: {qmp.execute('query-migrate')}")


def launch(
    base_command: list[str],
    name: str,
    memory: Path,
    disk: Path,
    out: Path,
    *,
    memory_size: str,
    shared_writable_ram: bool,
) -> tuple[subprocess.Popen[bytes], QMP, GuestSerial, socket.socket | None]:
    command = list(base_command)
    memory_index = next(
        i for i, value in enumerate(command) if value.startswith("memory-backend-file,id=ram,")
    )
    source_memory_options = command[memory_index].split(",")
    merge = "on" if "merge=on" in source_memory_options else "off"
    if shared_writable_ram:
        backend = f"memory-backend-file,id=ram,mem-path={memory},size={memory_size},share=on,readonly=off,merge={merge}"
    else:
        backend = f"memory-backend-file,id=ram,mem-path={memory},size={memory_size},share=off,readonly=on,rom=off,merge={merge}"
    command[memory_index] = backend
    drive_index = command.index("-drive") + 1
    command[drive_index] = f"if=none,id=disk,file={disk},format=qcow2"
    for index, value in enumerate(command):
        if value.startswith("socket,id=serial,path="):
            command[index] = f"socket,id=serial,path={out / (name + '.serial.sock')},server=on,wait=off"
        elif value.startswith("socket,id=login,path="):
            command[index] = f"socket,id=login,path={out / (name + '.login.sock')},server=on,wait=on"
        elif value.startswith("unix:") and value.endswith(".qmp.sock,server=on,wait=off"):
            command[index] = f"unix:{out / (name + '.qmp.sock')},server=on,wait=off"
    command = [value for index, value in enumerate(command) if not (value == "-incoming" and index + 1 < len(command) and command[index + 1] == "defer") and not (index > 0 and command[index - 1] == "-incoming" and value == "defer")]
    command.extend(["-incoming", "defer"])
    log = (out / f"{name}.qemu.log").open("wb")
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log)
    try:
        login = None
        if any(value.startswith("socket,id=login,path=") for value in command):
            login = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            login.settimeout(1)
            login_path = out / f"{name}.login.sock"
            deadline = time.monotonic() + 15
            while True:
                try:
                    login.connect(str(login_path))
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.05)
        qmp = QMP(out / f"{name}.qmp.sock")
        serial = GuestSerial(out / f"{name}.serial.sock")
        return process, qmp, serial, login
    except Exception:
        process.kill()
        process.wait()
        raise


def restore(qmp: QMP, state: Path, *, ignore_shared: bool) -> dict[str, Any]:
    capabilities = qmp.execute("query-migrate-capabilities")
    if ignore_shared:
        if not any(item.get("capability") == "x-ignore-shared" for item in capabilities):
            raise RuntimeError("QEMU lacks x-ignore-shared")
        qmp.execute(
            "migrate-set-capabilities",
            {"capabilities": [{"capability": "x-ignore-shared", "state": True}]},
        )
    state_result = qmp.execute("migrate-incoming", {"uri": f"file:{state}"})
    migration = wait_migration(qmp)
    if qmp.execute("query-status").get("status") == "paused":
        qmp.execute("cont")
    return {"start": state_result, "result": migration}


def save_state(qmp: QMP, target: Path, *, ignore_shared: bool) -> dict[str, Any]:
    qmp.execute("stop")
    deadline = time.monotonic() + 5
    while qmp.execute("query-status").get("status") != "paused":
        if time.monotonic() > deadline:
            raise TimeoutError("QEMU did not pause before snapshot")
        time.sleep(0.05)
    if ignore_shared:
        qmp.execute(
            "migrate-set-capabilities",
            {"capabilities": [{"capability": "x-ignore-shared", "state": True}]},
        )
    qmp.execute("migrate", {"uri": f"file:{target}"})
    return wait_migration(qmp)


def make_overlay(path: Path, backing: Path) -> None:
    subprocess.run(
        ["qemu-img", "create", "-f", "qcow2", "-F", "qcow2", "-b", str(backing), str(path)],
        check=True,
        capture_output=True,
        text=True,
    )


def check_disk_chain(path: Path, expected: list[Path]) -> dict[str, Any]:
    info = subprocess.run(
        ["qemu-img", "info", "--backing-chain", "--output=json", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    chain_data = json.loads(info.stdout)
    chain = [Path(image["filename"]).resolve() for image in chain_data]
    expected_paths = [image.resolve() for image in expected]
    if chain != expected_paths:
        raise RuntimeError(f"unexpected qcow2 backing chain for {path}: {chain} != {expected_paths}")
    check = subprocess.run(
        ["qemu-img", "check", str(path)], check=True, capture_output=True, text=True
    )
    return {
        "backing_chain": [str(image) for image in chain],
        "check_output": check.stdout.strip(),
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_child_generation_manifest(
    source_dir: Path,
    output_dir: Path,
    *,
    memory_size_bytes: int,
    qemu_version: str,
    qemu_command: list[str],
) -> dict[str, Any]:
    source_dir = source_dir.resolve()
    output_dir = output_dir.resolve()
    source_result = source_dir / "result.json"
    source_ram = source_dir / "memory.img"
    source_state = source_dir / "machine-state"
    source_disk = source_dir / "base.qcow2"
    child_ram = output_dir / "child-memory.img"
    child_state = output_dir / "child-machine-state"
    child_disk = output_dir / "parent.qcow2"
    required = (source_result, source_ram, source_state, source_disk, child_ram, child_state, child_disk)
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise ValueError(f"cannot write generation manifest; artifacts are missing: {missing}")

    child_artifact_hashes = [sha256(child_ram), sha256(child_state), sha256(child_disk)]
    generation_digest = hashlib.sha256(
        json.dumps(child_artifact_hashes, separators=(",", ":")).encode()
    ).hexdigest()
    manifest = {
        "schema_version": 2,
        "generation_id": f"sha256:{generation_digest}",
        "parent": {
            "template_dir": str(source_dir),
            "result_sha256": sha256(source_result),
            "ram_sha256": sha256(source_ram),
            "machine_state_sha256": sha256(source_state),
            "base_disk_sha256": sha256(source_disk),
        },
        "qemu_version": qemu_version,
        "qemu_command": qemu_command,
        "ram": {
            "path": child_ram.name,
            "sha256": child_artifact_hashes[0],
            "logical_size_bytes": memory_size_bytes,
            "readonly_for_descendants": True,
        },
        "machine_state": {
            "path": child_state.name,
            "sha256": child_artifact_hashes[1],
        },
        "disk": {
            "path": child_disk.name,
            "sha256": child_artifact_hashes[2],
            "readonly_for_descendants": True,
            "backing_path": str(source_disk),
            "backing_sha256": sha256(source_disk),
        },
    }
    (output_dir / "generation.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def validate_generation_manifest(manifest_path: Path) -> dict[str, Any]:
    manifest_path = manifest_path.resolve()
    try:
        manifest = json.loads(manifest_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read generation manifest {manifest_path}: {error}") from error
    if not isinstance(manifest, dict) or manifest.get("schema_version") not in (1, 2):
        raise ValueError("unsupported generation manifest schema")

    parent = manifest.get("parent")
    if not isinstance(parent, dict) or not isinstance(parent.get("template_dir"), str):
        raise ValueError("generation manifest has no parent template path")
    parent_dir = Path(parent["template_dir"])
    if not parent_dir.is_absolute():
        raise ValueError("parent template path must be absolute")

    def checked_hash(path: Path, expected: Any, label: str) -> None:
        if not isinstance(expected, str) or len(expected) != 64:
            raise ValueError(f"generation manifest has invalid {label} hash")
        if not path.is_file():
            raise RuntimeError(f"generation artifact is missing: {path}")
        actual = sha256(path)
        if actual != expected:
            raise RuntimeError(f"generation artifact hash mismatch: {label}: {path}")

    def local_artifact(section: str) -> tuple[Path, dict[str, Any]]:
        value = manifest.get(section)
        if not isinstance(value, dict) or not isinstance(value.get("path"), str):
            raise ValueError(f"generation manifest has invalid {section} artifact")
        relative = Path(value["path"])
        if relative.is_absolute() or relative.name != value["path"] or value["path"] in (".", ".."):
            raise ValueError(f"generation artifact path must be a local filename: {value['path']!r}")
        return manifest_path.parent / relative, value

    checked = 0
    parent_artifacts = {
        "result.json": "result_sha256",
        "memory.img": "ram_sha256",
        "machine-state": "machine_state_sha256",
        "base.qcow2": "base_disk_sha256",
    }
    for filename, key in parent_artifacts.items():
        expected = parent.get(key)
        if expected is None and manifest["schema_version"] == 1 and key in ("ram_sha256", "machine_state_sha256"):
            continue
        checked_hash(parent_dir / filename, expected, f"parent.{key}")
        checked += 1

    ram_path, ram = local_artifact("ram")
    state_path, state = local_artifact("machine_state")
    disk_path, disk = local_artifact("disk")
    for path, value, label in (
        (ram_path, ram, "ram"),
        (state_path, state, "machine_state"),
        (disk_path, disk, "disk"),
    ):
        checked_hash(path, value.get("sha256"), label)
        checked += 1

    backing_path = disk.get("backing_path")
    if not isinstance(backing_path, str) or Path(backing_path).resolve() != (parent_dir / "base.qcow2").resolve():
        raise ValueError("disk backing path does not match the recorded parent base image")
    if disk.get("backing_sha256") != parent.get("base_disk_sha256"):
        raise ValueError("disk backing hash does not match the parent base image hash")

    if manifest["schema_version"] == 2:
        artifact_hashes = [ram["sha256"], state["sha256"], disk["sha256"]]
        expected_id = hashlib.sha256(json.dumps(artifact_hashes, separators=(",", ":")).encode()).hexdigest()
        if manifest.get("generation_id") != f"sha256:{expected_id}":
            raise RuntimeError("generation ID does not match child artifact hashes")
    return {
        "status": "verified",
        "generation_id": manifest.get("generation_id"),
        "schema_version": manifest["schema_version"],
        "checked_artifacts": checked,
    }


def _rewrite_staging_paths(value: Any, staging: str, final: str) -> Any:
    if isinstance(value, str):
        return value.replace(staging, final)
    if isinstance(value, list):
        return [_rewrite_staging_paths(item, staging, final) for item in value]
    if isinstance(value, dict):
        return {key: _rewrite_staging_paths(item, staging, final) for key, item in value.items()}
    return value


def publish_atomically(
    output: Path,
    build: Callable[[Path], dict[str, Any]],
) -> dict[str, Any]:
    output = output.resolve()
    if output.exists():
        raise ValueError(f"output path already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".dvm-", dir=output.parent))
    staging.rmdir()
    try:
        result = build(staging)
        for json_path in staging.glob("*.json"):
            contents = json.loads(json_path.read_text())
            contents = _rewrite_staging_paths(contents, str(staging), str(output))
            json_path.write_text(json.dumps(contents, indent=2) + "\n")
        if output.exists():
            raise ValueError(f"output path appeared during experiment: {output}")
        os.rename(staging, output)
        return _rewrite_staging_paths(result, str(staging), str(output))
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    finally:
        if staging.exists():
            shutil.rmtree(staging, ignore_errors=True)


def parse_ram_mapping(text: str, memory_path: str) -> dict[str, int]:
    records: list[dict[str, int]] = []
    fields: dict[str, int] | None = None
    header = re.compile(r"^[0-9a-f]+-[0-9a-f]+\s")
    tracked = {"Pss", "Shared_Clean", "Shared_Dirty", "Private_Clean", "Private_Dirty"}
    for line in text.splitlines():
        if header.match(line):
            if fields is not None:
                records.append(fields)
            fields = {} if memory_path in line else None
            continue
        if fields is not None:
            key, separator, value = line.partition(":")
            if separator and key in tracked:
                fields[key.lower() + "_kib"] = int(value.strip().split()[0])
    if fields is not None:
        records.append(fields)
    if not records:
        raise RuntimeError(f"no smaps VMA for {memory_path}")
    return {key: sum(record.get(key, 0) for record in records) for key in tracked_keys()}


def ram_mapping(pid: int, memory: Path) -> dict[str, int]:
    text = Path(f"/proc/{pid}/smaps").read_text()
    return parse_ram_mapping(text, str(memory))


def tracked_keys() -> tuple[str, ...]:
    return ("pss_kib", "shared_clean_kib", "shared_dirty_kib", "private_clean_kib", "private_dirty_kib")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def prepare_identity(serial: GuestSerial) -> dict[str, str]:
    identity = secrets.token_hex(16)
    vmgenid = serial.command("vmgenid-reaction-read", "VMGENID-REACTION ")
    serial.command(f"entropy-add {secrets.token_hex(32)}", "RNG-RESEEDED")
    serial.command(f"identity-set {identity}", "IDENTITY-SET ")
    return {
        "identity": serial.command("identity-read", "IDENTITY "),
        "ssh_identity": serial.command("ssh-identity-read", "SSH-IDENTITY "),
        "vmgenid_reaction": vmgenid,
    }


def _run_in_directory(template_dir: Path, out: Path) -> dict[str, Any]:
    metadata = json.loads((template_dir / "result.json").read_text())
    template_ram = template_dir / "memory.img"
    base_disk = template_dir / "base.qcow2"
    machine_state = template_dir / "machine-state"
    qemu_command = metadata["clones"]["a"]["qemu_command"]
    template_qemu_command = metadata["template"]["qemu_command"]
    memory_backend = next(
        value for value in qemu_command if value.startswith("memory-backend-file,id=ram,")
    )
    memory_size = next(
        (item.removeprefix("size=") for item in memory_backend.split(",") if item.startswith("size=")),
        None,
    )
    memory_size_bytes = metadata["template"].get("memory_file_bytes")
    if not isinstance(memory_size, str) or type(memory_size_bytes) is not int:
        raise ValueError("template result does not describe its RAM backend size")
    for path in (template_ram, base_disk, machine_state):
        if not path.is_file():
            raise ValueError(f"template artifact is missing: {path}")
    if out.exists():
        raise ValueError(f"output path already exists: {out}")
    disk_free = shutil.disk_usage(out.parent).free
    if disk_free < 1024 * 1024 * 1024:
        raise ValueError("template-tree experiment requires 1 GiB free disk space")
    mem_available = next(
        int(line.split()[1]) for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:")
    )
    if mem_available < 2 * 1024 * 1024:
        raise ValueError("template-tree experiment requires 2 GiB MemAvailable")

    out.mkdir(mode=0o700, parents=True)
    out = out.resolve()
    original_hashes = {str(path): sha256(path) for path in (template_ram, base_disk, machine_state)}
    parent_disk = out / "parent.qcow2"
    make_overlay(parent_disk, base_disk)
    parent_process, parent_qmp, parent_serial, parent_login = launch(
        qemu_command, "parent", template_ram, parent_disk, out,
        memory_size=memory_size, shared_writable_ram=False
    )
    parent = VM(parent_process, parent_qmp, parent_serial, parent_login)
    child_full_state = out / "child-full.migration"
    try:
        parent_restore = restore(parent.qmp, machine_state, ignore_shared=True)
        parent_identity = prepare_identity(parent.serial)
        initial_ram = parent.serial.command("ram-read", "RAM ")
        initial_disk = parent.serial.command("disk-read", "DISK ")
        require(initial_ram == "RAM 90", f"parent initial RAM differs: {initial_ram!r}")
        require(initial_disk.endswith("0"), f"parent initial disk differs: {initial_disk!r}")
        parent.serial.command("ram-write 73", "RAM-WROTE 73")
        parent.serial.command("disk-write 73", "DISK-WROTE 73")
        parent_expected_ram = parent.serial.command("ram-read", "RAM ")
        parent_expected_disk = parent.serial.command("disk-read", "DISK ")
        parent.qmp.execute(
            "migrate-set-capabilities",
            {"capabilities": [{"capability": "x-ignore-shared", "state": False}]},
        )
        parent.qmp.execute("stop")
        deadline = time.monotonic() + 5
        while parent.qmp.execute("query-status").get("status") != "paused":
            if time.monotonic() > deadline:
                raise TimeoutError("parent clone did not pause")
            time.sleep(0.05)
        parent.qmp.execute("migrate", {"uri": f"file:{child_full_state}"})
        parent_migration = wait_migration(parent.qmp)
    finally:
        parent.close()

    child_ram = out / "child-memory.img"
    with child_ram.open("wb") as stream:
        stream.truncate(memory_size_bytes)
    child_template_process, child_template_qmp, child_template_serial, child_template_login = launch(
        qemu_command, "child-template", child_ram, parent_disk, out,
        memory_size=memory_size, shared_writable_ram=True
    )
    child_template = VM(child_template_process, child_template_qmp, child_template_serial, child_template_login)
    try:
        full_restore = restore(child_template.qmp, child_full_state, ignore_shared=False)
        require(child_template.serial.command("ram-read", "RAM ") == parent_expected_ram, "RAM did not migrate into child template")
        require(child_template.serial.command("disk-read", "DISK ") == parent_expected_disk, "disk state did not carry into child template")
        child_state = out / "child-machine-state"
        child_snapshot = save_state(child_template.qmp, child_state, ignore_shared=True)
    finally:
        child_template.close()
    child_full_state.unlink(missing_ok=True)
    os.chmod(child_ram, 0o444)
    os.chmod(parent_disk, 0o444)

    descendants: list[VM] = []
    descendant_restore: list[dict[str, Any]] = []
    descendant_identities: list[dict[str, str]] = []
    descendant_mappings: list[dict[str, int]] = []
    try:
        for index in range(2):
            name = f"descendant-{index}"
            disk = out / f"{name}.qcow2"
            make_overlay(disk, parent_disk)
            process, qmp, serial, login = launch(
                qemu_command, name, child_ram, disk, out,
                memory_size=memory_size, shared_writable_ram=False
            )
            vm = VM(process, qmp, serial, login)
            descendants.append(vm)
            descendant_restore.append(restore(vm.qmp, child_state, ignore_shared=True))
            descendant_identities.append(prepare_identity(vm.serial))
            require(vm.serial.command("ram-read", "RAM ") == parent_expected_ram, f"{name} did not inherit parent RAM")
            require(vm.serial.command("disk-read", "DISK ") == parent_expected_disk, f"{name} did not inherit parent disk")
        descendant_mappings = [ram_mapping(vm.process.pid, child_ram) for vm in descendants]
        require(
            all(mapping["pss_kib"] > 0 for mapping in descendant_mappings)
            and sum(mapping["shared_clean_kib"] + mapping["shared_dirty_kib"] for mapping in descendant_mappings) > 0,
            f"descendants did not demonstrate shared resident template pages: {descendant_mappings}",
        )
        descendants[0].serial.command("ram-write 91", "RAM-WROTE 91")
        descendants[0].serial.command("disk-write 91", "DISK-WROTE 91")
        a_ram = descendants[0].serial.command("ram-read", "RAM ")
        a_disk = descendants[0].serial.command("disk-read", "DISK ")
        b_ram = descendants[1].serial.command("ram-read", "RAM ")
        b_disk = descendants[1].serial.command("disk-read", "DISK ")
        require(a_ram.endswith("91") and a_disk.endswith("91"), "descendant A did not retain private writes")
        require(b_ram == parent_expected_ram and b_disk == parent_expected_disk, "descendant A writes leaked into B")
        after_mappings = [ram_mapping(vm.process.pid, child_ram) for vm in descendants]
        require(
            after_mappings[0]["private_dirty_kib"] > descendant_mappings[0]["private_dirty_kib"] + 16 * 1024,
            f"A's RAM writes did not produce measurable private pages: before={descendant_mappings[0]}, after={after_mappings[0]}",
        )
        identity_values = [parent_identity["identity"], *(item["identity"] for item in descendant_identities)]
        require(len({value.split()[1] for value in identity_values}) == len(identity_values), "machine IDs duplicated across generations")
    finally:
        for vm in descendants:
            vm.close()

    disk_checks = {
        "parent": check_disk_chain(parent_disk, [parent_disk, base_disk]),
        "descendant_0": check_disk_chain(
            out / "descendant-0.qcow2", [out / "descendant-0.qcow2", parent_disk, base_disk]
        ),
        "descendant_1": check_disk_chain(
            out / "descendant-1.qcow2", [out / "descendant-1.qcow2", parent_disk, base_disk]
        ),
    }

    result = {
        "experiment": "two-generation-template-tree-cow",
        "status": "verified",
        "host": {"kernel": os.uname().release, "page_size_bytes": os.sysconf("SC_PAGE_SIZE")},
        "qemu_version": subprocess.run(["qemu-system-x86_64", "--version"], capture_output=True, text=True, check=True).stdout.splitlines()[0],
        "template_source": str(template_dir.resolve()),
        "template_result_sha256": sha256(template_dir / "result.json"),
        "guest": metadata.get("guest"),
        "source_template_qemu_command": template_qemu_command,
        "source_clone_qemu_command": qemu_command,
        "memory_size_bytes": memory_size_bytes,
        "parent_ram": parent_expected_ram,
        "parent_disk": parent_expected_disk,
        "parent_identity": parent_identity,
        "descendant_identities": descendant_identities,
        "identity_unique_across_generations": True,
        "parent_full_migration": parent_migration,
        "child_full_restore": full_restore,
        "child_machine_state_snapshot": child_snapshot,
        "descendant_restores": descendant_restore,
        "qcow2_checks": disk_checks,
        "descendant_a_after_private_write": {"ram": a_ram, "disk": a_disk},
        "descendant_b_after_a_write": {"ram": b_ram, "disk": b_disk},
        "descendant_ram_mapping_before_divergence": descendant_mappings,
        "descendant_ram_mapping_after_divergence": after_mappings,
        "child_ram_sha256": sha256(child_ram),
        "parent_ram_sha256_after": sha256(template_ram),
        "parent_disk_sha256_after": sha256(base_disk),
        "input_hashes_before": original_hashes,
        "input_hashes_unchanged": original_hashes == {str(path): sha256(path) for path in (template_ram, base_disk, machine_state)},
        "allocated_bytes": {path.name: path.stat().st_blocks * 512 for path in (child_ram, parent_disk, out / "descendant-0.qcow2", out / "descendant-1.qcow2")},
    }
    generation = build_child_generation_manifest(
        template_dir,
        out,
        memory_size_bytes=memory_size_bytes,
        qemu_version=result["qemu_version"],
        qemu_command=qemu_command,
    )
    result["generation_manifest"] = {
        "path": "generation.json",
        "sha256": sha256(out / "generation.json"),
        "generation_id": generation["generation_id"],
    }
    result["status"] = "verified" if result["input_hashes_unchanged"] else "failed"
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    if result["status"] != "verified":
        raise RuntimeError("template input changed during experiment")
    return result


def run(template_dir: Path, out: Path) -> dict[str, Any]:
    return publish_atomically(out, lambda staging: _run_in_directory(template_dir, staging))


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template-dir", type=Path, help="completed DenseVM probe output directory")
    parser.add_argument("--out", type=Path, help="new experiment output directory")
    parser.add_argument("--validate-manifest", type=Path, help="verify a saved generation manifest and its artifacts")
    args = parser.parse_args(argv[1:])
    if args.validate_manifest is not None:
        if args.template_dir is not None or args.out is not None:
            parser.error("--validate-manifest cannot be combined with --template-dir or --out")
        try:
            print(json.dumps(validate_generation_manifest(args.validate_manifest), indent=2))
        except (OSError, ValueError, RuntimeError) as error:
            print(f"template-tree validation error: {error}", file=sys.stderr)
            return 1
        return 0
    if args.template_dir is None or args.out is None:
        parser.error("--template-dir and --out are required unless --validate-manifest is used")
    try:
        result = run(args.template_dir, args.out)
    except (OSError, ValueError, RuntimeError, TimeoutError, StopIteration) as error:
        print(f"template-tree error: {error}", file=sys.stderr)
        return 1
    print(args.out / "result.json")
    print(json.dumps({"status": result["status"], "ram_sharing_before": result["descendant_ram_mapping_before_divergence"], "ram_sharing_after": result["descendant_ram_mapping_after_divergence"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
