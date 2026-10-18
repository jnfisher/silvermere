"""replay.py against a fake telnet server (stdlib socketserver) that plays back a capture's
bytes step by step: the recorded steps must equal the capture's steps (the `until` discipline
and the attribution of text that arrives between commands)."""
import json
import os
import socketserver
import sys
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import replay  # noqa: E402

CAPTURES = os.path.abspath(os.path.join(HERE, "..", "..", "..", "tests", "captures"))


class Handler(socketserver.BaseRequestHandler):
    def read_line(self):
        buf = b""
        while not buf.endswith(b"\r"):
            c = self.request.recv(1)
            if not c:
                return None
            buf += c
        return buf

    def handle(self):
        for act, arg in self.server.actions:
            if act == "send":
                self.request.sendall(arg)
            elif act == "expect":
                line = self.read_line()
                if line is None:
                    return
                self.server.received.append((time.time(), line))
            elif act == "expect_chunk":
                # one recv after `arg` seconds: what the client wrote by then, in one piece
                time.sleep(arg)
                chunk = self.request.recv(65536)
                if not chunk:
                    return
                self.server.received.append((time.time(), chunk))
            elif act == "sleep":
                time.sleep(arg)
            elif act == "close":
                return


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, actions):
        super().__init__(("127.0.0.1", 0), Handler)
        self.actions = actions
        self.received = []   # (time, bytes) of each line or chunk read
        threading.Thread(target=self.serve_forever, daemon=True).start()

    @property
    def port(self):
        return self.server_address[1]


def actions_for(steps):
    out = []
    for s in steps:
        if s.op == "send":
            out += [("expect", None), ("send", s.raw)]
        elif s.op == "connect":
            out.append(("send", s.raw))
        elif s.op == "sleep":
            out.append(("send", s.raw))
        elif s.op == "disconnect":
            out.append(("close", None))
    return out + [("close", None)]


