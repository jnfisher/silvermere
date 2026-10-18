"""Python and Rust normalisation agree on the twelve T-005 captures.

Runs `cargo run -p silvermere-testkit --bin normalise` over tests/captures/* and compares
(name, fields with masks applied) event by event with normalise.events_for on the same raw.bin.
Needs cargo (and the vendored play/ modules, or $SILVERMERE_RESEARCH_DIR); run with `python3 -I tools/differential/tests/cross_check.py`.
"""
import glob
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
sys.path.insert(0, os.path.join(HERE, ".."))

import normalise  # noqa: E402
import replay  # noqa: E402


def rust_events(dirs):
    cp = subprocess.run(["cargo", "run", "-q", "-p", "silvermere-testkit", "--bin", "normalise",
                         "--"] + dirs, cwd=ROOT, capture_output=True, text=True, check=True)
    by = {}
    for line in cp.stdout.splitlines():
        e = json.loads(line)
        by.setdefault(e["capture"], []).append((e["name"], e["fields"]))
    return by


class CrossCheck(unittest.TestCase):
    def test_all_captures(self):
        dirs = sorted(os.path.dirname(p) for p in glob.glob(f"{ROOT}/tests/captures/*/script.json"))
        self.assertEqual(len(dirs), 12)
        rust = rust_events(dirs)
        masks = normalise.load_masks()
        for d in dirs:
            name = os.path.basename(d)
            with self.subTest(name):
                with open(f"{d}/script.json", encoding="utf-8") as f:
                    script = json.load(f)
                with open(f"{d}/raw.bin", "rb") as f:
                    raw = f.read()
                steps = replay.from_capture(script, raw)
                py = [(e["name"], e["fields"]) for e in normalise.events_of(steps, masks)]
                self.assertGreater(len(py), 0)
                self.assertEqual(py, rust.get(name, []))


if __name__ == "__main__":
    unittest.main()
