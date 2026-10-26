import unittest
import json
import tempfile
from pathlib import Path

import sweep_driver


def sample_result(percent=25):
    return {
        "baseline": {"count": 8},
        "clones": {"count": 8},
        "effective_ram_divergence_percent": percent,
        "ram_isolation": True,
        "disk_isolation": True,
    }


def sample_stats(percent=25):
    return {
        "clone_count": 8,
        "divergence_percent": percent,
        "baseline_process_pss_kib": 1_600_000,
        "before_divergence": {"process_pss_kib": 460_000},
        "after_divergence": {
            "process_pss_kib": 468_000,
            "process_private_kib": 430_000,
            "process_pss_plus_full_template_cache_kib": 730_000,
        },
    }


class SummarizeRunTests(unittest.TestCase):
    def get_summarize_run(self):
        function = getattr(sweep_driver, "summarize_run", None)
        self.assertTrue(callable(function), "sweep_driver.summarize_run must be implemented")
        return function

    def test_extracts_memory_metrics_from_a_matching_isolation_run(self):
        row = self.get_summarize_run()(sample_result(), sample_stats(), percentage=25, count=8)

        self.assertEqual(row["divergence_percent"], 25)
        self.assertEqual(row["baseline_process_pss_kib"], 1_600_000)
        self.assertEqual(row["clone_pss_before_kib"], 460_000)
        self.assertEqual(row["clone_pss_after_kib"], 468_000)
        self.assertEqual(row["clone_private_after_kib"], 430_000)
        self.assertEqual(row["clone_pss_plus_full_cache_kib"], 730_000)

    def test_rejects_a_result_for_a_different_divergence(self):
        with self.assertRaisesRegex(ValueError, "divergence"):
            self.get_summarize_run()(sample_result(10), sample_stats(10), percentage=25, count=8)

    def test_rejects_a_failed_isolation_check(self):
        result = sample_result()
        result["ram_isolation"] = False

        with self.assertRaisesRegex(ValueError, "RAM isolation"):
            self.get_summarize_run()(result, sample_stats(), percentage=25, count=8)


class RunSweepTests(unittest.TestCase):
    def test_runs_percentages_sequentially_and_writes_verified_index(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            densevm = root / "densevm-fake"
            densevm.write_text(
                "#!/usr/bin/env python3\n"
                "import json, pathlib, sys\n"
                "args = sys.argv[1:]\n"
                "if args[0] == 'probe':\n"
                "    out = pathlib.Path(args[args.index('--out') + 1])\n"
                "    count = int(args[args.index('--count') + 1])\n"
                "    percent = int(args[args.index('--divergence-percent') + 1])\n"
                "    out.mkdir()\n"
                "    result = {'baseline': {'count': count}, 'clones': {'count': count}, "
                "'effective_ram_divergence_percent': percent, 'ram_isolation': True, "
                "'disk_isolation': True}\n"
                "    (out / 'result.json').write_text(json.dumps(result))\n"
                "elif args[0] == 'stats':\n"
                "    result = json.loads(pathlib.Path(args[1]).read_text())\n"
                "    percent = result['effective_ram_divergence_percent']\n"
                "    summary = {'clone_count': result['clones']['count'], 'divergence_percent': percent, "
                "'baseline_process_pss_kib': 100, 'before_divergence': {'process_pss_kib': 80}, "
                "'after_divergence': {'process_pss_kib': 81, 'process_private_kib': 50, "
                "'process_pss_plus_full_template_cache_kib': 120}}\n"
                "    print(json.dumps(summary))\n"
            )
            densevm.chmod(0o755)
            output = root / "sweep"

            index_path = sweep_driver.run_sweep(
                densevm, output, count=2, percentages=(0, 100)
            )
            index = json.loads(index_path.read_text())
            summaries_exist = (output / "000-percent" / "summary.json").is_file()
            final_result_exists = (output / "100-percent" / "result.json").is_file()

        self.assertEqual(index["status"], "verified")
        self.assertEqual([run["divergence_percent"] for run in index["runs"]], [0, 100])
        self.assertTrue(summaries_exist)
        self.assertTrue(final_result_exists)


if __name__ == "__main__":
    unittest.main()