class ReplayTests(unittest.TestCase):
    def test_capture_playback_gives_back_the_capture_steps(self):
        d = os.path.join(CAPTURES, "entry-existing")
        with open(os.path.join(d, "script.json"), encoding="utf-8") as f:
            script = json.load(f)
        with open(os.path.join(d, "raw.bin"), "rb") as f:
            raw = f.read()
        want = replay.from_capture(script, raw)
        server = Server(actions_for(want))
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual([(s.index, s.op, s.send) for s in got], [(s.index, s.op, s.send) for s in want])
        for g, w in zip(got, want):
            self.assertEqual(g.raw, w.raw, f"step {w.index} {w.send!r}")
            self.assertEqual(g.offset, w.offset)
        self.assertFalse(any(s.timed_out for s in got))

    def test_flood_capture_playback_gives_back_the_capture_steps(self):
        # the burst goes out as one write; the steps and their bytes are the recorded ones
        d = os.path.join(CAPTURES, "flood-and-refusals")
        with open(os.path.join(d, "script.json"), encoding="utf-8") as f:
            script = json.load(f)
        with open(os.path.join(d, "raw.bin"), "rb") as f:
            raw = f.read()
        want = replay.from_capture(script, raw)
        server = Server(actions_for(want))
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual([(s.index, s.op, s.send) for s in got], [(s.index, s.op, s.send) for s in want])
        self.assertEqual(b"".join(s.raw for s in got), b"".join(s.raw for s in want))
        burst = [s for s in got if s.send in ("search", "exp") and s.offset == got[6].offset]
        self.assertEqual(len(burst), 15)
        self.assertFalse(any(s.timed_out for s in got))

    def test_room_walk_capture_plays_back_through_anchored_waits(self):
        # m1/room-walk's script_options (T-079): every in-realm step waits for its own echo,
        # then its prompt, then quiet; the recorded steps still come back byte for byte
        d = os.path.join(CAPTURES, "room-walk")
        with open(os.path.join(d, "script.json"), encoding="utf-8") as f:
            script = json.load(f)
        with open(os.path.join(d, "raw.bin"), "rb") as f:
            raw = f.read()
        with open(os.path.join(HERE, "..", "scenarios", "m1", "room-walk.json"),
                  encoding="utf-8") as f:
            opts = json.load(f)["script_options"]
        self.assertTrue(opts["echo_anchored"])
        script.update(opts, settle=0.2)   # the quiet wait, shortened for the fake server
        want = replay.from_capture(script, raw)
        server = Server(actions_for(want))
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual([(s.index, s.send) for s in got], [(s.index, s.send) for s in want])
        for g, w in zip(got, want):
            self.assertEqual(g.raw, w.raw, f"step {w.index} {w.send!r}")
        self.assertFalse(any(s.timed_out for s in got))
        # the recording itself is shifted at 36-38: `go manhole`'s reply arrived after `w` was
        # sent, so those recorded bytes have no echo before their prompt and the wait falls
        # back to the plain prompt (`echo_missing`); live, each side waits for its own echo
        self.assertEqual({s.index for s in got if s.echo_missing}, {37, 38})

    def test_text_between_commands_belongs_to_the_step_it_arrived_in(self):
        script = {"steps": [
            {"op": "connect", "until": "Name:"},
            {"send": "a", "until": "ok1"},
            {"op": "sleep", "seconds": 0.8},
            {"send": "b", "until": "ok2"},
        ]}
        server = Server([("send", b"Name: "), ("expect", None), ("send", b"ok1\r\n"),
                         ("sleep", 0.3), ("send", b"late\r\n"), ("expect", None),
                         ("send", b"ok2\r\n"), ("close", None)])
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual([s.raw for s in got],
                         [b"Name: ", b"ok1\r\n", b"late\r\n", b"ok2\r\n"])

    def test_two_connections_each_keep_their_own_steps(self):
        script = {"steps": [
            {"op": "connect", "until": "Name:"},
            {"as": "B", "op": "connect", "until": "Name:"},
            {"send": "a", "until": "ok"},
            {"as": "B", "send": "b", "until": "ok"},
        ]}
        server = Server([("send", b"Name: "), ("expect", None), ("send", b"ok\r\n"),
                         ("close", None)])
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual([s.client for s in got], ["A", "B", "A", "B"])
        self.assertEqual([s.raw for s in got], [b"Name: ", b"Name: ", b"ok\r\n", b"ok\r\n"])

    def test_timeout_is_reported_and_stops_the_replay(self):
        script = {"steps": [
            {"op": "connect", "until": "Username:", "timeout": 0.5},
            {"send": "x", "until": "never"},
        ]}
        server = Server([("send", b"Hello\r\n"), ("sleep", 2), ("close", None)])
        try:
            got = replay.Player(server.port).run_script(script)
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual(len(got), 1)
        self.assertTrue(got[0].timed_out)
        self.assertEqual(got[0].until, "Username:")

    def test_connection_refused_is_reported_not_raised(self):
        server = Server([])
        port = server.port
        server.shutdown()
        server.server_close()
        got = replay.Player(port).run_script({"steps": [{"op": "connect", "until": "Username:"}]})
        self.assertEqual(len(got), 1)
        self.assertIsNotNone(got[0].error)


def play(script, actions):
    server = Server(actions)
    try:
        got = replay.Player(server.port).run_script(script)
    finally:
        server.shutdown()
        server.server_close()
    return got, server.received


PROMPT = b"[HP=35]:"
REDRAW = b"\r\n\x1b[79D\x1b[KRoom\r\n" + PROMPT


class BurstTests(unittest.TestCase):
    def test_sends_typed_without_waiting_go_out_in_one_write(self):
        script = {"steps": [
            {"op": "connect", "until": "Name:"},
            {"send": "search", "nowait": True},
            {"send": "exp", "nowait": True},
            {"send": "exp", "nowait": True},
            {"op": "sleep", "seconds": 0.5},
            {"send": "w", "until": "ok"},
        ]}
        got, received = play(script, [
            ("send", b"Name: "), ("expect_chunk", 0.3), ("send", b"queued\r\n"),
            ("expect", None), ("send", b"ok\r\n"), ("close", None)])
        self.assertEqual(received[0][1], b"search\rexp\rexp\r", "the burst is one write")
        self.assertEqual([(s.index, s.send) for s in got],
                         [(0, None), (1, "search"), (2, "exp"), (3, "exp"), (4, None), (5, "w")])
        self.assertEqual(len({s.offset for s in got[1:4]}), 1, "one offset for the burst")
        self.assertEqual(got[3].raw, b"")
        self.assertEqual(got[4].raw, b"queued\r\n")
        self.assertEqual(got[5].raw, b"ok\r\n")

    def test_a_lone_nowait_send_is_sent_as_before(self):
        script = {"steps": [
            {"op": "connect", "until": "Name:"},
            {"send": "a", "nowait": True},
            {"send": "b", "until": "ok"},
        ]}
        got, received = play(script, [
            ("send", b"Name: "), ("expect", None), ("expect", None), ("send", b"ok\r\n"),
            ("close", None)])
        self.assertEqual([r[1] for r in received], [b"a\r", b"b\r"])
        self.assertFalse(any(s.timed_out for s in got))


