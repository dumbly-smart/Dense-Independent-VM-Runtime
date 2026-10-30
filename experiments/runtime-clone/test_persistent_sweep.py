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
                    "memory_peak_bytes": 1300,
                    "memory_events": {"max": 0, "oom": 0, "oom_kill": 0},
                    "scope": "test cgroup",
                },
                "after_clone_start": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1200,
                    "memory_stat": {"kernel": 125, "file": 220},
                    "memory_peak_bytes": 1400,
                    "memory_events": {"max": 1, "oom": 0, "oom_kill": 0},
                    "scope": "test cgroup",
                },
                "after_divergence": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1400,
                    "memory_stat": {"kernel": 150, "file": 250},
                    "memory_peak_bytes": 1700,
                    "memory_events": {"max": 2, "oom": 0, "oom_kill": 0},
                    "scope": "test cgroup",
                },
                "after_sibling_stop": {
                    "status": "measured",
                    "path": "/sys/fs/cgroup/test",
                    "current_bytes": 1300,
                    "memory_stat": {"kernel": 140, "file": 230},
                    "memory_peak_bytes": 1700,
                    "memory_events": {"max": 2, "oom": 0, "oom_kill": 0},
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
        self.assertEqual(summary["memory_peak_bytes"], {
            "before_clones": 1300,
            "after_clone_start": 1400,
            "after_divergence": 1700,
            "after_sibling_stop": 1700,
        })
        self.assertEqual(summary["memory_events_delta_clone_start_to_divergence"], {
            "max": 1,
            "oom": 0,
            "oom_kill": 0,
        })

    def test_does_not_turn_unavailable_cgroup_data_into_zero(self):
        summary = persistent_sweep.summarize_cgroup_memory(
            {
                "before_clones": {"status": "measured", "current_bytes": 1000},
                "after_divergence": {"status": "unavailable", "reason": "no access"},
            }
        )

        self.assertEqual(summary["status"], "unavailable")
        self.assertIsNone(summary["delta_bytes"])

    def test_does_not_invent_an_event_delta_for_inconsistent_counter_sets(self):
        samples = {
            "before_clones": {"status": "measured", "path": "/scope", "current_bytes": 10},
            "after_clone_start": {
                "status": "measured",
                "path": "/scope",
                "current_bytes": 20,
                "memory_events": {"oom": 0},
            },
            "after_divergence": {
                "status": "measured",
                "path": "/scope",
                "current_bytes": 30,
                "memory_events": {"oom": 0, "oom_kill": 0},
            },
        }

        summary = persistent_sweep.summarize_cgroup_memory(samples)

        self.assertEqual(summary["status"], "measured")
        self.assertIsNone(summary["memory_events_delta_clone_start_to_divergence"])


if __name__ == "__main__":
    unittest.main()
