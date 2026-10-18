import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

import known_gaps  # noqa: E402


def ev(step, name, line="", **fields):
    e = {"step": step, "name": name, "cat": "", "fields": fields}
    if name == "unknown":
        e["line"] = line
    return e


# Rules are copied here so the tests outlive the entries of known_gaps.json: the monster/NPC
# rule (T-035) was removed when that card merged, the `par` rule (T-043) goes with its card.
RULES = known_gaps.load(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                     "fixtures_known_gaps_rules.json"))


class KnownGaps(unittest.TestCase):
    rules = RULES

    def test_monster_presence_dropped_from_orig_only(self):
        events = [ev(1, "echo", cmd="n"), ev(1, "room_also_here", who="Nathaniel"),
                  ev(1, "unknown", "a guardsman just left to the east."),
                  ev(1, "unknown", "Welcome to Newhaven!"), ev(1, "room_exits")]
        kept, dropped = known_gaps.apply(events, "orig", self.rules)
        self.assertEqual([e["name"] for e in kept], ["echo", "unknown", "room_exits"])
        self.assertEqual(dropped, {"T-035": 2})
        kept, dropped = known_gaps.apply(events, "new", self.rules)
        self.assertEqual(len(kept), 5, "a new-side monster line is not forgiven")

    def test_party_is_scoped_to_the_command(self):
        orig = [ev(3, "echo", cmd="par"), ev(3, "party_none"), ev(3, "unknown", "x"),
                ev(4, "echo", cmd="top"), ev(4, "party_none")]
        kept, dropped = known_gaps.apply(orig, "orig", self.rules)
        self.assertEqual([e["step"] for e in kept], [3, 4, 4])
        self.assertEqual(dropped, {"T-043": 2})
        new = [ev(3, "echo", cmd="par"), ev(3, "syntax", syntax="PARTY")]
        kept, dropped = known_gaps.apply(new, "new", self.rules)
        self.assertEqual([e["name"] for e in kept], ["echo"])

    def test_byte_mask_removes_the_orig_monster_line_and_its_reset_only_on_orig(self):
        text = ("\x1b[79D\x1b[K\x1b[0;32mObvious exits: north\r\n"
                "\x1b[79D\x1b[K\x1b[0;37mAlso here: \x1b[1;35mfat giant\x1b[0;37m, "
                "\x1b[0;35mrat\x1b[0m.\r\n\x1b[0m\x1b[79D\x1b[K\x1b[0;37m[HP=35]:")
        want = ("\x1b[79D\x1b[K\x1b[0;32mObvious exits: north\r\n"
                "\x1b[79D\x1b[K\x1b[0;37m[HP=35]:")
        self.assertEqual(known_gaps.mask_bytes(text, "orig", self.rules), want)
        self.assertEqual(known_gaps.mask_bytes(text, "new", self.rules), text)

    def test_noise_and_its_prompt_go_from_both_sides_only_that_prompt(self):
        events = [ev(2, "echo", cmd="n"), ev(2, "noise", dir="north"), ev(2, "prompt", hp="35"),
                  ev(2, "unknown", "Newhaven, Weapons Shop"), ev(2, "prompt", hp="35"),
                  ev(3, "noise", dir="up")]
        for side in ("orig", "new"):
            kept, dropped = known_gaps.apply(events, side, self.rules)
            self.assertEqual([e["name"] for e in kept], ["echo", "unknown", "prompt"], side)
            self.assertEqual(dropped, {"T-040": 3}, side)

    def test_a_prompt_that_does_not_follow_a_noise_stays(self):
        events = [ev(1, "prompt", hp="35"), ev(1, "noise", dir="north"), ev(1, "echo", cmd="e"),
                  ev(1, "prompt", hp="35")]
        kept, dropped = known_gaps.apply(events, "new", self.rules)
        self.assertEqual([e["name"] for e in kept], ["prompt", "echo", "prompt"])

    def test_a_noise_last_in_its_step_takes_the_prompt_it_erased(self):
        # m4/sysop step 15 on a live run: the step's own prompt, then the noise, which erased
        # it (ANSI_CLRLINE) and drew it again on the next step's echo line
        events = [ev(15, "prompt", hp="100"), ev(15, "echo", cmd="sysop goto khazarad"),
                  ev(15, "prompt", hp="100"), ev(15, "noise", dir="north"),
                  ev(16, "prompt", hp="100"), ev(16, "echo", cmd="sysop goto lostcity")]
        kept, dropped = known_gaps.apply(events, "new", self.rules)
        self.assertEqual([(e["step"], e["name"]) for e in kept],
                         [(15, "prompt"), (15, "echo"), (16, "prompt"), (16, "echo")])
        self.assertEqual(dropped, {"T-040": 2})
        # the last event of all: the same
        kept, dropped = known_gaps.apply(events[:4], "new", self.rules)
        self.assertEqual([e["name"] for e in kept], ["prompt", "echo"])
        self.assertEqual(dropped, {"T-040": 2})
        # no prompt before it in its step: nothing more goes
        kept, dropped = known_gaps.apply([ev(15, "echo", cmd="e"), ev(15, "noise", dir="up"),
                                          ev(16, "prompt", hp="100")], "new", self.rules)
        self.assertEqual([e["name"] for e in kept], ["echo", "prompt"])
        self.assertEqual(dropped, {"T-040": 1})

    def test_noise_bytes_go_with_their_prompt_and_leave_the_room_alone(self):
        text = ("n\r\n\x1b[79D\x1b[K\x1b[0;35mYou hear movement to the north.\r\n"
                "\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:\x1b[0;37;40m\x1b[79D\x1b[K"
                "\x1b[1;36mNewhaven, Weapons Shop\r\n")
        want = "n\r\n\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;36mNewhaven, Weapons Shop\r\n"
        for side in ("orig", "new"):
            self.assertEqual(known_gaps.mask_bytes(text, side, self.rules), want)

    def test_noise_bytes_with_the_scrambled_direction_word(self):
        # the original puts a random letter and a backspace inside the direction word
        for word in ("wA\x08est", "n \x08orth", "s?\x08outh", "northeaZ\x08st"):
            text = ("w\r\n\x1b[79D\x1b[K\x1b[0;35mYou hear movement to the %s.\r\n"
                    "\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:\x1b[0;37;40m\x1b[79D\x1b[K"
                    "\x1b[1;36mNewhaven, Healer\r\n" % word)
            want = "w\r\n\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;36mNewhaven, Healer\r\n"
            for side in ("orig", "new"):
                self.assertEqual(known_gaps.mask_bytes(text, side, self.rules), want, word)

    def test_a_prompt_in_the_next_step_is_not_the_noise_s(self):
        # the noise is the last event of step 1; step 2 (a bare Enter) starts with its own prompt
        events = [ev(1, "echo", cmd="n"), ev(1, "noise", dir="north"),
                  ev(2, "prompt", hp="35"), ev(2, "unknown", "Newhaven, Healer")]
        kept, dropped = known_gaps.apply(events, "new", self.rules)
        self.assertEqual([e["name"] for e in kept], ["echo", "prompt", "unknown"])
        self.assertEqual(dropped, {"T-040": 1})

    def test_byte_mask_leaves_plain_lines_alone(self):
        text = "Welcome to Newhaven! You are standing at the gates.\r\n"
        self.assertEqual(known_gaps.mask_bytes(text, "orig", self.rules), text)