class BareEnterTests(unittest.TestCase):
    def enter_script(self, **opts):
        return {"steps": [
            {"op": "connect", "until": "Name:"},
            {"send": "E", "until": r"\[HP=\d+\]:"},
            dict({"send": "", "until": r"Room\n[^\n]*\[HP=\d+\]:", "timeout": 10}, **opts),
        ]}

    def test_the_enter_waits_for_the_output_to_settle(self):
        got, received = play(self.enter_script(settle=0.4), [
            ("send", b"Name: "), ("expect", None), ("send", PROMPT),
            ("sleep", 0.3), ("send", b"\r\nlate line\r\n" + PROMPT),
            ("expect", None), ("send", REDRAW), ("close", None)])
        sent_late = received[0][0] + 0.3
        self.assertGreaterEqual(received[1][0] - sent_late, 0.35,
                                "the Enter comes a settle period after the last output")
        self.assertEqual(got[2].raw, REDRAW)
        self.assertEqual(got[2].retries, 0)
        self.assertIn(b"late line", got[1].raw)

    def test_the_originals_dropped_enter_is_sent_again_and_its_echo_cut(self):
        got, received = play(self.enter_script(settle=0.1, drop_after=0.5), [
            ("send", b"Name: "), ("expect", None), ("send", b"\r\n" + PROMPT),
            ("expect", None), ("send", b"\r\n"),           # the drop: only the echo
            ("expect", None), ("send", REDRAW), ("close", None)])
        self.assertEqual(len(received), 3, "E, the dropped Enter, the retry")
        self.assertEqual(got[2].retries, 1)
        self.assertTrue(got[2].matched)
        self.assertEqual(got[1].raw, b"\r\n" + PROMPT, "the step before keeps its bytes")
        self.assertEqual(got[2].raw, REDRAW, "the step reads as one Enter taken at once")
        self.assertEqual(got[2].offset, got[1].offset + len(got[1].raw))

    def test_a_slow_answer_is_not_a_drop(self):
        got, received = play(self.enter_script(settle=0.1, drop_after=0.3), [
            ("send", b"Name: "), ("expect", None), ("send", PROMPT),
            ("expect", None), ("send", b"\r\n\x1b[79D"), ("sleep", 0.8),
            ("send", b"\x1b[KRoom\r\n" + PROMPT), ("sleep", 0.5), ("close", None)])
        self.assertEqual(len(received), 2, "no second Enter")
        self.assertEqual(got[2].retries, 0)
        self.assertTrue(got[2].matched)
        self.assertEqual(got[2].raw, REDRAW)

    def test_retries_stop_at_the_limit_and_the_step_times_out(self):
        actions = [("send", b"Name: "), ("expect", None), ("send", PROMPT)]
        for _ in range(replay.MAX_RETRIES + 1):
            actions += [("expect", None), ("send", b"\r\n")]
        actions += [("sleep", 3), ("close", None)]
        got, received = play(self.enter_script(settle=0.1, drop_after=0.3, timeout=1.5), actions)
        self.assertEqual(len(received), 2 + replay.MAX_RETRIES)
        self.assertTrue(got[2].timed_out)
        self.assertEqual(got[2].retries, replay.MAX_RETRIES)
        # only the last echo is left: the step shows what the server finally answered
        self.assertEqual(got[2].raw, b"\r\n")


class BareEnterGuardTests(unittest.TestCase):
    def test_a_bare_enter_with_a_skip_condition_is_left_to_the_recorder(self):
        self.assertTrue(replay.is_bare_enter({"send": "", "until": "x"}))
        self.assertFalse(replay.is_bare_enter({"send": "", "until": "x", "skip_if": "y"}))
        self.assertFalse(replay.is_bare_enter({"send": "", "until": "x", "if_unset": "v"}))


