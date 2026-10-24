import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("summarize.py")


def sample_result(offset: int) -> dict:
    return {
        "experiment": "two-clone-cow-probe",
        "ksm_run": 0,
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
        summary = self.summarize(sample_result(0), sample_result(10))
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
        summary = self.summarize(result, sample_result(0))
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
        ksm = sample_result(0)
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


if __name__ == "__main__":
    unittest.main()
