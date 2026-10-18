"""compare.py adds finished segments together and skips an unfinished one."""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def seg(run, n, finished, exp_b, exp_i):
    d = os.path.join(run, f"seg-{n:02d}")
    os.makedirs(d)
    logs = []
    for who, exp in (("Brynja", exp_b), ("Isolde", exp_i)):
        ld = os.path.join(run, f"log-{n}-{who}")
        os.makedirs(ld)
        with open(os.path.join(ld, "raw.bin"), "wb") as f:
            for _ in range(exp):
                f.write(b"You gain 10 experience.\r\n")
        logs.append(ld)
        open(os.path.join(d, f"{who}.out"), "w").write("")
    with open(os.path.join(d, "bridges.txt"), "w") as f:
        for ld in logs:
            f.write("{'alive': True, 'logdir': '%s', 'port': 2325}\n" % ld)
    open(os.path.join(d, "start"), "w").write("1000")
    if finished:
        open(os.path.join(d, "end"), "w").write("4600")   # one hour


class Compare(unittest.TestCase):
    def test_sums_finished_segments_only(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 10, 10)
            seg(run, 2, True, 10, 9)
            seg(run, 3, False, 99, 99)
            out = subprocess.run([sys.executable, "-I", os.path.join(HERE, "compare.py"), run],
                                 capture_output=True, text=True).stdout
        self.assertIn("2 segments, 2.00 h", out)
        self.assertIn("Brynja: exp/hr 100 (20 kills credited)", out)
        self.assertIn("Isolde: exp/hr 95 (19 kills credited)", out)
        self.assertIn("95%", out)
        self.assertIn("GATES: PASS", out)

    def test_the_echo_of_a_typed_command_is_not_an_unknown_line(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 1, 1)
            ld = os.path.join(run, "log-1-Brynja")
            with open(os.path.join(ld, "raw.bin"), "ab") as f:
                f.write(b"a beast\r\n")
            args = [sys.executable, "-I", os.path.join(HERE, "compare.py"), run]
            echoed = subprocess.run(args, capture_output=True, text=True)
            with open(os.path.join(ld, "events.jsonl"), "w") as f:
                f.write('{"kind": "send", "text": "a beast"}\n')
            typed = subprocess.run(args, capture_output=True, text=True)
        self.assertEqual(echoed.returncode, 1, echoed.stdout)
        self.assertIn("'a beast'", echoed.stdout)
        self.assertEqual(typed.returncode, 0, typed.stdout)

    def test_a_prompt_with_the_bots_typing_after_it_is_judged_as_the_prompt(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 1, 1)
            ld = os.path.join(run, "log-1-Brynja")
            with open(os.path.join(ld, "raw.bin"), "ab") as f:
                f.write(b"Please choose your race [ ? for help ] : st\r\n[MAJORMUD]: exp\r\n")
            with open(os.path.join(ld, "events.jsonl"), "w") as f:
                f.write('{"kind": "send", "text": "st"}\n{"kind": "send", "text": "exp"}\n')
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "compare.py"), run],
                               capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("race prompts met", r.stdout)

    def test_a_party_row_is_not_an_unknown_line(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 1, 1)
            with open(os.path.join(run, "log-1-Brynja", "raw.bin"), "ab") as f:
                f.write(b"  Brynja Stonebrook              (Warrior)             [H:100%] R - Midrank\r\n")
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "compare.py"), run],
                               capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_a_split_party_is_flagged_as_advisory_only(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 10, 1)
            notes = "Party: Isolde is missing: going back for them\n" * 3
            open(os.path.join(run, "seg-01", "Brynja.out"), "w").write(notes)
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "compare.py"), run],
                               capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("party split notes from the bots: going back for a missing member x3", r.stdout)
        self.assertIn("ADVISORY: the party was split often on silvermere", r.stdout)

    def test_a_dead_bridge_fails_the_gate(self):
        with tempfile.TemporaryDirectory() as run:
            seg(run, 1, True, 3, 3)
            path = os.path.join(run, "seg-01", "bridges.txt")
            text = open(path).read().replace("'alive': True", "'alive': False", 1)
            open(path, "w").write(text)
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "compare.py"), run],
                               capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("1 disconnect(s) on silvermere", r.stdout)


if __name__ == "__main__":
    unittest.main()
