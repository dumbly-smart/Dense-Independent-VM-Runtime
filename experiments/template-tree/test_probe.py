import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).parent))
from probe import (
    build_child_generation_manifest,
    extend_backing_chain,
    load_template_artifacts,
    parse_ram_mapping,
    publish_atomically,
    validate_generation_manifest,
)


class RamMappingTests(unittest.TestCase):
    def test_sums_matching_ram_vmas_and_ignores_unrelated_mappings(self) -> None:
        sample = """\
00400000-00401000 r--p 00000000 08:01 1 /tmp/template-memory.img
Size: 4 kB
Rss: 4 kB
Pss: 3 kB
Shared_Clean: 0 kB
Shared_Dirty: 4 kB
Private_Clean: 0 kB
Private_Dirty: 0 kB
00500000-00501000 rw-p 00000000 00:00 0 [heap]
Size: 4 kB
Rss: 4 kB
Pss: 4 kB
Shared_Clean: 0 kB
Shared_Dirty: 0 kB
Private_Clean: 0 kB
Private_Dirty: 4 kB
00600000-00601000 rw-p 00001000 08:01 1 /tmp/template-memory.img
Size: 4 kB
Rss: 4 kB
Pss: 2 kB
Shared_Clean: 0 kB
Shared_Dirty: 0 kB
Private_Clean: 0 kB
Private_Dirty: 4 kB
"""
        self.assertEqual(
            parse_ram_mapping(sample, "/tmp/template-memory.img"),
            {
                "pss_kib": 5,
                "shared_clean_kib": 0,
                "shared_dirty_kib": 4,
                "private_clean_kib": 0,
                "private_dirty_kib": 4,
            },
        )

    def test_requires_a_matching_ram_mapping(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "no smaps VMA"):
            parse_ram_mapping("00400000-00401000 rw-p 0 00:00 0 [heap]\n", "/tmp/missing.img")


