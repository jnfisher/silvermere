"""Unit tests of smoke.py's parsing and telnet handling (no server needed).

python3 -I -m unittest discover -s tools/smoke -p 'test_*.py'
"""

import os
import socket
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import smoke  # noqa: E402

LOOK = (
    "look\r\nNewhaven, Village Entrance\r\n"
    "    Welcome to Newhaven! You are standing at the crude wooden gates.\r\n"
    "You notice large sign here.\r\n"
    "Obvious exits: north, south, west, southeast\r\n[HP=30]:"
)


class Parsing(unittest.TestCase):
    def test_room_name_is_the_line_before_the_description(self):
        self.assertEqual(smoke.room_name(LOOK), "Newhaven, Village Entrance")

    def test_room_name_without_a_description(self):
        out = "north\r\nNewhaven, Narrow Path\r\nAlso here: Nathaniel.\r\nObvious exits: east\r\n"
        self.assertEqual(smoke.room_name(out), "Newhaven, Narrow Path")

    def test_exits(self):
        self.assertEqual(smoke.parse_exits(LOOK), ["north", "south", "west", "southeast"])
        self.assertEqual(smoke.parse_exits("Obvious exits: NONE!!!\r\n"), ["none!!!"])
        self.assertEqual(smoke.parse_exits("no exits line"), [])

    def test_training_is_never_sent(self):
        a, b = socket.socketpair()
        t = smoke.Telnet.__new__(smoke.Telnet)
        t.sock, t.text, t.mark = a, "", 0
        for line in ("train stats", "TRAIN", "  tra"):
            with self.assertRaises(smoke.Failure):
                t.send_line(line)
        a.close()
        b.close()

    def test_login_fields_are_not_commands(self):
        a, b = socket.socketpair()
        t = smoke.Telnet.__new__(smoke.Telnet)
        t.sock, t.text, t.mark = a, "", 0
        t.send_line("travis", game=False)
        t.send_line("trapdoor1", game=False)
        self.assertEqual(b.recv(64), b"travis\r\ntrapdoor1\r\n")
        with self.assertRaises(smoke.Failure):
            t.send_line("trapdoor1")
        a.close()
        b.close()

    def test_the_password_never_prints(self):
        s = smoke.Smoke("h", 2323, "travis", "trapdoor1", 1.0)
        s.ok("logged in")
        conn = smoke.Telnet.__new__(smoke.Telnet)
        conn.text = "Password: trapdoor1\r\n"
        s.conn = conn
        out = smoke.failure_report(s, smoke.Failure("refusing to send 'trapdoor1'"))
        self.assertNotIn("trapdoor1", out)
        self.assertIn("refusing to send '********'", out)

    def test_targets(self):
        self.assertEqual(smoke.parse_target("example.org"), ("example.org", 2323))
        self.assertEqual(smoke.parse_target("example.org:23"), ("example.org", 23))
        self.assertEqual(smoke.parse_target("[2001:db8::1]:2323"), ("2001:db8::1", 2323))
        self.assertEqual(smoke.parse_target("[2001:db8::1]"), ("2001:db8::1", 2323))
        self.assertEqual(smoke.parse_target("2001:db8::1"), ("2001:db8::1", 2323))


class Negotiation(unittest.TestCase):
    def setUp(self):
        self.a, self.b = socket.socketpair()
        self.t = smoke.Telnet.__new__(smoke.Telnet)
        self.t.sock, self.t.text, self.t.mark, self.t._state = self.a, "", 0, "data"
        self.t._pending = ""
        self.t.timeout = 2

    def tearDown(self):
        self.a.close()
        self.b.close()

    def test_naws_is_79_columns_and_the_rest_refused(self):
        IAC, DO, WILL, SB, SE, WONT = 255, 253, 251, 250, 240, 252
        self.t._feed(bytes([IAC, DO, 31, IAC, DO, 24, IAC, WILL, 1]) + b"hi")
        self.b.settimeout(1)
        got = self.b.recv(64)
        self.assertEqual(got, bytes([IAC, WILL, 31, IAC, SB, 31, 0, 79, 0, 24, IAC, SE, IAC, WONT, 24]))
        self.assertEqual(self.t.text, "hi")

    def test_ansi_and_backspaces_are_taken_out(self):
        self.t._feed(b"\x1b[0;32mObvious exits: nA\x08orth, sX\x08outh\r\n\x1b[79D\x1b[K")
        self.assertEqual(self.t.text, "Obvious exits: north, south\r\n")

    def test_escapes_and_backspaces_split_across_chunks(self):
        self.t._feed(b"Obvious exits: nA\x1b[0;3")
        self.t._feed(b"2m\x08orth\r\nsX")
        self.t._feed(b"\x08outh")
        self.assertEqual(self.t.text, "Obvious exits: north\r\nsouth")

    def test_subnegotiation_is_skipped_across_chunks(self):
        self.t._feed(bytes([255, 250, 24, 1]))
        self.t._feed(bytes([255, 240]) + b"ok")
        self.assertEqual(self.t.text, "ok")

    def test_until_reports_a_closed_connection(self):
        self.b.sendall(b"Username:")
        self.assertTrue(self.t.until("Username:"))
        self.b.close()
        with self.assertRaises(smoke.Failure):
            self.t.until("Password:")


if __name__ == "__main__":
    unittest.main()
