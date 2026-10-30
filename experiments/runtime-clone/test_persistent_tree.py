import json
import tempfile
import unittest
from unittest import mock
from pathlib import Path

import persistent_tree


class PersistentMemoryMapTests(unittest.TestCase):
    def test_captures_memory_map_for_each_live_clone_id(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "densevm-fake"
            binary.write_text(
                "#!/usr/bin/env python3\n"
                "import json, sys\n"
                "args = sys.argv[1:]\n"
                "assert args[:2] == ['memory-map', 'clone']\n"
                "clone_id = args[2]\n"
                "assert args[3] == '--state-dir'\n"
                "print(json.dumps({'clone': {'id': clone_id, 'generation_id': 'gen', "
                "'qemu_pid': 42, 'ram_file': '/tmp/ram.img'}, "
                "'page_estimates': {'shared_pages_estimate': 17}}))\n"
            )
            binary.chmod(0o755)
            result = {"steps": []}

            reports = persistent_tree.capture_clone_memory_maps(
                result, binary, root / "state", [{"id": "clone-a", "generation_id": "gen"}, {"id": "clone-b", "generation_id": "gen"}]
            )

            self.assertEqual(list(reports), ["clone-a", "clone-b"])
            self.assertEqual(reports["clone-a"]["page_estimates"]["shared_pages_estimate"], 17)
            self.assertEqual(len(result["steps"]), 2)

    def test_rejects_a_memory_map_for_a_different_clone_id(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "densevm-fake"
            binary.write_text(
                "#!/usr/bin/env python3\n"
                "import json\n"
                "print(json.dumps({'clone': {'id': 'wrong'}, 'page_estimates': {}}))\n"
            )
            binary.chmod(0o755)
            result = {"steps": []}

            with self.assertRaisesRegex(RuntimeError, "reported clone ID"):
                persistent_tree.capture_clone_memory_maps(
                    result, binary, root / "state", [{"id": "clone-a", "generation_id": "gen"}]
                )

    def test_records_that_one_persistent_clone_stopped_and_sibling_survived(self):
        class Serial:
            def command(self, command, expected):
                return {"ram-read": "RAM 91", "disk-read": "DISK 91"}[command]

        stopped = {"id": "clone-a", "qemu_pid": 101}
        sibling = {"id": "clone-b", "qemu_pid": 202}
        result = {"state": "stopped", "pid_gone_on_return": True}
        serials = {"clone-b": Serial()}

        with mock.patch.object(Path, "exists", side_effect=[False, True]):
            report = persistent_tree.validate_sibling_survival_after_stop(
                result,
                stopped,
                [sibling],
                serials,
                {"clone-b": {"ram": 91, "disk": 91}},
            )

        self.assertEqual(report["stopped_clone"], "clone-a")
        self.assertEqual(report["survivors"]["clone-b"]["ram_reply"], "RAM 91")
        self.assertEqual(report["survivors"]["clone-b"]["disk_reply"], "DISK 91")

    def test_rejects_sibling_failure_after_persistent_clone_stop(self):
        stopped = {"id": "clone-a", "qemu_pid": 101}
        sibling = {"id": "clone-b", "qemu_pid": 202}
        result = {"state": "stopped", "pid_gone_on_return": True}

        with mock.patch.object(Path, "exists", return_value=False):
            with self.assertRaisesRegex(RuntimeError, "sibling QEMU process"):
                persistent_tree.validate_sibling_survival_after_stop(
                    result,
                    stopped,
                    [sibling],
                    {},
                    {"clone-b": {"ram": 73, "disk": 73}},
                )


if __name__ == "__main__":
    unittest.main()
