import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

import differential  # noqa: E402
import normalise  # noqa: E402
import replay  # noqa: E402


def step(i, op, send, offset, raw):
    return replay.StepCapture(i, op, send, offset, raw)


# The two rules m4/aoe carried for T-093, kept here as fixtures: the only tests of after_echo,
# then_drop_one and the command-scoped unknown_lines of known_gaps.py.
T093_RULES = json.loads(r'''[
 {
  "card": "T-093",
  "side": "orig",
  "command": "x",
  "why": "fixture: the rule m4/aoe carried for T-093; kept as the test of after_echo, then_drop_one and command-scoped unknown_lines",
  "after_echo": [
   "combat_off",
   "prompt"
  ],
  "byte_patterns": [
   "(?:\\x1b\\[79D\\x1b\\[K)?(?:\\x1b\\[[0-9;]*m)*\\*Combat Off\\*\\r\\n(?:\\x1b\\[79D\\x1b\\[K)?(?:\\x1b\\[[0-9;]*m)*\\[HP=[^\\]\\r\\n]*\\]:(?=You will exit after a period of silent meditation)"
  ]
 },
 {
  "card": "T-093",
  "side": "new",
  "command": "x",
  "why": "fixture: the rule m4/aoe carried for T-093; kept as the test of after_echo, then_drop_one and command-scoped unknown_lines",
  "unknown_lines": [
   "^\\.*Your spell has no effect in this room!\\.*$"
  ],
  "byte_patterns": [
   "(?<=\\.)Your spell has no effect in this room!\\r?\\n(?:\\x1b\\[79D\\x1b\\[K)?(?:\\x1b\\[[0-9;]*m)*(?:\\[HP=[^\\]\\r\\n]*\\]:)?(?=\\.*(?:\\x1b\\[[0-9;]*m)*\\r?\\nYour character has been saved)"
  ],
  "then_drop_one": [
   "prompt"
  ]
 }
]''')