class GenerationManifestTests(unittest.TestCase):
    def test_manifest_records_hashed_artifacts_and_parent_disk_provenance(self) -> None:
        import json
        import tempfile
        from hashlib import sha256

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            output = root / "tree"
            source.mkdir()
            output.mkdir()
            files = {
                source / "result.json": b"source result",
                source / "memory.img": b"root ram",
                source / "machine-state": b"root state",
                source / "base.qcow2": b"root disk",
                output / "child-memory.img": b"child ram",
                output / "child-machine-state": b"child state",
                output / "parent.qcow2": b"parent disk",
            }
            for path, content in files.items():
                path.write_bytes(content)

            manifest = build_child_generation_manifest(
                source,
                output,
                memory_size_bytes=256 * 1024 * 1024,
                qemu_version="QEMU emulator version 11.1.2",
                qemu_command=["-machine", "q35", "-device", "virtio-net-pci"],
            )
            written = json.loads((output / "generation.json").read_text())

            self.assertEqual(manifest, written)
            self.assertEqual(manifest["schema_version"], 3)
            child_artifacts = [
                sha256(b"child ram").hexdigest(),
                sha256(b"child state").hexdigest(),
                sha256(b"parent disk").hexdigest(),
            ]
            expected_generation_id = sha256(
                json.dumps(child_artifacts, separators=(",", ":")).encode()
            ).hexdigest()
            self.assertEqual(manifest["generation_id"], f"sha256:{expected_generation_id}")
            self.assertEqual(manifest["parent"]["artifacts"]["result"]["sha256"], sha256(b"source result").hexdigest())
            self.assertEqual(manifest["parent"]["artifacts"]["ram"]["sha256"], sha256(b"root ram").hexdigest())
            self.assertEqual(manifest["parent"]["artifacts"]["machine_state"]["sha256"], sha256(b"root state").hexdigest())
            self.assertEqual(manifest["ram"]["path"], "child-memory.img")
            self.assertEqual(manifest["ram"]["sha256"], sha256(b"child ram").hexdigest())
            self.assertEqual(manifest["ram"]["logical_size_bytes"], 256 * 1024 * 1024)
            self.assertEqual(manifest["machine_state"]["sha256"], sha256(b"child state").hexdigest())
            self.assertEqual(manifest["disk"]["backing_path"], str((source / "base.qcow2").resolve()))
            self.assertEqual(manifest["disk"]["sha256"], sha256(b"parent disk").hexdigest())
            self.assertEqual(manifest["qemu_command"], ["-machine", "q35", "-device", "virtio-net-pci"])
            validation = validate_generation_manifest(output / "generation.json")
            self.assertEqual(validation["status"], "verified")
            self.assertEqual(validation["generation_id"], manifest["generation_id"])
            self.assertEqual(validation["checked_artifacts"], 7)

            (output / "child-memory.img").write_bytes(b"modified ram")
            with self.assertRaisesRegex(RuntimeError, "hash mismatch"):
                validate_generation_manifest(output / "generation.json")

    def test_child_generation_manifest_tracks_parent_generation_artifact_names(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "generation-a"
            output = root / "generation-b"
            source.mkdir()
            output.mkdir()
            (source / "result.json").write_text(json.dumps({"guest": {}}))
            (source / "generation.json").write_text(
                json.dumps(
                    {
                        "schema_version": 2,
                        "ram": {"path": "child-memory.img"},
                        "machine_state": {"path": "child-machine-state"},
                        "disk": {"path": "parent.qcow2"},
                    }
                )
            )
            for path, contents in (
                (source / "child-memory.img", b"generation A RAM"),
                (source / "child-machine-state", b"generation A state"),
                (source / "parent.qcow2", b"generation A disk"),
                (output / "child-memory.img", b"generation B RAM"),
                (output / "child-machine-state", b"generation B state"),
                (output / "parent.qcow2", b"generation B disk"),
            ):
                path.write_bytes(contents)

            manifest = build_child_generation_manifest(
                source,
                output,
                memory_size_bytes=256 * 1024 * 1024,
                qemu_version="QEMU emulator version 11.1.2",
                qemu_command=["-machine", "q35"],
            )

            self.assertEqual(manifest["schema_version"], 3)
            self.assertEqual(manifest["parent"]["artifacts"]["ram"]["path"], "child-memory.img")
            self.assertEqual(manifest["parent"]["artifacts"]["machine_state"]["path"], "child-machine-state")
            self.assertEqual(manifest["parent"]["artifacts"]["disk"]["path"], "parent.qcow2")
            self.assertEqual(manifest["disk"]["backing_path"], str((source / "parent.qcow2").resolve()))
            self.assertEqual(validate_generation_manifest(output / "generation.json")["status"], "verified")
            (output / "result.json").write_text(
                json.dumps(
                    {
                        "guest": {"kind": "synthetic-initramfs"},
                        "source_template_qemu_command": ["-machine", "q35"],
                        "parent_ram": "RAM 73",
                        "parent_disk": "DISK 73",
                    }
                )
            )
            loaded = load_template_artifacts(output)
            self.assertEqual(loaded["ram"], output / "child-memory.img")
            self.assertEqual(loaded["machine_state"], output / "child-machine-state")
            self.assertEqual(loaded["disk"], output / "parent.qcow2")
            self.assertEqual(loaded["memory_size_bytes"], 256 * 1024 * 1024)
            self.assertEqual(loaded["qemu_command"], ["-machine", "q35"])
            self.assertEqual(loaded["initial_ram"], "RAM 73")
            self.assertEqual(loaded["initial_disk"], "DISK 73")


class DiskLineageTests(unittest.TestCase):
    def test_new_overlay_extends_the_complete_parent_backing_chain(self) -> None:
        overlay = Path("generation-c/parent.qcow2")
        parent_chain = [
            Path("generation-b/parent.qcow2"),
            Path("generation-a/parent.qcow2"),
            Path("root/base.qcow2"),
        ]

        self.assertEqual(extend_backing_chain(overlay, parent_chain), [overlay, *parent_chain])


class AtomicPublicationTests(unittest.TestCase):
    def test_staging_keeps_qmp_socket_path_under_linux_unix_limit(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / ("o" * 80)
            measured: list[int] = []

            def build(staging: Path) -> dict[str, str]:
                measured.append(len(str(staging / "child-template.serial.sock").encode()))
                staging.mkdir()
                return {}

            publish_atomically(output, build)

            self.assertLess(measured[0], 108)

    def test_publishes_complete_directory_and_rewrites_staging_paths(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "template"

            def build(staging: Path) -> dict[str, str]:
                staging.mkdir()
                (staging / "result.json").write_text(
                    json.dumps({"artifact": str(staging / "memory.img")})
                )
                (staging / "memory.img").write_bytes(b"RAM")
                return {"artifact": str(staging / "memory.img")}

            result = publish_atomically(output, build)

            self.assertEqual(result["artifact"], str(output / "memory.img"))
            self.assertEqual(json.loads((output / "result.json").read_text())["artifact"], str(output / "memory.img"))
            self.assertTrue((output / "memory.img").is_file())
            self.assertEqual(list(Path(temporary).glob(".dvm-*")), [])

    def test_failed_build_does_not_publish_a_partial_directory(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "template"

            def fail(staging: Path) -> dict[str, str]:
                staging.mkdir()
                (staging / "partial").write_text("incomplete")
                raise RuntimeError("simulated experiment failure")

            with self.assertRaisesRegex(RuntimeError, "simulated experiment failure"):
                publish_atomically(output, fail)

            self.assertFalse(output.exists())
            self.assertEqual(list(Path(temporary).glob(".dvm-*")), [])


if __name__ == "__main__":
    unittest.main()