class BurstCheckTests(unittest.TestCase):
    CHECK = {"from": 1, "to": 4, "warnings": "^W$", "queued": "^(?:.*\\]:)?q$",
             "expect_warnings": 2, "max_retries": 2,
             "reset": [{"send": "s", "until": "back ok", "timeout": 5}]}

    def script(self, **check):
        return {"burst_check": dict(self.CHECK, **check), "steps": [
            {"op": "connect", "until": "Name:"},
            {"send": "n", "until": "room ok"},
            {"send": "a", "nowait": True},
            {"send": "b", "nowait": True},
            {"op": "sleep", "seconds": 0.5},
            {"send": "w", "until": "done"},
        ]}

    def attempt(self, answer):
        return [("expect", None), ("send", b"room ok\r\n"), ("expect_chunk", 0.2),
                ("send", answer)]

    def test_a_burst_drained_mid_burst_is_done_again_and_only_the_whole_one_kept(self):
        drained, whole = b"W\r\nq\r\nW\r\n", b"W\r\nW\r\nq\r\n"
        actions = [("send", b"Name: ")] + self.attempt(drained)
        actions += [("expect", None), ("send", b"back ok\r\n")] + self.attempt(whole)
        actions += [("expect", None), ("send", b"done\r\n"), ("close", None)]
        got, received = play(self.script(), actions)
        self.assertEqual([r[1] for r in received],
                         [b"n\r", b"a\rb\r", b"s\r", b"n\r", b"a\rb\r", b"w\r"])
        self.assertEqual([s.index for s in got], [0, 1, 2, 3, 4, 5])
        self.assertEqual(got[1].burst_retries, 1)
        self.assertFalse(got[1].burst_unclean)
        self.assertEqual(b"".join(s.raw for s in got),
                         b"Name: room ok\r\n" + whole + b"done\r\n",
                         "the drained attempt and the walk out are not in the capture")
        self.assertEqual(got[1].raw, b"room ok\r\n")
        self.assertFalse(any(s.timed_out for s in got))

    def test_a_whole_burst_is_kept_as_it_is(self):
        whole = b"W\r\nW\r\n[HP=1]:q\r\n"
        actions = [("send", b"Name: ")] + self.attempt(whole)
        actions += [("expect", None), ("send", b"done\r\n"), ("close", None)]
        got, received = play(self.script(), actions)
        self.assertEqual(len(received), 3)
        self.assertEqual(got[1].burst_retries, 0)
        self.assertFalse(got[1].burst_unclean)
        self.assertEqual(b"".join(s.raw for s in got), b"Name: room ok\r\n" + whole + b"done\r\n")

    def test_retries_stop_at_the_limit_and_the_last_attempt_is_kept(self):
        drained = b"q\r\nW\r\n"
        actions = [("send", b"Name: ")] + self.attempt(drained)
        for _ in range(2):
            actions += [("expect", None), ("send", b"back ok\r\n")] + self.attempt(drained)
        actions += [("expect", None), ("send", b"done\r\n"), ("close", None)]
        got, received = play(self.script(), actions)
        self.assertEqual(len(received), 3 * 2 + 2 + 1, "three attempts, two walks out, then w")
        self.assertEqual(got[1].burst_retries, 2)
        self.assertTrue(got[1].burst_unclean, "no clean attempt: flagged for problems_of")
        self.assertEqual(b"".join(s.raw for s in got), b"Name: room ok\r\n" + drained + b"done\r\n",
                         "the last attempt is compared, drain and all")


class WalkOutTests(unittest.TestCase):
    def test_a_walk_out_that_does_not_match_keeps_the_attempt_and_is_flagged(self):
        script = BurstCheckTests().script()
        script["burst_check"]["reset"][0]["timeout"] = 0.5
        drained = b"q\r\nW\r\n"
        actions = [("send", b"Name: ")] + BurstCheckTests().attempt(drained)
        actions += [("expect", None), ("send", b"somewhere else\r\n")]
        actions += [("expect", None), ("send", b"done\r\n"), ("close", None)]
        got, received = play(script, actions)
        self.assertEqual([r[1] for r in received], [b"n\r", b"a\rb\r", b"s\r", b"w\r"])
        self.assertTrue(got[1].walk_out_failed)
        self.assertFalse(got[1].burst_unclean)
        self.assertEqual(got[1].burst_retries, 0)
        self.assertIn(drained, b"".join(s.raw for s in got), "nothing is cut")


class CutRangesTests(unittest.TestCase):
    def test_cuts_and_offsets(self):
        raw, offs = replay.cut_ranges(b"0123456789", [0, 3, 5, 6, 9], [(3, 5), (7, 8)])
        self.assertEqual(raw, b"0125689")
        self.assertEqual(offs, [0, 3, 3, 4, 6])

    def test_no_cuts(self):
        self.assertEqual(replay.cut_ranges(b"abc", [1], []), (b"abc", [1]))


if __name__ == "__main__":
    unittest.main()