class ScenarioOptions(unittest.TestCase):
    def test_merge_steps_collapses_a_burst_into_its_first_step(self):
        steps = [step(5, "send", "d", 0, b"a"), step(6, "send", "search", 1, b"b"),
                 step(7, "send", "exp", 2, b"c"), step(8, "send", "exp", 3, b"d"),
                 step(9, "send", "w", 4, b"e")]
        out = differential.merge_steps(steps, [[6, 8]])
        self.assertEqual([(s.index, s.raw) for s in out], [(5, b"a"), (6, b"bcd"), (9, b"e")])
        self.assertEqual(steps[1].raw, b"b", "the input steps are not modified")

    def test_a_connect_step_owns_the_banner_that_follows_it(self):
        steps = [step(0, "connect", None, 0, b"BANNER\r\n"),
                 step(1, "send", "Aldric", 8, b"Aldric\r\nPassword:")]
        sends = normalise.sends_of(steps)
        self.assertEqual(sends, [(0, 0, None), (1, 8, "Aldric")])
        raw = b"".join(s.raw for s in steps)
        _, steps_of_lines = normalise.combined_lines(raw, sends)
        self.assertEqual(steps_of_lines[0], 0, "the banner line is the connect step's")
        self.assertIn(1, steps_of_lines)

    def test_an_echo_classifies_the_same_wherever_the_send_offset_landed(self):
        raw = (b"Your search is over.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:search\r\n"
               b"\x1b[0;36mYour search revealed nothing.\r\n")
        at = raw.find(b"search\r")
        masks = normalise.load_masks()
        seen = set()
        for off in range(at - 8, at + 12):     # before, inside and after the echo
            ev = normalise.events_for(raw, [(1, off, "search")], masks)
            seen.add(tuple(e["name"] for e in ev if e["name"] in ("echo", "unknown")))
        self.assertEqual(len(seen), 1, seen)
        self.assertIn("echo", next(iter(seen)))


    def _sc(self, **kw):
        sc = {"name": "t/opts", "script": "/nonexistent.json", "ignore_host_banner": True,
              "deterministic": []}
        sc.update(kw)
        return sc

    def _run(self, sc, orig, new):
        masks = normalise.load_masks()
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        return differential.compare(sc, mk(orig), mk(new), masks)[0]

    def test_random_steps_are_not_diffed_as_events(self):
        orig = b"x\r\n*Combat Engaged*\r\n[HP=10]:"
        new = b"x\r\n*Combat Off*\r\n[HP=10]:"
        self.assertEqual(self._run(self._sc(), orig, new)["status"], "fail")
        res = self._run(self._sc(random_steps=[1]), orig, new)
        self.assertEqual(res["status"], "pass", res)

    def test_random_steps_still_count_unknown_lines(self):
        orig = b"x\r\n[HP=10]:"
        new = b"x\r\nQqq zzz never a pattern line.\r\n[HP=10]:"
        res = self._run(self._sc(random_steps=[1]), orig, new)
        self.assertEqual(res["status"], "fail")
        self.assertTrue(res["unknown_extra"], res)

    def test_divergences_drop_a_decided_line_in_both_layers(self):
        line = b"  INITIALIZE-- Re-initialize MajorMUD buffers\r\n"
        orig = b"x\r\nSysop sub-commands:\r\n" + line + b"[HP=10]:"
        new = b"x\r\nSysop sub-commands:\r\n[HP=10]:"
        sc = self._sc(deterministic=["x"])
        self.assertEqual(self._run(sc, orig, new)["status"], "fail")
        sc["divergences"] = [{"card": "T-055", "side": "orig",
                              "unknown_lines": [r"^\s*INITIALIZE-- Re-initialize MajorMUD buffers\s*$"],
                              "byte_patterns": [r"[^\r\n]*INITIALIZE-- Re-initialize MajorMUD buffers[^\r\n]*\r?\n?"]}]
        res = self._run(sc, orig, new)
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["byte"]["checked"], 1)

    def test_required_events_must_show_on_both_sides(self):
        orig = b"x\r\n*Combat Engaged*\r\n[HP=10]:"
        new = b"x\r\n*Combat Off*\r\n[HP=10]:"
        sc = self._sc(random_steps=[1], required_events={"1": ["combat_off"]})
        res = self._run(sc, orig, new)
        self.assertEqual(res["status"], "fail")
        self.assertIn("orig: step 1 lacks combat_off", res["problems"])
        res = self._run(sc, new, new)
        self.assertEqual(res["status"], "pass", res)


    def test_value_masks_mask_one_settings_line(self):
        orig = b"x\r\nLife for this CHAR  2 minutes\r\nDisplay Mode:       Verbose\r\n[HP=10]:"
        new = b"x\r\nLife for this CHAR  5 minutes\r\nDisplay Mode:       Verbose\r\n[HP=10]:"
        self.assertEqual(self._run(self._sc(), orig, new)["status"], "fail")
        sc = self._sc(value_masks=[{"event": "settings_line", "where": {"key": "Life for this CHAR"},
                                    "fields": ["val"]}])
        self.assertEqual(self._run(sc, orig, new)["status"], "pass")
        other = new.replace(b"Verbose", b"Brief")
        self.assertEqual(self._run(sc, orig, other)["status"], "fail", "other lines stay compared")
        # only the digits: the words around the number stay compared
        hours = new.replace(b"5 minutes", b"5 hours")
        self.assertEqual(self._run(sc, orig, hours)["status"], "fail", "the unit is compared")

    def test_byte_gaps_are_counted_and_shown(self):
        import report
        line = b"  INITIALIZE-- Re-initialize MajorMUD buffers\r\n"
        orig = b"x\r\nSysop sub-commands:\r\n" + line + b"[HP=10]:"
        new = b"x\r\nSysop sub-commands:\r\n[HP=10]:"
        sc = self._sc(deterministic=["x"], divergences=[{
            "card": "T-055", "side": "orig",
            "unknown_lines": [r"^\s*INITIALIZE-- Re-initialize MajorMUD buffers\s*$"],
            "byte_patterns": [r"[^\r\n]*INITIALIZE-- Re-initialize MajorMUD buffers[^\r\n]*\r?\n?"]}])
        res = self._run(sc, orig, new)
        self.assertEqual(res["known_gaps"]["T-055"], {"orig": 1, "new": 0, "orig_bytes": 1})
        self.assertEqual(report.gaps_cell(res["known_gaps"]), "T-055: 1/0 +1/0")

    def test_wanderers_skip_summons_and_fight_steps(self):
        steps = [step(1, "send", "sysop monster 7", 0, b""), step(2, "send", "attack", 0, b""),
                 step(3, "send", "rest", 0, b"")]
        ev = lambda st, name: {"step": st, "name": name, "fields": {}}
        oe = [ev(1, "monster_arrives"), ev(2, "mon_hits_you"), ev(3, "mon_dodge_you")]
        ne = [ev(3, "monster_arrives"), ev(3, "monster_death")]
        hits = differential.wanderers({"random_steps": [2]}, steps, oe, ne)
        self.assertEqual(hits, ["orig step 3: mon_dodge_you", "new step 3: monster_arrives"])

    def test_a_wanderer_failure_is_rerun_once_and_recorded(self):
        import tempfile, types
        clean = b"x\r\nYou are now resting.\r\n[HP=10]:"
        wander = b"x\r\nA giant rat moves into the room from the east.\r\nYou are now resting.\r\n[HP=10]:"
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        plays = {"orig": [mk(wander), mk(clean)], "new": [mk(clean), mk(clean)]}
        sc = self._sc()
        saved = (differential.side_steps, differential.load_scenario)
        try:
            differential.side_steps = lambda side, src, scn, setup: plays[side].pop(0)
            differential.load_scenario = lambda name: dict(sc, name=name)
            with tempfile.TemporaryDirectory() as d:
                args = types.SimpleNamespace(include_statistical=False, orig=None, new=None,
                                             no_setup=True, report=d)
                res, _ = differential.run_one("t/rerun", args, normalise.load_masks())
                self.assertTrue(os.path.isfile(os.path.join(d, "t/rerun/attempt1/diff.txt")))
        finally:
            differential.side_steps, differential.load_scenario = saved
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["rerun"]["status"], "fail")
        self.assertTrue(res["rerun"]["wanderers"][0].startswith("orig step 1: monster_arrives_generic"))
        self.assertTrue(any("rerun once after interference" in n for n in res["notes"]))

    def test_a_scenario_that_cannot_start_again_is_not_rerun(self):
        """`no_rerun` (m3/bank: its character is not where it began): the interference is noted
        and the first attempt's failure stands; side_steps is called once per side."""
        import tempfile, types
        clean = b"x\r\nYou are now resting.\r\n[HP=10]:"
        wander = b"x\r\nA giant rat moves into the room from the east.\r\nYou are now resting.\r\n[HP=10]:"
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        calls = []

        def side_steps(side, src, scn, setup):
            calls.append(side)
            return mk(wander if side == "orig" else clean)
        sc = self._sc(no_rerun=True)
        saved = (differential.side_steps, differential.load_scenario)
        try:
            differential.side_steps = side_steps
            differential.load_scenario = lambda name: dict(sc, name=name)
            with tempfile.TemporaryDirectory() as d:
                args = types.SimpleNamespace(include_statistical=False, orig=None, new=None,
                                             no_setup=True, report=d)
                res, _ = differential.run_one("t/norerun", args, normalise.load_masks())
        finally:
            differential.side_steps, differential.load_scenario = saved
        self.assertEqual(sorted(calls), ["new", "orig"])
        self.assertEqual(res["status"], "fail")
        self.assertNotIn("rerun", res)
        self.assertTrue(any("not rerun (the scenario cannot start again)" in n for n in res["notes"]),
                        res["notes"])

    def test_an_exact_prompt_redraw_is_collapsed_and_counted(self):
        import diff
        P = "\x1b[79D\x1b[K\x1b[0;37m[HP=10]:"
        c = {}
        self.assertEqual(diff.collapse_prompt_redraws("a\r\n" + P + P + P, c), "a\r\n" + P)
        self.assertEqual(c, {"prompt_redraws": 2})
        other = "a\r\n" + P + "\x1b[79D\x1b[K\x1b[0;37m[HP=9]:"
        self.assertEqual(diff.collapse_prompt_redraws(other), other, "a changed prompt stays")
        between = "a\r\n" + P + "b\r\n" + P
        self.assertEqual(diff.collapse_prompt_redraws(between), between, "text between stays")
        o = b"x\r\ntorch conjured.\r\n[HP=10]:"
        n = b"x\r\ntorch conjured.\r\n[HP=10]:[HP=10]:"
        res = self._run(self._sc(deterministic=["x"]), o, n)
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["redraws"]["new"], {"events": 1, "bytes": 1, "rejoined": 0})
        self.assertEqual(res["redraws"]["orig"], {"events": 0, "bytes": 0, "rejoined": 0})
        import report
        self.assertEqual(report.redraws_cell(res["redraws"]), "0/2")
        self.assertEqual(res["notes"], [], "under 3: not flagged")
        many = b"x\r\ntorch conjured.\r\n[HP=10]:[HP=10]:[HP=10]:[HP=10]:"
        res = self._run(self._sc(deterministic=["x"]), o, many)
        self.assertEqual(res["status"], "fail", "a lopsided count fails the scenario")
        self.assertTrue(any("prompt redraws differ" in x for x in res["problems"]), res["problems"])
        # a random scenario: the count follows the random fight's length (run 39035493410), a note
        res = self._run(self._sc(m2={"sequence": []}), o, many)
        self.assertFalse(any("prompt redraws differ" in x for x in res["problems"]), res["problems"])
        self.assertTrue(any("prompt redraws differ" in x for x in res["notes"]), res["notes"])

    def test_an_effect_wearing_off_is_interference(self):
        steps = [step(1, "send", "rest", 0, b"")]
        oe = [{"step": 1, "name": "wccmsg", "fields": {"msgnum": "2678"}},
              {"step": 1, "name": "wccmsg", "fields": {"msgnum": "100"}}]
        self.assertEqual(differential.wanderers({}, steps, oe, []), ["orig step 1: wccmsg 2678"])

    def _rerun(self, plays):
        import tempfile, types
        sc = self._sc(deterministic=["x", "y"])
        saved = (differential.side_steps, differential.load_scenario)
        try:
            differential.side_steps = lambda side, src, scn, setup: plays[side].pop(0)
            differential.load_scenario = lambda name: dict(sc, name=name)
            with tempfile.TemporaryDirectory() as d:
                args = types.SimpleNamespace(include_statistical=False, orig=None, new=None,
                                             no_setup=True, report=d)
                return differential.run_one("t/rerun2", args, normalise.load_masks())[0]
        finally:
            differential.side_steps, differential.load_scenario = saved

    def test_no_rerun_when_the_failure_comes_before_the_interference(self):
        mk = lambda a, b: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, a),
                           step(2, "send", "y", 4 + len(a), b)]
        ok = b"x\r\nYou are now resting.\r\n[HP=10]:"
        bad = b"x\r\nYou are now standing.\r\n[HP=10]:"
        rat = b"y\r\nA giant rat moves into the room from the east.\r\n[HP=10]:"
        quiet = b"y\r\n[HP=10]:"
        # step 1 differs, the rat comes at step 2: the scenario's own failure, not rerun
        plays = {"orig": [mk(ok, rat)], "new": [mk(bad, quiet)]}
        res = self._rerun(plays)
        self.assertEqual(res["status"], "fail")
        self.assertNotIn("rerun", res)
        self.assertTrue(any("not rerun" in n for n in res["notes"]), res["notes"])
        # the rat at step 1 and the failure there or after: rerun once
        rat_at_1 = b"x\r\nA giant rat moves into the room from the east.\r\nYou are now resting.\r\n[HP=10]:"
        plays = {"orig": [mk(rat_at_1, quiet), mk(ok, quiet)],
                 "new": [mk(ok, quiet), mk(ok, quiet)]}
        res = self._rerun(plays)
        self.assertEqual(res["status"], "pass", res)
        self.assertIn("rerun", res)

    REDRAW = b"\x1b[79D\x1b[K\x1b[0;37m[HP=300\x1b[0;37m/MA=2\x1b[0;37m]:"

    def test_an_echo_split_by_a_prompt_redraw_is_rejoined(self):
        """m4/spells step 13 (run 38911979888): the slow tick's redraw cut the echo in two."""
        split = b"use scroll of vine s" + self.REDRAW + b"trike\r\nYou read the scroll.\r\n[HP=300/MA=2]:"
        whole = b"use scroll of vine strike\r\nYou read the scroll.\r\n[HP=300/MA=0]:"
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"),
                          step(1, "send", "use scroll of vine strike", 4, raw),
                          step(2, "send", "y", 4 + len(raw), b"y\r\n[HP=300]:")]
        res = differential.compare(self._sc(deterministic=[], mask_prompt=True), mk(split), mk(whole),
                                   normalise.load_masks())[0]
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["redraws"]["orig"]["rejoined"], 1)
        self.assertEqual(res["redraws"]["new"]["rejoined"], 0)
        self.assertTrue(any("step 1 cut by a prompt redraw, rejoined" in n for n in res["notes"]))
        # only the two shapes the original can produce are rejoined
        one = normalise.rejoin_split_lines
        self.assertEqual(one(b"[HP=1]:USE SCR" + self.REDRAW + b"OLL\r\n", "use scroll"),
                         (b"[HP=1]:USE SCROLL\r\n", 1), "case-insensitive prefix of the send")
        raw = b"a\r\n" + self.REDRAW + b"x\r\n"
        self.assertEqual(one(raw, "x"), (raw, 0), "a prompt at a line start stays")
        raw = b"done." + self.REDRAW + b"\r\n"
        # the whole command, then the redraw, then its CR echo: a tick between the last byte and
        # the CR, which both servers allow (T-094)
        self.assertEqual(one(raw, "done."), (b"done.\r\n", 1), "the whole send, then the line end")
        self.assertEqual(one(raw, "look"), (raw, 0), "not the step's send: it stays")

    def test_the_captured_line_start_prompt_shape_rejoins_one_or_two_redraws(self):
        one = normalise.rejoin_split_lines
        pre = b"\x1b[79D\x1b[K\x1b[0;37m[HP=300/MA=2]:"
        self.assertEqual(one(pre + b"use scr" + self.REDRAW + b"oll of vine strike\r\n",
                             "use scroll of vine strike"),
                         (pre + b"use scroll of vine strike\r\n", 1))
        self.assertEqual(one(pre + b"use s" + self.REDRAW + b"cro" + self.REDRAW + b"ll\r\n", "use scroll"),
                         (pre + b"use scroll\r\n", 2))
        # after the whole command, before its CR echo (a tick between the last byte and the CR)
        self.assertEqual(one(pre + b"use scroll" + self.REDRAW + b"\r\nYou read it.\r\n", "use scroll"),
                         (pre + b"use scroll\r\nYou read it.\r\n", 1))
        # but not when something other than the line end follows it
        tail = pre + b"use scroll" + self.REDRAW + b"You read it.\r\n"
        self.assertEqual(one(tail, "use scroll"), (tail, 0))

    def test_a_redraw_between_the_complete_echo_and_the_output_is_not_rejoined_and_fails(self):
        """m4/use step 24 (run 38920877617): a tick between the CR's echo and the line's output.
        The original cannot (MbbsHost.cs:426, :929, :458-459), and since T-094 silvermere cannot
        either, so the shape is a difference on either side, never rejoined or counted as a gap."""
        one = normalise.rejoin_split_lines
        P = b"\x1b[79D\x1b[K\x1b[0;37m[HP=100\x1b[0;37m/MA=50\x1b[0;37m]:"
        R = b"\x1b[79D\x1b[K\x1b[0;37m[HP=36\x1b[0;37m/MA=50\x1b[0;37m]:"
        send = "sysop summon minor healing potion"
        cut = P + send.encode() + b"\r\n" + R + b"\x1b[0;37;40mminor healing potion conjured.\r\n" + P
        whole = P + send.encode() + b"\r\n\x1b[0;37;40mminor healing potion conjured.\r\n" + P
        self.assertEqual(one(cut, send), (cut, 0), "not an echo or dots split")
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", send, 4, raw)]
        sc = self._sc(deterministic=[send], mask_prompt=True)
        masks = normalise.load_masks()
        for orig, new in ((whole, cut), (cut, whole)):
            res = differential.compare(sc, mk(orig), mk(new), masks)[0]
            self.assertEqual(res["status"], "fail", res)
            self.assertNotIn("T-094", res["known_gaps"])

    def test_a_redraw_inside_a_message_is_not_rejoined_and_fails_the_scenario(self):
        """(B) `You lit ` + redraw + `the torch.` is a real difference the harness must keep."""
        cut = b"light\r\nYou lit " + self.REDRAW + b"the torch.\r\n[HP=300/MA=2]:"
        whole = b"light\r\nYou lit the torch.\r\n[HP=300/MA=2]:"
        self.assertEqual(normalise.rejoin_split_lines(cut, "light"), (cut, 0))
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "light", 4, raw)]
        res = differential.compare(self._sc(deterministic=["light"]), mk(cut), mk(whole),
                                   normalise.load_masks())[0]
        self.assertEqual(res["status"], "fail", res)
        # nor are dots with text in them, or a send that the text does not spell
        dots = b"x\r\n.." + self.REDRAW + b"..a\r\n"
        self.assertEqual(normalise.rejoin_split_lines(dots, "x"), (dots, 0))
        wrong = b"[HP=1]:use sc" + self.REDRAW + b"xyz\r\n"
        self.assertEqual(normalise.rejoin_split_lines(wrong, "use scroll"), (wrong, 0))

    def test_the_exit_dots_split_by_a_redraw_are_rejoined_and_offsets_follow(self):
        dots = b"x\r\nYou will exit after a period of silent meditation.\r\n......" + self.REDRAW + b"..........\r\nSaved.\r\n"
        steps = [step(0, "send", "x", 0, dots), step(1, "send", "y", len(dots), b"y\r\n[HP=1]:")]
        out, counts = normalise.rejoin_steps(steps)
        self.assertEqual(counts, {0: 1})
        self.assertIn(b"................\r\nSaved.", out[0].raw)
        self.assertEqual(out[1].offset, len(out[0].raw), "the next step starts where the rejoined one ends")
        self.assertEqual(steps[0].raw, dots, "the captured bytes are not modified")

    def test_the_echo_wait_tolerates_a_redraw_between_any_two_characters(self):
        import re
        st = [{"send": "E", "until": r"\[HP=1\]:"}, {"send": "use scroll", "until": r"\[HP=1\]:"}]
        replay.anchor_on_echo(st)
        rx = re.compile(st[1]["until"])
        self.assertTrue(rx.search("use scroll\r\nok\r\n[HP=1]:"))
        self.assertTrue(rx.search("use s[HP=300/MA=2]:croll\r\nok\r\n[HP=1]:"))
        self.assertIsNone(rx.search("[HP=1]:\r\n[HP=1]:"), "a stale prompt is still not the echo")

    def test_a_temple_ambient_line_is_interference_and_the_wear_off_too(self):
        ev = lambda st, name, **f: {"step": st, "name": name, "fields": f}
        steps = [step(1, "send", "x", 0, b"")]
        for num in ("2676", "2677", "2678", "2296"):  # 2296 wear-off; 2676-2678 Temple
            hits = differential.wanderers({}, steps, [], [ev(1, "wccmsg", msgnum=num)])
            self.assertEqual(hits, [f"new step 1: wccmsg {num}"])
        self.assertEqual(differential.wanderers({}, steps, [], [ev(1, "wccmsg", msgnum="14")]), [])

    def test_max_reruns_plays_a_scenario_again_while_wanderers_keep_coming(self):
        import tempfile, types
        clean = b"x\r\nYou are now resting.\r\n[HP=10]:"
        wander = b"x\r\nA giant rat moves into the room from the east.\r\nYou are now resting.\r\n[HP=10]:"
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]

        def play(orig_runs, max_reruns):
            plays = {"orig": [mk(r) for r in orig_runs], "new": [mk(clean) for _ in orig_runs]}
            sc = self._sc(max_reruns=max_reruns)
            saved = (differential.side_steps, differential.load_scenario)
            try:
                differential.side_steps = lambda side, src, scn, setup: plays[side].pop(0)
                differential.load_scenario = lambda name: dict(sc, name=name)
                with tempfile.TemporaryDirectory() as d:
                    args = types.SimpleNamespace(include_statistical=False, orig=None, new=None,
                                                 no_setup=True, report=d)
                    res = differential.run_one("t/many", args, normalise.load_masks())[0]
                    attempts = sorted(os.listdir(os.path.join(d, "t/many")))
            finally:
                differential.side_steps, differential.load_scenario = saved
            return res, attempts

        res, attempts = play([wander, wander, clean], 2)
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["reruns"], 2)
        self.assertEqual(res["rerun"]["status"], "fail", "the first attempt")
        self.assertIn("attempt1", attempts)
        self.assertIn("attempt2", attempts)
        self.assertTrue(any("rerun 2 times after interference" in n for n in res["notes"]), res["notes"])
        # one rerun is the default: the second wanderer fails the scenario, with the reason
        res, _ = play([wander, wander], 1)
        self.assertEqual(res["status"], "fail")
        self.assertTrue(any("1 rerun spent" in n for n in res["notes"]), res["notes"])

    def test_the_exit_gap_rules_count_in_the_x_step_and_the_originals_side_only(self):
        """cmd_quit leaves autocombat: the original prints `*Combat Off*` and a
        prompt before "You will exit"; silvermere does not, and its queued spell fires among the
        dots. Both are counted under T-093 in the exit step; elsewhere they still fail."""
        sc = self._sc(deterministic=["x"], mask_prompt=True, divergences=T093_RULES)
        P = b"\x1b[79D\x1b[K\x1b[0;37m[HP=30000\x1b[0;37m/MA=1932\x1b[0;37m]:"
        tail = b"Your character has been saved. If you have any comments or suggestions, please\r\nleave them in E-mail to Sysop. Thanks.\r\n\r\n"
        orig = (b"x\r\n\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n" + P
                + b"You will exit after a period of silent meditation.\r\n................\x1b[0m\r\n" + tail)
        new = (b"x\r\nYou will exit after a period of silent meditation.\r\n"
               b"....Your spell has no effect in this room!\r\n............\x1b[0m\r\n" + tail)
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        masks = normalise.load_masks()
        res = differential.compare(sc, mk(orig), mk(new), masks)[0]
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["known_gaps"]["T-093"]["orig"], 2)
        self.assertEqual(res["known_gaps"]["T-093"]["new"], 1)
        self.assertEqual(res["byte"]["equal"], res["byte"]["checked"])
        # the same lines in another command's step are not covered; and an exit that prints
        # the Combat Off on both sides has nothing to count
        other = differential.compare(sc, mk(orig.replace(b"x\r\n", b"y\r\n", 1)),
                                     mk(new.replace(b"x\r\n", b"y\r\n", 1)), masks)[0]
        self.assertEqual(other["status"], "fail", "outside the x step the gap is not counted")
        self.assertEqual((other["known_gaps"]["T-093"]["orig"], other["known_gaps"]["T-093"]["new"]), (0, 0))
        # the rule is for the original's side: once silvermere prints the Combat Off too (T-093
        # landed), the rule has to go, or the new side's line is the difference
        both = differential.compare(sc, mk(orig), mk(orig), masks)[0]
        self.assertEqual(both["status"], "fail")
        # a different line among the dots still fails
        odd = new.replace(b"Your spell has no effect in this room!", b"Something else happens!")
        self.assertEqual(differential.compare(sc, mk(orig), mk(odd), masks)[0]["status"], "fail")

    def test_the_t093_prompt_among_the_dots_is_counted_with_the_spell_line(self):
        """Run 38925318886: the queued spell's line, then the tick's redraw of the prompt (the cast
        cost mana), then the rest of the dots. One prompt directly after the spell line is part of
        the shape; a second one, or a prompt elsewhere in the step, is not."""
        sc = self._sc(deterministic=["x"], mask_prompt=True, divergences=T093_RULES)
        P = b"\x1b[79D\x1b[K\x1b[0;37m[HP=30000\x1b[0;37m/MA=1932\x1b[0;37m]:"
        R = b"\x1b[79D\x1b[K\x1b[0;37m[HP=74\x1b[0;37m/MA=1949\x1b[0;37m]:"
        tail = b"Your character has been saved. If you have any comments or suggestions, please\r\nleave them in E-mail to Sysop. Thanks.\r\n\r\n"
        orig = (b"x\r\n\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n" + P
                + b"You will exit after a period of silent meditation.\r\n................\x1b[0m\r\n" + tail)
        new = (b"x\r\nYou will exit after a period of silent meditation.\r\n"
               b"....Your spell has no effect in this room!\r\n" + R + b"............\x1b[0m\r\n" + tail)
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        masks = normalise.load_masks()
        res = differential.compare(sc, mk(orig), mk(new), masks)[0]
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["known_gaps"]["T-093"]["orig"], 2)
        self.assertEqual(res["known_gaps"]["T-093"]["new"], 2, "the spell line and its prompt")
        self.assertEqual(res["byte"]["equal"], res["byte"]["checked"])
        # two prompts after the spell line: the second is a difference
        twice = new.replace(R + b"....", R + R + b"....", 1)
        self.assertEqual(differential.compare(sc, mk(orig), mk(twice), masks)[0]["status"], "fail")
        # a prompt before the spell line is not part of it
        before = new.replace(b"....Your spell", R + b"....Your spell", 1)
        self.assertEqual(differential.compare(sc, mk(orig), mk(before), masks)[0]["status"], "fail")

    def test_a_failure_opens_its_diff_with_both_sides_text_of_the_failing_step(self):
        masks = normalise.load_masks()
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        sc = self._sc()
        lines = b"".join(b"line %d\r\n" % i for i in range(60))
        _, _, _, text = differential.compare(
            sc, mk(b"x\r\n\x1b[0;36mYou are now resting.\r\n[HP=10]:"),
            mk(b"x\r\n" + lines + b"You are now standing.\r\n[HP=10]:"), masks)
        self.assertIn("=== orig step 1 ('x')\nx\nYou are now resting.\n[HP=10]:", text)
        self.assertIn(f"=== new step 1 ('x'), last {differential.DIAG_LINES} of 63 lines", text)
        self.assertNotIn("\nline 21\n", text)
        self.assertIn("\nline 22\n", text)
        self.assertNotIn("\x1b", text.split("--- orig")[0])
        # a passing scenario carries no step text
        _, _, _, ok = differential.compare(sc, mk(b"x\r\n[HP=10]:"), mk(b"x\r\n[HP=10]:"), masks)
        self.assertNotIn("===", ok)

    def test_a_skip_names_its_card_and_plays_nothing(self):
        args = type("A", (), {"include_statistical": False})()
        saved = differential.load_scenario
        differential.load_scenario = lambda name: dict(
            saved("m4/aoe"), name=name, skip={"card": "T-099", "why": "a later card"})
        try:
            res, text = differential.run_one("m4/aoe", args, normalise.load_masks())
        finally:
            differential.load_scenario = saved
        self.assertEqual(res["status"], "skipped")
        self.assertTrue(res["reason"].startswith("T-099: "), res["reason"])
        self.assertIsNone(text)

    def test_the_configure_show_developer_pair_is_a_counted_gap_on_the_new_side_only(self):
        sc = differential.load_scenario("m4/sysop")
        ev = lambda name, **f: {"step": 25, "name": name, "fields": f, "line": f.pop("line", "")}
        mk = lambda *lines: [ev("echo", cmd="sysop configure show")] + [
            ev("unknown", line=l) for l in lines]
        gen = "Monster Generation Rate:     5 seconds"
        regen = "Minutes before regenerating: 5 minutes"
        other = "Maximum users in game:       256"
        new = mk("Current Configuration Settings:", gen, regen, other)
        kept, dropped = differential.known_gaps.apply(new, "new", sc["divergences"])
        self.assertEqual(dropped, {"T-055": 2})
        self.assertEqual([e["line"] for e in kept if e["name"] == "unknown"],
                         ["Current Configuration Settings:", other], "any other line stays")
        orig = mk("Current Configuration Settings:", other)
        kept, dropped = differential.known_gaps.apply(orig, "orig", sc["divergences"])
        self.assertEqual(dropped, {})
        kept, dropped = differential.known_gaps.apply(mk(gen), "orig", sc["divergences"])
        self.assertEqual(dropped, {}, "the original's own lines are not dropped")
        # the byte view: only the two lines go, counted
        counts = {}
        text = f"{gen}\r\n{regen}\r\n{other}\r\n"
        out = differential.known_gaps.mask_bytes(text, "new", sc["divergences"], counts)
        self.assertEqual((out, counts), (other + "\r\n", {"T-055": 2}))

    def test_the_configure_show_credit_lines_are_a_counted_gap_on_the_original_side_only(self):
        sc = differential.load_scenario("m4/sysop")
        ev = lambda name, line="", **f: {"step": 25, "name": name, "fields": f, "line": line}
        mk = lambda *lines: [ev("echo", cmd="sysop configure show")] + [
            ev("unknown", l) for l in lines]
        credits = ["Normal Credit Consumption:   60", "Exempt Credit Conusmption:   0",
                   "Free time for exempt users:  0"]
        rest = ["Current Configuration Settings:", "Maximum users in game:       256",
                "Current pollrate:            275", "Current initrate:            195",
                "Logfile: wccmmud.log"]
        kept, dropped = differential.known_gaps.apply(
            mk(rest[0], *credits, *rest[1:]), "orig", sc["divergences"])
        self.assertEqual(dropped, {"T-055": 3})
        self.assertEqual([e["line"] for e in kept if e["name"] == "unknown"], rest,
                         "every other line stays")
        kept, dropped = differential.known_gaps.apply(mk(*credits), "new", sc["divergences"])
        self.assertEqual(dropped, {}, "silvermere's side is not dropped")
        counts = {}
        text = "\r\n".join(credits + rest[1:2]) + "\r\n"
        out = differential.known_gaps.mask_bytes(text, "orig", sc["divergences"], counts)
        self.assertEqual((out, counts), (rest[1] + "\r\n", {"T-055": 3}))

    def test_unknown_value_masks_hide_values_and_keep_labels(self):
        sc = differential.load_scenario("m4/sysop")
        ev = lambda l: {"step": 25, "name": "unknown", "fields": {}, "line": l}
        orig = [ev("Current pollrate:            275"), ev("Current initrate:            195"),
                ev("Logfile: wccmmud.log"), ev("Maximum users in game:       256")]
        new = [ev("Current pollrate:            0"), ev("Current initrate:            0"),
               ev("Logfile: tracing"), ev("Maximum users in game:       256")]
        masks = sc["unknown_value_masks"]
        self.assertEqual(differential.diff.unknown_extra(orig, new, masks), [])
        self.assertEqual(len(differential.diff.unknown_extra(orig, new)), 3, "unmasked: 3")
        # a label that is missing, or a value on any other line, still counts
        self.assertEqual(differential.diff.unknown_extra(orig, new[1:] + [ev("Logfile")], masks),
                         ["Logfile"])
        other = new[:3] + [ev("Maximum users:       40")]
        self.assertEqual(differential.diff.unknown_extra(orig, other, masks),
                         ["Maximum users:       40"], "a changed label counts")
        # the whole step 25 of a live run: every orig-only and new-only line is a counted
        # divergence or value-masked, nothing else is left
        full = lambda *ls: [{"step": 25, "name": "echo", "fields": {"cmd": "sysop configure show"},
                             "line": ""}] + [ev(l) for l in ls]
        o = full("Current Configuration Settings:", "Normal Credit Consumption:   60",
                 "Exempt Credit Conusmption:   0", "Free time for exempt users:  0",
                 "Maximum users in game:       256", "Current pollrate:            275",
                 "Current initrate:            195", "Logfile: wccmmud.log")
        n = full("Current Configuration Settings:", "Monster Generation Rate:     5 seconds",
                 "Minutes before regenerating: 5 minutes", "Maximum users in game:       256",
                 "Current pollrate:            0", "Current initrate:            0",
                 "Logfile: tracing")
        o, od = differential.known_gaps.apply(o, "orig", sc["divergences"])
        n, nd = differential.known_gaps.apply(n, "new", sc["divergences"])
        self.assertEqual((od, nd), ({"T-055": 3}, {"T-055": 2}))
        self.assertEqual(differential.diff.diff_events(o, n, masks), [])
        self.assertEqual(differential.diff.unknown_extra(o, n, masks), [])

    def test_a_rerun_plays_the_restore_on_both_sides_first(self):
        import tempfile
        import types
        clean = b"x\r\nYou are now resting.\r\n[HP=10]:"
        wander = b"x\r\nA giant rat moves into the room from the east.\r\nYou are now resting.\r\n[HP=10]:"
        mk = lambda raw: [step(0, "connect", None, 0, b"Hi\r\n"), step(1, "send", "x", 4, raw)]
        sc = self._sc(rerun_restore="t/restore.json")
        log = []
        for restore_problems, want in (([], "pass"), (["orig: timeout waiting for 'x' (rerun_restore)"], "fail")):
            plays = {"orig": [mk(wander), mk(clean)], "new": [mk(clean), mk(clean)]}
            saved = (differential.side_steps, differential.load_scenario, differential.restore_side)
            try:
                def play(side, src, scn, setup, plays=plays):
                    log.append(("play", side))
                    return plays[side].pop(0)

                def restore(side, src, scn, problems=restore_problems):
                    log.append(("restore", side, scn["rerun_restore"]))
                    return problems if side == "orig" else []
                differential.side_steps, differential.restore_side = play, restore
                differential.load_scenario = lambda name: dict(sc, name=name)
                with tempfile.TemporaryDirectory() as d:
                    args = types.SimpleNamespace(include_statistical=False, orig=None, new=None,
                                                 no_setup=True, report=d)
                    res, _ = differential.run_one("t/restore", args, normalise.load_masks())
            finally:
                differential.side_steps, differential.load_scenario, differential.restore_side = saved
            self.assertEqual(log, [("play", "orig"), ("play", "new"),
                                   ("restore", "orig", "t/restore.json"),
                                   ("restore", "new", "t/restore.json"),
                                   ("play", "orig"), ("play", "new")])
            log.clear()
            self.assertEqual(res["status"], want, res)
            self.assertEqual(res["problems"], restore_problems)

    def test_no_restore_without_a_rerun(self):
        saved = differential.restore_side
        calls = []
        try:
            differential.restore_side = lambda *a: calls.append(a) or []
            self._rerun({"orig": [[step(0, "connect", None, 0, b"Hi\r\n"),
                                   step(1, "send", "x", 4, b"x\r\n[HP=10]:"),
                                   step(2, "send", "y", 9, b"y\r\n[HP=10]:")]],
                         "new": [[step(0, "connect", None, 0, b"Hi\r\n"),
                                  step(1, "send", "x", 4, b"x\r\n[HP=10]:"),
                                  step(2, "send", "y", 9, b"y\r\n[HP=10]:")]]})
        finally:
            differential.restore_side = saved
        self.assertEqual(calls, [])

    def test_the_quest_restore_rests_aldric_and_walks_him_back_to_the_start(self):
        sc = differential.load_scenario("m4/quest")
        quest = differential.read_json(sc["script"])["steps"]
        steps = differential.read_json(sc["rerun_restore"])["steps"]
        self.assertEqual(steps[:5], quest[:5], "the quest's own login and entry")
        self.assertEqual(steps[-4:], quest[-4:], "and its exit and logoff")
        self.assertEqual(steps[5]["send"], "rest")
        # the rest waits for its own reply, then the full-HP prompt (the entry's prompt, from
        # before the command, cannot satisfy it); `st` confirms 35/35; the end state is the
        # Narrow Road at full hit points
        self.assertEqual(steps[5]["until"], r"You are now resting\.(?s:.*)\[HP=35\]:")
        self.assertGreaterEqual(steps[5]["timeout"], 600)
        self.assertEqual(steps[6]["send"], "st")
        self.assertIn("Hits:", steps[6]["until"])
        self.assertIn("35/35", steps[6]["until"])
        self.assertEqual([x["send"] for x in steps[7:-4]], ["n", "n", "w", "n", "n", "nw", "w", "w"])
        self.assertEqual(steps[-5]["until"], r"Narrow Road(?s:.*)\[HP=35\]:")
        self.assertTrue(steps[-5]["required"])
        self.assertEqual([x["send"] for x in quest[5:7]], ["e", "e"], "the quest starts there")
        # the quest no longer goes down to the Arena (mongroup 6, generates at 90%)
        self.assertNotIn("d", [x.get("send") for x in quest])
        self.assertNotIn("search", [x.get("send") for x in quest])

    def test_a_noise_after_the_steps_own_prompt_passes_counted(self):
        # the live m4/sysop failure: silvermere's step 15 got a monster's noise after its
        # prompt; one prompt more, and 129 differences when the events were paired by index
        P = b"\x1b[79D\x1b[K\x1b[0;37m[HP=100/MA=50]:"
        noise = b"\x1b[79D\x1b[K\x1b[0;36mYou hear movement to the north.\r\n" + P

        def mk(tail):
            first = step(1, "send", "sysop goto khazarad", 4, b"sysop goto khazarad\r\n" + tail)
            return [step(0, "connect", None, 0, b"Hi\r\n"), first,
                    step(2, "send", "sysop goto lostcity", 4 + len(first.raw),
                         b"sysop goto lostcity\r\n" + P)]
        sc = {"name": "t", "script": "/x", "ignore_host_banner": True, "deterministic": []}
        res = differential.compare(sc, mk(P), mk(P + noise), normalise.load_masks())[0]
        self.assertEqual(res["status"], "pass", res)
        # the noise and its prompt are a random event (random_events.json), counted per side
        self.assertEqual(res["random"]["random: monster noise"], {"orig": 0, "new": 2})

    def test_one_extra_event_is_one_difference(self):
        import diff
        ev = lambda st, name, **f: {"step": st, "name": name, "fields": f}
        a = [ev(i, "prompt", hp="100") if i % 2 else ev(i, "echo", cmd=str(i)) for i in range(60)]
        b = a[:15] + [ev(15, "prompt", hp="100")] + a[15:]
        out = diff.diff_events(a, b)
        self.assertEqual(out, [{"index": 15, "kind": "missing", "extra_in": "new",
                                "event": "prompt", "step": 15}])
        # a changed field stays a field difference at its place, the rest equal
        c = [dict(e, fields=dict(e["fields"])) for e in a]
        c[30]["fields"]["cmd"] = "other"
        self.assertEqual([(d["kind"], d["index"]) for d in diff.diff_events(a, c)], [("field", 30)])
        self.assertEqual(diff.diff_events(a, a), [])

if __name__ == "__main__":
    unittest.main()