class RandomArrivals(unittest.TestCase):
    rule = [r for r in known_gaps.load_random() if r["card"] == "random: monster arrival"]

    def test_only_for_statistical_scenarios(self):
        self.assertEqual(len(self.rule), 1)
        self.assertTrue(self.rule[0]["statistical_only"])

    def test_arrival_events_and_lines_dropped(self):
        events = [ev(5, "echo", cmd="st"),
                  ev(5, "monster_arrives", mon="thin giant rat", frm="nowhere"),
                  ev(5, "monster_arrives_generic", mon="lashworm", dir="east"), ev(5, "prompt"),
                  ev(5, "unknown", "A thin giant rat creeps into the room from nowhere?"),
                  ev(5, "stat_name")]
        kept, dropped = known_gaps.apply(events, "new", self.rule)
        # a text the game data does not have stays unclassified (text fidelity)
        self.assertEqual([e["name"] for e in kept], ["echo", "unknown", "stat_name"])
        self.assertEqual(dropped, {"random: monster arrival": 3})

    def test_arrival_bytes_with_their_prompt(self):
        text = ("st\r\n\x1b[79D\x1b[K\x1b[1;33mA thin giant rat\x1b[0;32m creeps into the room "
                "from nowhere.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:Name: Fennick")
        self.assertEqual(known_gaps.mask_bytes(text, "new", self.rule), "st\r\nName: Fennick")
        orig = ("\x1b[79D\x1b[K\x1b[1;33mgiant rat\x1b[0;32m moves into the room from the "
                "nA\x08orth.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:")
        self.assertEqual(known_gaps.mask_bytes(orig, "orig", self.rule), "")


