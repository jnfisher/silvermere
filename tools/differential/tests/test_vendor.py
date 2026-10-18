"""The vendored copy of the MBBSEmu test harness's play/ modules still describes the registry that
tests/fixtures/patterns.json (the T-004 export, read by the Rust side) exports."""
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import replay  # noqa: E402

FIXTURE = os.path.join(HERE, "..", "..", "..", "tests", "fixtures", "patterns.json")


class VendorTests(unittest.TestCase):
    def test_source_note_and_fetcher_exist(self):
        vendor = replay.VENDOR
        self.assertTrue(os.path.isfile(os.path.join(vendor, "SOURCE.md")))
        # the distribution is downloaded into a cache, never kept in the tree (T-120)
        self.assertFalse(os.path.exists(os.path.join(vendor, "dist")))
        self.assertTrue(os.access(os.path.join(vendor, "..", "fetch-stock.sh"), os.X_OK))

    def test_vendored_registry_export_equals_the_fixture(self):
        _, _, pt = replay.load_play()
        exported = json.loads(json.dumps(pt.export_registry(with_data=False)))
        with open(FIXTURE, encoding="utf-8") as f:
            fixture = json.load(f)
        self.assertEqual(exported, fixture)


if __name__ == "__main__":
    unittest.main()
