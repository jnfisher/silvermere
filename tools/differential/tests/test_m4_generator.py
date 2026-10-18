"""T-098: the committed m4 scenario files are what m4_scenarios.py writes, so editing the
JSON by hand (and the next run of the generator undoing it) fails here."""
import filecmp
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
M4_REL = os.path.join("tools", "differential", "scenarios", "m4")


def files(top):
    return sorted(os.path.relpath(os.path.join(d, f), top)
                  for d, _, fs in os.walk(top) for f in fs if f.endswith(".json"))


class GeneratorMatchesCommitted(unittest.TestCase):
    def test_regenerating_changes_no_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            # the generator reads the other scenarios and the recorded captures from the root
            for rel in (os.path.join("tools", "differential", "scenarios"),
                        os.path.join("tests", "captures")):
                shutil.copytree(os.path.join(ROOT, rel), os.path.join(tmp, rel))
            # nothing of the m4 directory is copied in: a committed file the generator no
            # longer writes is then missing from its output, and fails below
            shutil.rmtree(os.path.join(tmp, M4_REL))
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "m4_scenarios.py"), tmp],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            committed, generated = os.path.join(ROOT, M4_REL), os.path.join(tmp, M4_REL)
            self.assertEqual(files(generated), files(committed))
            drift = [f for f in files(committed)
                     if not filecmp.cmp(os.path.join(committed, f), os.path.join(generated, f),
                                        shallow=False)]
            self.assertEqual(drift, [], "tools/differential/m4_scenarios.py does not write these "
                             "committed files; edit the generator, run it from the repository "
                             "root and commit the result")


if __name__ == "__main__":
    unittest.main()
