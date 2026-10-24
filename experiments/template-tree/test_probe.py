import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).parent))
from probe import parse_ram_mapping


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


if __name__ == "__main__":
    unittest.main()
