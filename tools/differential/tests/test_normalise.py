"""Mask handling of normalise.py (the event masks themselves live in tests/fixtures/masks.json)."""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import normalise  # noqa: E402
import replay  # noqa: E402



class ExitProgress(unittest.TestCase):
    """The silent-meditation exit prints one line of 16 dots (the first at once,
    then three per fast tick); the registry's loose `chargen_values` pattern matched
    it. It is its own event now, one per line: the bridge joins every read into one stream, so
    reads cannot cut it, and a line break among the dots must show."""

    HEAD = b"You will exit after a period of silent meditation.\r\n"
    TAIL = (b"\x1b[0m\r\nYour character has been saved. If you have any comments or "
            b"suggestions, please\r\n")

    def names(self, raw):
        return [e["name"] for e in normalise.events_for(raw, [], normalise.load_masks())]

    def test_the_dot_line_is_one_exit_progress_event(self):
        names = self.names(self.HEAD + b"................" + self.TAIL)
        self.assertEqual(names.count("exit_progress"), 1)
        self.assertNotIn("chargen_values", names)

    def test_a_line_break_among_the_dots_is_a_second_event(self):
        whole = self.names(self.HEAD + b"................" + self.TAIL)
        broken = self.names(self.HEAD + b"........\r\n........" + self.TAIL)
        self.assertEqual(broken.count("exit_progress"), 2)
        self.assertNotEqual(whole, broken)

    def test_other_chargen_value_lines_stay(self):
        self.assertEqual(self.names(b"40   50   none   black\r\n"), ["chargen_values"])


class MaskTests(unittest.TestCase):
    def test_statistical_masks_add_the_prompt_hp(self):
        m = normalise.load_masks()
        self.assertNotIn("prompt", m["events"])
        s = normalise.with_statistical_events(m)
        self.assertIn("hp", s["events"]["prompt"])
        self.assertTrue(normalise._masked(s, "prompt", "hp"))
        self.assertFalse(normalise._masked(m, "prompt", "hp"))

    def test_masked_field_keeps_wording_checks(self):
        m = normalise.load_masks()
        raw = b"You smack rat for 7 damage!\r\n"
        a = normalise.events_for(raw, [], m)
        b = normalise.events_for(raw.replace(b"7", b"9"), [], m)
        self.assertEqual(a, b)
        c = normalise.events_for(raw.replace(b"rat", b"bat"), [], m)
        self.assertNotEqual(a, c)


class TwoConnectionTests(unittest.TestCase):
    def test_each_connections_events_keep_their_own_steps(self):
        # B's offsets start again at 0; joined with A's bytes they would put its markers in A's text
        a = [replay.StepCapture(0, "send", "hi", 0, b"[HP=35]:hi\r\nYou say \"hi\"\r\n[HP=35]:"),
             replay.StepCapture(2, "send", "bow", 40, b"bow\r\nYou bow.\r\n[HP=35]:")]
        b = [replay.StepCapture(1, "send", "smile", 0, b"[HP=35]:smile\r\nYou smile happily!\r\n[HP=35]:",
                                client="B")]
        ev = normalise.events_of([a[0], b[0], a[1]], normalise.load_masks())
        by = {(e["name"], e.get("line")): e["step"] for e in ev}
        self.assertEqual(by[("you_say", None)], 0)
        self.assertEqual(by[("unknown", "You smile happily!")], 1)
        self.assertEqual(by[("unknown", "You bow.")], 2)
        self.assertEqual([e["step"] for e in ev], sorted(e["step"] for e in ev))


if __name__ == "__main__":
    unittest.main()
