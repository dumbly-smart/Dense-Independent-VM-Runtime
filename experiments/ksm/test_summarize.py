import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("summarize.py")
sys.path.insert(0, str(SCRIPT.parent))
from summarize import summarize


def sample_result(offset: int, ksm_run: int = 0) -> dict:
    return {
        "experiment": "two-clone-cow-probe",
        "ksm_run": ksm_run,
        "qemu_version": "QEMU emulator version 11.1.2",
        "kernel_version": "7.2.11-test",
        "guest": {"kind": "prepared", "image_sha256": "abc123"},
        "host": {
            "kernel_version": "7.2.11-test",
            "page_size_bytes": 4096,
            "kvm": {"accessible": True},
        },
        "guest_ram_mib": 256,
        "ram_divergence_pages": 8192,
        "divergence_scope": "clone A only",
        "clones": {"count": 2},
        "snapshots": {
            "before_divergence": {
                "clones": [
                    {
                        "memory": {"pss_kib": 100 + offset},
                        "ram_mapping": {"pss_kib": 40 + offset},
                    },
                    {
                        "memory": {"pss_kib": 200 + offset},
                        "ram_mapping": {"pss_kib": 60 + offset},
                    },
                ]
            },
            "after_divergence": {
                "clones": [
                    {
                        "memory": {"pss_kib": 130 + offset},
                        "ram_mapping": {"pss_kib": 70 + offset},
                    },
                    {
                        "memory": {"pss_kib": 230 + offset},
                        "ram_mapping": {"pss_kib": 90 + offset},
                    },
                ]
            },
        },
        "ksm_telemetry": {
            "scope": "host-global counters; other KSM workloads may affect deltas",
            "counters_before": {"pages_shared": 10 + offset, "pages_sharing": 20},
            "counters_after": {"pages_shared": 14 + offset, "pages_sharing": 26},
            "ksmd_cpu_ticks_before": 1,
            "ksmd_cpu_ticks_after": 5,
            "ksmd_cpu_ticks_delta": 4,
        },
    }


class KsmSummaryTests(unittest.TestCase):
    def summarize(self, control: dict, with_ksm: dict) -> dict:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            control_path = root / "control.json"
            ksm_path = root / "ksm.json"
            output_path = root / "summary.json"
            control_path.write_text(json.dumps(control))
            ksm_path.write_text(json.dumps(with_ksm))
            subprocess.run(
                [sys.executable, str(SCRIPT), str(control_path), str(ksm_path), str(output_path)],
                check=True,
                capture_output=True,
                text=True,
            )
            return json.loads(output_path.read_text())

    def test_sums_clone_pss_and_reports_global_ksm_counter_deltas(self) -> None:
        summary = self.summarize(sample_result(0), sample_result(10, ksm_run=1))
        self.assertEqual(summary["status"], "paired")
        self.assertEqual(summary["control"]["clone_pss_kib"], {"before": 300, "after": 360})
        self.assertEqual(summary["control"]["ram_mapping_pss_kib"], {"before": 100, "after": 160})
        self.assertEqual(summary["with_ksm"]["clone_pss_kib"], {"before": 320, "after": 380})
        self.assertEqual(summary["with_ksm"]["ksm_counter_deltas"]["pages_shared"], 4)
        self.assertEqual(summary["with_ksm"]["ksm_counter_deltas"]["pages_sharing"], 6)
        self.assertEqual(summary["with_ksm"]["ksmd_cpu_ticks_delta"], 4)
        self.assertIn("host-global", summary["counter_scope_warning"])

    def test_missing_pss_is_unknown_instead_of_zero(self) -> None:
        result = sample_result(0)
        del result["snapshots"]["before_divergence"]["clones"][0]["memory"]["pss_kib"]
        summary = self.summarize(result, sample_result(0, ksm_run=1))
        self.assertIsNone(summary["control"]["clone_pss_kib"]["before"])

    def test_rejects_non_probe_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bad = root / "bad.json"
            good = root / "good.json"
            bad.write_text("{}")
            good.write_text(json.dumps(sample_result(0)))
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(bad), str(good), str(root / "summary.json")],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("two-clone-cow-probe", result.stderr)

    def test_rejects_mismatched_probe_configurations(self) -> None:
        control = sample_result(0)
        ksm = sample_result(0, ksm_run=1)
        ksm["ram_divergence_pages"] = 4096
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            control_path = root / "control.json"
            ksm_path = root / "ksm.json"
            control_path.write_text(json.dumps(control))
            ksm_path.write_text(json.dumps(ksm))
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(control_path), str(ksm_path), str(root / "summary.json")],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("not comparable", result.stderr)

    def test_rejects_runs_with_different_guest_images(self) -> None:
        control = sample_result(0)
        ksm = sample_result(0, ksm_run=1)
        ksm["guest"]["image_sha256"] = "different"
        with self.assertRaisesRegex(ValueError, "guest provenance"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                control_path = root / "control.json"
                ksm_path = root / "ksm.json"
                control_path.write_text(json.dumps(control))
                ksm_path.write_text(json.dumps(ksm))
                summarize(control_path, ksm_path)

    def test_rejects_runs_with_different_host_configuration(self) -> None:
        control = sample_result(0)
        ksm = sample_result(0, ksm_run=1)
        ksm["host"]["page_size_bytes"] = 65536
        with self.assertRaisesRegex(ValueError, "host configuration"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                control_path = root / "control.json"
                ksm_path = root / "ksm.json"
                control_path.write_text(json.dumps(control))
                ksm_path.write_text(json.dumps(ksm))
                summarize(control_path, ksm_path)

    def test_rejects_missing_provenance(self) -> None:
        control = sample_result(0)
        del control["guest"]
        with self.assertRaisesRegex(ValueError, "provenance"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                control_path = root / "control.json"
                ksm_path = root / "ksm.json"
                control_path.write_text(json.dumps(control))
                ksm_path.write_text(json.dumps(sample_result(0, ksm_run=1)))
                summarize(control_path, ksm_path)

    def test_rejects_ksm_run_values_that_do_not_match_pair_roles(self) -> None:
        for control_run, ksm_run in ((1, 1), (0, 0), (0, "2"), (True, 1)):
            with self.subTest(control_run=control_run, ksm_run=ksm_run):
                with self.assertRaisesRegex(ValueError, "ksm_run"):
                    with tempfile.TemporaryDirectory() as temporary:
                        root = Path(temporary)
                        control_path = root / "control.json"
                        ksm_path = root / "ksm.json"
                        control_path.write_text(json.dumps(sample_result(0, control_run)))
                        ksm_path.write_text(json.dumps(sample_result(0, ksm_run)))
                        summarize(control_path, ksm_path)

    def test_normalizes_sysfs_string_ksm_run_values(self) -> None:
        control = sample_result(0, ksm_run="0")
        with_ksm = sample_result(0, ksm_run="1")
        summary = self.summarize(control, with_ksm)
        self.assertEqual(summary["control"]["ksm_run_at_result"], 0)
        self.assertEqual(summary["with_ksm"]["ksm_run_at_result"], 1)


if __name__ == "__main__":
    unittest.main()