class MbbsemuPadding(unittest.TestCase):
    """The real known_gaps.json rule for MBBSEmu's printf ignoring the `0` flag."""
    rules = known_gaps.load()

    def test_cents_are_normalised_on_both_sides_and_the_rest_is_still_compared(self):
        orig = [ev(1, "bank_balance", dep="0 copper farthings [0. 0 gold crowns]")]
        new = [ev(1, "bank_balance", dep="0 copper farthings [0.00 gold crowns]")]
        o, _ = known_gaps.apply(orig, "orig", self.rules)
        n, _ = known_gaps.apply(new, "new", self.rules)
        self.assertEqual(o[0]["fields"], n[0]["fields"])
        other = [ev(1, "bank_balance", dep="5 copper farthings [0.05 gold crowns]")]
        n2, _ = known_gaps.apply(other, "new", self.rules)
        self.assertNotEqual(o[0]["fields"], n2[0]["fields"])

    def test_byte_check_ignores_only_the_cents_digits(self):
        a = "On deposit: 0 copper farthings [\x1b[1;37m0. 0 \x1b[0mgold crowns]\r\n"
        b = "On deposit: 0 copper farthings [\x1b[1;37m0.00 \x1b[0mgold crowns]\r\n"
        self.assertEqual(known_gaps.mask_bytes(a, "orig", self.rules),
                         known_gaps.mask_bytes(b, "new", self.rules))
        c = "On deposit: 5 copper farthings [\x1b[1;37m0.05 \x1b[0mgold crowns]\r\n"
        self.assertNotEqual(known_gaps.mask_bytes(a, "orig", self.rules),
                            known_gaps.mask_bytes(c, "new", self.rules))

    def test_both_kinds_of_normalisation_are_counted_under_the_card(self):
        orig = [ev(1, "bank_balance", dep="0 copper farthings [0. 0 gold crowns]"),
                ev(2, "bank_balance", dep="0 copper farthings [0. 0 gold crowns]")]
        _, hits = known_gaps.apply(orig, "orig", self.rules)
        self.assertEqual(hits, {"MBBSEmu": 2})
        _, none = known_gaps.apply([ev(1, "bank_balance", dep="[0.00 gold crowns]")], "new",
                                   self.rules)
        self.assertEqual(none, {})
        bytes_hits = {}
        known_gaps.mask_bytes("[\x1b[1;37m0. 0 \x1b[0mgold crowns]", "orig", self.rules, bytes_hits)
        self.assertEqual(bytes_hits, {"MBBSEmu": 1})

    def test_the_summary_lists_the_counts(self):
        import tempfile
        import report
        result = {"scenario": "m3/bank", "status": "pass", "event_diffs": 0, "unknown_extra": [],
                  "byte": {"equal": 1, "checked": 1}, "problems": [],
                  "known_gaps": {"MBBSEmu": {"orig": 3, "new": 0}}}
        with tempfile.TemporaryDirectory() as d:
            report.write_summary(d, [result])
            text = open(os.path.join(d, "summary.md"), encoding="utf-8").read()
        self.assertIn("MBBSEmu: 3/0", text)


if __name__ == "__main__":
    unittest.main()
