import tempfile
import unittest
from pathlib import Path

from probe import read_metrics


class ReadMetricsTests(unittest.TestCase):
    def test_reads_all_requested_smaps_rollup_fields(self):
        fields = {
            "Rss": 10,
            "Pss": 8,
            "Shared_Clean": 3,
            "Shared_Dirty": 1,
            "Private_Clean": 2,
            "Private_Dirty": 4,
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "smaps_rollup"
            path.write_text("".join(f"{key}: {value} kB\n" for key, value in fields.items()))
            parsed = read_metrics(path, set(fields))

        self.assertEqual(parsed, {f"{key.lower()}_kib": value for key, value in fields.items()})

    def test_reports_missing_requested_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "smaps_rollup"
            path.write_text("Rss: 10 kB\n")
            with self.assertRaisesRegex(RuntimeError, "Pss"):
                read_metrics(path, {"Rss", "Pss"})


if __name__ == "__main__":
    unittest.main()
