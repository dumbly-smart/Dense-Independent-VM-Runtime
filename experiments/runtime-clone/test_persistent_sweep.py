import unittest

import persistent_sweep


class PersistentSweepCgroupSummaryTests(unittest.TestCase):
    def test_reports_current_delta_and_stat_samples_for_a_measured_cgroup(self):
        summary = persistent_sweep.summarize_cgroup_memory(
            {
                "before_clones": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1000,
                    "memory_limit": "3221225472",
                    "memory_max_bytes": 3221225472,
                    "memory_stat": {"kernel": 100, "file": 200},
                    "scope": "test cgroup",
                },
                "after_clone_start": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1200,
                    "memory_stat": {"kernel": 125, "file": 220},
                    "scope": "test cgroup",
                },
                "after_divergence": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1400,
                    "memory_stat": {"kernel": 150, "file": 250},
                    "scope": "test cgroup",
                },
                "after_sibling_stop": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1300,
                    "memory_stat": {"kernel": 140, "file": 230},
                    "scope": "test cgroup",
                },
            }
        )

        self.assertEqual(summary["status"], "measured")
        self.assertEqual(summary["delta_bytes"], 400)
        self.assertEqual(summary["memory_max_bytes"], 3221225472)
        self.assertEqual(summary["delta_clone_start_to_divergence_bytes"], 200)
        self.assertEqual(summary["after_sibling_stop_bytes"], 1300)
        self.assertEqual(summary["memory_stat_after"]["kernel"], 150)

    def test_does_not_turn_unavailable_cgroup_data_into_zero(self):
        summary = persistent_sweep.summarize_cgroup_memory(
            {
                "before_clones": {"status": "measured", "current_bytes": 1000},
                "after_divergence": {"status": "unavailable", "reason": "no access"},
            }
        )

        self.assertEqual(summary["status"], "unavailable")
        self.assertIsNone(summary["delta_bytes"])


if __name__ == "__main__":
    unittest.main()
