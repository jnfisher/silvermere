"""differential.py end to end, without servers (capture:DIR sources) and against a silent
fake server (the new side that answers nothing, as before T-021)."""
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "differential.py")
CAPS = os.path.abspath(os.path.join(HERE, "..", "..", "..", "tests", "captures"))


def run(*args, env=None):
    return subprocess.run([sys.executable, "-I", TOOL, "run", *args], capture_output=True, text=True,
                          env=env)


class DifferentialTests(unittest.TestCase):
    def test_capture_against_itself_passes(self):
        with tempfile.TemporaryDirectory() as rep:
            cap = f"capture:{CAPS}/entry-existing"
            p = run("--scenario", "entry-existing", "--orig", cap, "--new", cap, "--report", rep)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn("PASS  entry-existing", p.stdout)
            with open(f"{rep}/entry-existing/summary.json", encoding="utf-8") as f:
                s = json.load(f)
            self.assertEqual(s["event_diffs"], 0)
            self.assertEqual(s["byte"]["checked"], s["byte"]["equal"])
            self.assertGreater(s["byte"]["checked"], 0)
            for name in ("orig.raw", "new.raw", "orig.events.jsonl", "new.events.jsonl",
                         "diff.txt", "summary.json"):
                self.assertTrue(os.path.isfile(f"{rep}/entry-existing/{name}"), name)
            self.assertTrue(os.path.isfile(f"{rep}/summary.md"))

    def test_different_capture_fails_with_a_readable_diff(self):
        p = run("--scenario", "entry-existing", "--orig", f"capture:{CAPS}/entry-existing",
                "--new", f"capture:{CAPS}/entry-new-account")
        self.assertEqual(p.returncode, 1)
        self.assertIn("FAIL  entry-existing", p.stdout)
        self.assertIn("--- orig", p.stdout)
        self.assertIn("+++ new", p.stdout)

    def test_silent_new_server_reports_a_timeout_and_exits_nonzero(self):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(5)
        held = []
        threading.Thread(target=lambda: [held.append(srv.accept()[0]) for _ in range(3)],
                         daemon=True).start()
        try:
            p = run("--scenario", "entry-existing", "--no-setup", "--max-step-timeout", "1",
                    "--orig", f"capture:{CAPS}/entry-existing", "--new", f"127.0.0.1:{srv.getsockname()[1]}")
        finally:
            srv.close()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("new: timeout waiting for 'Username:'", p.stdout)
        self.assertNotIn("Traceback", p.stderr)

    def test_nothing_listening_is_reported_not_raised(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        p = run("--scenario", "entry-existing", "--no-setup",
                "--orig", f"capture:{CAPS}/entry-existing", "--new", f"127.0.0.1:{port}")
        self.assertEqual(p.returncode, 1)
        self.assertIn("new: cannot connect", p.stdout)
        self.assertNotIn("Traceback", p.stderr)

    def test_stage_with_an_empty_override_directory_says_what_is_missing(self):
        with tempfile.TemporaryDirectory() as root:
            p = run("--stage", "--scenario", "entry-existing", env={"SILVERMERE_RESEARCH_DIR": root})
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("MBBSEmu not found", p.stderr)
        self.assertNotIn("Traceback", p.stderr)

    def test_stage_reports_a_failing_stage_script_not_a_traceback(self):
        with tempfile.TemporaryDirectory() as root:
            os.makedirs(f"{root}/tools/mbbsemu")
            os.makedirs(f"{root}/test")
            open(f"{root}/tools/mbbsemu/MBBSEmu", "w").close()
            with open(f"{root}/test/stage.sh", "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\necho no dist >&2\nexit 1\n")
            os.chmod(f"{root}/test/stage.sh", 0o755)
            p = run("--stage", "--scenario", "entry-existing", env={"SILVERMERE_RESEARCH_DIR": root})
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("stage.sh failed: no dist", p.stderr)
        self.assertNotIn("Traceback", p.stderr)

    def test_statistical_scenarios_are_skipped(self):
        cap = f"capture:{CAPS}/fight-arena"
        p = run("--scenario", "fight-arena", "--orig", cap, "--new", cap)
        self.assertEqual(p.returncode, 0)
        self.assertIn("SKIP", p.stdout)


class RetryProblemTests(unittest.TestCase):
    def test_a_resent_enter_is_a_problem_on_the_new_side_only(self):
        sys.path.insert(0, os.path.join(HERE, ".."))
        import differential
        import replay
        step = replay.StepCapture(5, "send", "", 100, b"\r\n", until="x", retries=1)
        self.assertEqual(differential.problems_of("orig", [step]), [])
        self.assertEqual(differential.problems_of("new", [step]),
                         ["new: step 5 dropped a bare Enter (1 resent)"])

    def test_a_retry_inside_a_merged_group_is_still_reported(self):
        sys.path.insert(0, os.path.join(HERE, ".."))
        import differential
        import replay
        steps = [replay.StepCapture(6, "send", "a", 0, b"x"),
                 replay.StepCapture(7, "send", "", 1, b"\r\n", until="x", retries=1, burst_retries=2)]
        merged = differential.merge_steps(steps, [[6, 7]])
        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0].retries, 1)
        self.assertEqual(merged[0].burst_retries, 2)
        self.assertEqual(differential.problems_of("new", merged),
                         ["new: step 6 dropped a bare Enter (1 resent)"])


class BurstProblemTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, os.path.join(HERE, ".."))

    def test_no_clean_attempt_is_a_problem_on_either_side(self):
        import differential
        import replay
        step = replay.StepCapture(5, "send", "n", 0, b"", until="x", burst_retries=2,
                                  burst_unclean=True)
        for side in ("orig", "new"):
            self.assertEqual(differential.problems_of(side, [step]),
                             [f"{side}: step 5: the flood burst drained mid-burst in every attempt "
                              "(2 redone)"])

    def test_a_redone_burst_alone_is_not_a_problem(self):
        import differential
        import replay
        step = replay.StepCapture(5, "send", "n", 0, b"", until="x", burst_retries=1)
        self.assertEqual(differential.problems_of("orig", [step]), [])
        self.assertEqual(differential.problems_of("new", [step]), [])

    def test_a_failed_walk_out_is_a_problem(self):
        import differential
        import replay
        step = replay.StepCapture(5, "send", "n", 0, b"", until="x")
        step.walk_out_failed = True
        self.assertEqual(differential.problems_of("orig", [step]),
                         ["orig: step 5: the flood burst drained mid-burst and the walk out to "
                          "redo it did not match (0 redone)"])

    def test_an_unknown_line_shows_its_text_in_the_diff(self):
        import diff
        e = {"step": 48, "name": "unknown", "fields": {}, "line": "A rat wanders in."}
        self.assertEqual(diff.render_event(e), "[48] unknown line='A rat wanders in.'")

    def test_unknown_lines_are_keyed_by_text_and_step(self):
        import diff
        ev = lambda st, line: {"step": st, "name": "unknown", "fields": {}, "line": line}
        pad = lambda st: {"step": st, "name": "echo", "fields": {"cmd": str(st)}}
        masks = [r"^(Pollrate:\s*)\d+$"]
        a = [pad(1), ev(1, "A rat wanders in."), pad(2), ev(2, "Pollrate: 275")]
        self.assertEqual(diff.diff_events(a, a, masks), [])
        # a changed line is a difference at its place, not a pass
        changed = [pad(1), ev(1, "A bat wanders in."), pad(2), ev(2, "Pollrate: 275")]
        out = diff.diff_events(a, changed, masks)
        self.assertEqual([(d["field"], d["orig"], d["new"]) for d in out],
                         [("line", "A rat wanders in.", "A bat wanders in.")])
        # the same line at another step is a difference, though the multiset is equal
        moved = [pad(1), pad(2), ev(2, "A rat wanders in."), ev(2, "Pollrate: 275")]
        self.assertTrue(diff.diff_events(a, moved, masks))
        self.assertEqual(diff.unknown_extra(a, moved, masks), [], "unknown_extra alone misses it")
        # only the value of a masked line may differ; its label may not
        valued = [pad(1), ev(1, "A rat wanders in."), pad(2), ev(2, "Pollrate: 0")]
        self.assertEqual(diff.diff_events(a, valued, masks), [])
        self.assertTrue(diff.diff_events(a, valued))
        # text after the masked value is not part of the value: a difference there counts
        trailing = [pad(1), ev(1, "A rat wanders in."), pad(2), ev(2, "Pollrate: 275 ms")]
        self.assertTrue(diff.diff_events(a, trailing, masks))
        loose = [r"^(Pollrate:\s*)\d+"]
        self.assertTrue(diff.diff_events(a, trailing, loose))
        self.assertEqual(diff.unknown_extra(a, trailing, loose), ["Pollrate: 275 ms"])
        relabelled = [pad(1), ev(1, "A rat wanders in."), pad(2), ev(2, "Pollsate: 0")]
        self.assertTrue(diff.diff_events(a, relabelled, masks))

    def test_script_options_reach_the_script(self):
        import differential

        class Spy:
            def __init__(self, *a):
                self.seen = None

            def run_script(self, script, *a, **k):
                Spy.seen = script
                return []
        orig = differential.replay.Player
        differential.replay.Player = Spy
        try:
            sc = {"script": os.path.join(HERE, "..", "scenarios", "m1", "scripts",
                                         "flood-and-refusals.json"),
                  "script_options": {"echo_anchored": True, "settle": 1.5}}
            differential.live("new", "127.0.0.1:1", sc, setup=False)
        finally:
            differential.replay.Player = orig
        self.assertTrue(Spy.seen["echo_anchored"])
        self.assertEqual(Spy.seen["settle"], 1.5)

    def test_the_job_log_line_shows_the_retries(self):
        import differential
        r = {"burst_check": True, "burst_retries": {"orig": 1, "new": 0},
             "enter_retries": {"orig": 0, "new": 0}}
        self.assertEqual(differential.retry_note(r), "  [burst_retries orig=1 new=0]")
        r = {"burst_check": False, "burst_retries": {"orig": 0, "new": 0},
             "enter_retries": {"orig": 1, "new": 0}}
        self.assertEqual(differential.retry_note(r), "  [enter_retries orig=1 new=0]")
        self.assertEqual(differential.retry_note({"status": "skipped"}), "")


if __name__ == "__main__":
    unittest.main()
