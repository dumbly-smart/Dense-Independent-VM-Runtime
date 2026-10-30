import json
import tempfile
import unittest
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


if __name__ == "__main__":
    unittest.main()
