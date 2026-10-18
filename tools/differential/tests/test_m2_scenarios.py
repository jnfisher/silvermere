"""The m2 statistical scenarios (T-040; John, 2026-10-16): sample counts with a time cap, the
sheet scripts' structure, the per-session substitutions."""
import json
import re
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

import differential  # noqa: E402
import stats  # noqa: E402

PROMPT_RX = r"\[HP=-?\d+[^\]\n]*\]:"


class SheetScenarios(unittest.TestCase):
    def setUp(self):
        differential.AS_USER, differential.SUBST = None, {}

    def tearDown(self):
        differential.AS_USER, differential.SUBST = None, {}

    def expanded(self, name):
        sc = differential.load_scenario(name)
        script = stats.expand_script(differential.read_json(sc["script"]), sc["m2"])
        return sc, differential.substitute(script, sc)

    def test_each_sheet_stops_on_2000_swings_or_30_minutes(self):
        for sheet, mon in (("rat", "giant rat"), ("kobold", "kobold thief"),
                           ("slime", "acid slime")):
            sc = differential.load_scenario(f"m2/swings-{sheet}")
            m2 = sc["m2"]
            self.assertEqual((m2["min_swings"], m2["stop"]["at_least"], m2["stop"]["minutes"]),
                             (2000, 2000, 30))
            self.assertEqual(m2["monster"], mon)
            self.assertTrue(sc["world_patterns"], "monster texts are checked against game data")

    def test_the_cycle_checks_the_stop_and_the_logoff_follows_it(self):
        sc, script = self.expanded("m2/swings-rat")
        steps = script["steps"]
        checks = [i for i, s in enumerate(steps) if s.get("stop_check")]
        self.assertEqual(len(checks), sc["m2"]["repeat"]["times"])
        after = [i for i, s in enumerate(steps) if s.get("after_stop")]
        self.assertEqual(len(after), 1)
        self.assertGreater(after[0], checks[-1])
        self.assertEqual([s.get("send") for s in steps[-4:]], ["x", "X", "X", "Y"])
        # the cycles' own variables: one fight's end does not skip the next fight's waits
        self.assertIn("over0", json.dumps(steps[checks[0]:checks[1]]))
        self.assertIn("over1", json.dumps(steps[checks[1]:checks[2]]))

    def test_the_sheet_is_set_and_undone(self):
        _, script = self.expanded("m2/swings-rat")
        sends = [s.get("send") for s in script["steps"]]
        for cmd in ("sysop tweak race 11", "sysop tweak class 2", "sysop tweak strength 79",
                    "sysop tweak agility 42", "sysop tweak health 74", "sysop monster 1",
                    "a rat", "sysop tweak race 1", "sysop tweak level 1"):
            self.assertIn(cmd, sends)
        undo = [s["send"] for s in script["on_abort"]]
        self.assertIn("sysop tweak level 1", undo)
        self.assertIn("sysop tweak race 1", undo)
        steps = script["steps"]
        a = next(i for i, s in enumerate(steps) if s.get("after_stop"))
        x = next(i for i, s in enumerate(steps) if s.get("send") == "x")
        self.assertEqual(undo, [s["send"] for s in steps[a:x] if s["send"] != "sysop buffers"],
                         "the abort undoes as the end does")

    def test_the_watch_restores_its_level(self):
        sc = differential.load_scenario("m2/spawn-arena")
        script = differential.read_json(sc["script"])
        self.assertIn("sysop tweak level 15", [s.get("send") for s in script["steps"]])
        self.assertIn("sysop tweak level 1", [s["send"] for s in script["on_abort"]])
        self.assertFalse(sc["m2"]["gate_wanders"])   # advisory here: m2/wander is the watch
        self.assertEqual(sc["m2"]["stop"], {"minutes": 25})

    def test_the_session_substitutions(self):
        differential.AS_USER = "Bevan:bevan1"
        differential.SUBST = {"@ROOM@": "2500"}
        sc, script = self.expanded("m2/swings-kobold")
        sends = [s.get("send") for s in script["steps"]]
        # the name, the retry after MBBSEmu's "already logged in" (only when refused), the password
        self.assertEqual([sends[1], sends[3], sends[4]], ["Bevan", "Bevan", "bevan1"])
        self.assertEqual(script["steps"][3]["if_set"], "busy")
        self.assertIn("sysop express_train 2500 1", sends)
        self.assertIn("sysop monster 7", sends)
        self.assertNotIn("@", json.dumps(script["steps"]))
        setup = differential.substitute(differential.read_json(os.path.join(
            differential.SCENARIOS, "m2/setup/orig-character-sysop.json")), {})
        self.assertEqual(setup["steps"][8]["send"], "\b" * 5 + "Bevan")

    def test_the_sheet_stops_on_both_samples(self):
        import re
        import replay
        m2 = differential.load_scenario("m2/swings-slime")["m2"]
        mon = m2["stop"]["also"][0]
        self.assertEqual(mon["at_least"], 2000)
        lines = ["The acid slime whips you with its pseudopod for 8 damage!",
                 "The large acid slime flails at you!",
                 "The acid slime lashes at you, but you dodge out of the way!",
                 "The acid slime whips Bevan with its pseudopod for 3 damage!",
                 "Acid burns you for 1 damage!", "The acid slime dissolves into a puddle."]
        self.assertEqual([bool(re.search(mon["count"], ln)) for ln in lines],
                         [True, True, True, False, False, False])

        class R:
            def __init__(self, text):
                self.text = text

            def stream(self):
                return self.text
        stop = {"count": "P", "at_least": 2, "also": [{"count": "M", "at_least": 3}],
                "minutes": 30}
        self.assertIsNone(replay.stop_reached(stop, R("P\nP\nM\n"), str, now=0))
        got = replay.stop_reached(stop, R("P\nP\nM\nM\nM\n"), str, now=10)
        self.assertEqual((got["reason"], got["count"], got["also"]), ("count", 2, [3]))

    def test_one_monster_a_cycle_and_the_room_cleared_first(self):
        sc, script = self.expanded("m2/swings-rat")
        steps = script["steps"]
        first = next(i for i, s in enumerate(steps) if s.get("stop_check"))
        nxt = next(i for i, s in enumerate(steps) if i > first and s.get("stop_check"))
        cycle = [s.get("send") for s in steps[first:nxt]]
        self.assertEqual(cycle.count("sysop monster 1"), 1)   # the target only (item 6)
        before = [s.get("send") for s in steps[:first]]
        self.assertNotIn("sysop monster 1", before)
        self.assertEqual(sum(1 for x in before if x and x.startswith("a {clr")), 6)
        self.assertEqual(before.count("sysop buffers"), 1)
        after = [s.get("send") for s in steps if s.get("send") == "sysop buffers"]
        self.assertEqual(len(after), 2)   # the start and the end

    def test_the_stop_count_matches_the_players_swings_only(self):
        import re
        rx = differential.load_scenario("m2/swings-rat")["m2"]["stop"]["count"]
        lines = ["[HP=30000]:You whap large giant rat for 1 damage!",
                 "You swipe at giant rat!", "You swing at angry giant rat who dodges your attack!",
                 "Your swing at giant rat hits, but glances off its armour.",
                 "The giant rat lunges at you!", "Bevan beats giant rat for 7 damage!",
                 "You gain 9 experience.", "A giant rat creeps into the room from the east."]
        self.assertEqual([bool(re.search(rx, ln)) for ln in lines],
                         [True, True, True, True, False, False, False, False])


class Masks(unittest.TestCase):
    """The m2 masks hide values only, never labels (the #72 review; text fidelity first)."""

    SAMPLES = ["\x1b[32mHits: \x1b[36m   35/35    \x1b[32mArmour",
               "\x1b[32mExp: \x1b[36m12              \x1b[32mPerception",
               "Exp needed for next level: \x1b[36m988 (1000) [1%]", "[HP=35/MA=4]:",
               "You whap X for 12 damage!", "You gain 12 experience.",
               "You notice 2 silver nobles, 15 copper farthings here."]
    LABELS = ["Hits: ", "Exp: ", "next level: ", "(1000)", "%]", "[HP=", "/MA=", "]:", "for ",
              " damage!", "You gain ", " experience.", " silver nobles", " copper farthings"]

    def test_no_mask_takes_a_label(self):
        import diff
        import glob
        for path in glob.glob(os.path.join(differential.SCENARIOS, "m2", "scripts", "*.json")):
            regs = [m["regex"] for m in differential.read_json(path).get("mask", [])]
            for raw in self.SAMPLES:
                out = diff.script_mask(raw, regs)
                for label in self.LABELS:
                    if label in raw:
                        self.assertIn(label, out, (path, raw, out))

    def test_the_fight_scripts_mask_every_random_number(self):
        import diff
        regs = [m["regex"] for m in differential.read_json(os.path.join(
            differential.SCENARIOS, "m2", "scripts", "fight-arena.json"))["mask"]]
        for raw in self.SAMPLES:
            out = diff.script_mask(raw, regs)
            left = re.sub(r"\x1b\[[0-9;]*m|\(1000\)|/35\b", "", out)
            self.assertNotRegex(left, r"\d", (raw, out))


class Collapse(unittest.TestCase):
    def test_only_the_knock_out_is_collapsed(self):
        ev = lambda name: {"step": 0, "name": name, "fields": {"who": "Corvin"}}  # noqa: E731
        out = differential.collapse_repeats([ev("drops_unconscious"), ev("drops_unconscious"),
                                             ev("you_killed"), ev("you_killed")])
        self.assertEqual([e["name"] for e in out],
                         ["drops_unconscious", "you_killed", "you_killed"])


class WanderWatch(unittest.TestCase):
    def test_counts_the_monster_in_the_watch_steps_only(self):
        ev = lambda st, name, **f: {"step": st, "name": name, "fields": f}  # noqa: E731
        evs = [ev(3, "monster_arrives", mon="lashworm", frm="the west"),     # a summon, room A
               ev(5, "monster_arrives", mon="nasty lashworm", base="lashworm", frm="the east"),
               ev(5, "monster_arrives", mon="giant rat", frm="the north"),   # not the watch's
               ev(7, "monster_arrives", mon="lashworm", frm="the west")]     # the fight step
        sm = stats.samples(evs, (), "lashworm", {5, 6})
        self.assertEqual((sm["wanders"], sm["arrivals"]), (1, 0))

    def test_the_watch_is_gated_on_walk_ins_only(self):
        m2 = differential.load_scenario("m2/wander")["m2"]
        self.assertTrue(m2["watch"] and m2["gate_wanders"])
        script = differential.read_json(differential.load_scenario("m2/wander")["script"])
        self.assertEqual(sum(1 for s in script["steps"] if s.get("watch")), 2)


class CacheSize(unittest.TestCase):
    """m2_live.sh's match_cache: the original's active-monster cache size from SYSOP BUFFERS."""

    def test_the_buffers_script_logs_in_reads_and_logs_off(self):
        differential.AS_USER = "Arlo:arlo1"
        try:
            script = differential.substitute(differential.read_json(os.path.join(
                differential.SCENARIOS, "m2/setup/buffers.json")), {})
        finally:
            differential.AS_USER = None
        sends = [st.get("send") for st in script["steps"]]
        self.assertEqual(sends[1], "Arlo")
        self.assertIn("sysop buffers", sends)
        self.assertEqual(sends[-4:], ["x", "X", "X", "Y"])
        until = next(st["until"] for st in script["steps"] if st.get("send") == "sysop buffers")
        text = ("Buffer Statistics: (1234 ticks)\n"
                "Active Monsters: Size: 21    Used: 21    Hits: 9     Miss: 40    Mod: 3    \n"
                "[HP=35]:")
        self.assertRegex(text, until)
        # the line m2_live.sh greps (`Active Monsters: +Size: +[0-9]+`)
        self.assertEqual(re.search(r"Active Monsters: +Size: +([0-9]+)", text).group(1), "21")


class UnknownLines(unittest.TestCase):
    """The sample-count scenarios (`m2.stop`) compare unclassified lines as digit-masked sets
    (#72, item C); fixed scripts keep the multiset."""

    def ev(self, line):
        return {"step": 0, "name": "unknown", "fields": {}, "line": line}

    def test_more_cycles_and_other_numbers_are_not_new_lines(self):
        import diff
        orig = [self.ev("a rat"), self.ev("Buffer Statistics: (33883058 - 517 Sec)"),
                self.ev("Rooms:           Size: 51    Used: 64492 Hits: 31329 Miss: 49976 Mod: 8")]
        new = [self.ev("a rat")] * 30 + [
            self.ev("Buffer Statistics: (43253760 - 660 Sec)"),
            self.ev("Rooms:           Size: 2001  Used: 2001  Hits: 0     Miss: 0     Mod: 0"),
            self.ev("Command not recognized."), self.ev("Command not recognized.")]
        self.assertEqual(diff.unknown_extra(orig, new, as_set=True), ["Command not recognized."])
        # a fixed script keeps the multiset: 29 more `a rat` than the original
        self.assertEqual(diff.unknown_extra(orig, new).count("a rat"), 29)

    def test_only_the_sample_count_runs_compare_as_sets(self):
        names = ("m2/swings-rat", "m2/swings-kobold", "m2/swings-slime", "m2/spawn-arena",
                 "m2/wander", "m2/rest", "m2/fight-arena", "m2/death-rebirth")
        got = {n for n in names if differential.unknown_as_set(differential.load_scenario(n))}
        self.assertEqual(got, {"m2/swings-rat", "m2/swings-kobold", "m2/swings-slime",
                               "m2/spawn-arena", "m2/wander"})
        self.assertTrue(differential.unknown_as_set({"unknown_as_set": True}))
        self.assertFalse(differential.unknown_as_set({"statistical": True}))

    def test_the_initialize_script_only_shows_the_cycle(self):
        script = differential.read_json(os.path.join(differential.SCENARIOS,
                                                     "m2/setup/initialize.json"))
        sends = [st.get("send") for st in script["steps"]]
        self.assertIn("sysop initialize -1", sends)   # a negative rate never restarts it
        self.assertFalse([x for x in sends if x and x.startswith("sysop initialize")
                          and x != "sysop initialize -1"])
        until = next(st["until"] for st in script["steps"]
                     if st.get("send") == "sysop initialize -1")
        text = ("Last Cycle:  Map 17, Room 1204, Monster 0, Known 0/1\n"
                "Active Monsters: 7311, Known Monsters: 1093\nState: 2 (Rooms)  (Rate 195)\n")
        self.assertRegex(text, until)
        # the State m2_live.sh's init_state greps
        self.assertEqual(re.search(r"State: -?[0-9]+ [^(]*\([A-Za-z ]+\)", text).group(0),
                         "State: 2 (Rooms)")


class OrigEvidence(unittest.TestCase):
    """The original's WCCACMSR summary and the log flush (#72, maxactive 27 under MBBSEmu)."""

    def test_acmsr_summary(self):
        got = stats.acmsr_summary([5, 1, 2, 30, 7311])
        self.assertEqual(got, {"count": 5, "min": 1, "max": 7311, "gaps_below_max": 7306,
                               "above_27": 2})
        self.assertEqual(stats.acmsr_summary([]), {"count": 0})

    def test_the_reload_script_only_reloads(self):
        script = differential.read_json(os.path.join(differential.SCENARIOS,
                                                     "m2/setup/reload.json"))
        sends = [st.get("send") for st in script["steps"]]
        self.assertEqual([x for x in sends if x and x.startswith("sysop")], ["sysop reload"])


class EnoughAttempts(unittest.TestCase):
    """death-rebirth and rest wait for a monster that hurts (in the arena, mostly the acid slime,
    about one spawn in eight): enough rounds that a run rarely ends without one (#72)."""

    def test_rounds(self):
        for name, send, want in (("m2/death-rebirth", "a {mon}", 25), ("m2/rest", None, 20)):
            sc = differential.load_scenario(name)
            self.assertEqual(sc["m2"]["repeat"]["times"], want)
            if send:
                script = stats.expand_script(differential.read_json(sc["script"]), sc["m2"])
                rounds = [st for st in script["steps"] if st.get("send") == send]
                self.assertEqual(len(rounds), want)
                self.assertTrue(all("skip_if" in st for st in rounds))


class RestOutcome(unittest.TestCase):
    """m2/rest's `rest` waits for its outcome line: still engaged, `rest` breaks off the
    auto-combat first (`*Combat Off*` and a prompt), and a wait for the
    first prompt left `rested` unset, so the original rested twice (live run 39030247260)."""

    def test_the_first_prompt_is_not_the_outcome(self):
        import re
        script = differential.read_json(differential.load_scenario("m2/rest")["script"])
        rests = [st for st in script["steps"] if st.get("send") == "rest"]
        self.assertTrue(rests)
        orig = ("[HP=19]:rest\n*Combat Off*\n[HP=19]:You are now resting.\n"
                "[HP=19 (Resting) ]:")
        for st in rests:
            m = re.search(st["until"], orig)
            self.assertIsNotNone(m)
            self.assertIn("You are now resting", m.group(0))
            self.assertIsNone(re.search(st["until"], "[HP=19]:rest\n*Combat Off*\n[HP=19]:"))
            self.assertIsNotNone(re.search(st["until"], "You are too sick to rest!\n[HP=9]:"))
            # at 0 hit points or below the dispatcher refuses it
            m = re.search(st["until"], "[HP=0]:rest\nYou may not do that while you are mortally "
                                       "wounded!\n[HP=0]:")
            self.assertIsNotNone(m)
            self.assertIsNone(re.search(st["set"]["rested"], m.group(0)))


class SlowFight(unittest.TestCase):
    """m2/death-rebirth: a fight still on after 300 s (fists against a regenerating filthbug,
    live run 39030247260) is waited out in the cycle, not a timeout."""

    def test_each_cycle_waits_a_slow_fight_out(self):
        sc = differential.load_scenario("m2/death-rebirth")
        script = stats.expand_script(differential.read_json(sc["script"]), sc["m2"])
        waits = [st for st in script["steps"] if st.get("op") == "sleep"
                 and st.get("until") == r"\*Combat Off\*|\[HP=-\d+" and "skip_if" in st]
        self.assertEqual(len(waits), 2 * sc["m2"]["repeat"]["times"])
        for k in range(sc["m2"]["repeat"]["times"]):
            first, slow = waits[2 * k], waits[2 * k + 1]
            self.assertTrue(first["may_time_out"] and slow["may_time_out"])
            self.assertIn(f"over{k}", first["set"])
            self.assertEqual(first["if_unset"], [f"over{k}"])
            # the slow wait at most once per run (CLEANUP_AT)
            self.assertEqual(slow["if_unset"], [f"over{k}", "slow"])
            self.assertIn("slow", slow["set"])
        # a monster already gone ends the cycle without either wait
        attacks = [st for st in script["steps"] if st.get("send") == "a {mon}"]
        self.assertTrue(all("You don't see" in st["set"][f"over{k}"]
                            for k, st in enumerate(attacks)))


class NightlyCleanup(unittest.TestCase):
    """MBBSEmu's nightly cleanup (m4 run 38997202711): named, and an m2 run logs off at it."""

    LINE = ('Sorry to interrupt here, but the server will be shutting down in 5 minutes for the '
            'nightly "auto-cleanup" process. Please finish up and log off... thank you!')

    def test_a_stop_run_logs_off_at_the_warning(self):
        import replay

        class R:
            def __init__(self, text):
                self.text = text

            def stream(self):
                return self.text
        stop = {"count": "^P$", "at_least": 2000, "minutes": 30}
        self.assertIsNone(replay.stop_reached(stop, R("P\n"), str, now=0))
        got = replay.stop_reached(stop, R("P\n" + self.LINE + "\nP\n"), str, now=60)
        self.assertEqual((got["reason"], got["count"]), ("cleanup", 2))

    def test_the_warning_is_the_stated_cause(self):
        oe = [{"step": 4, "name": "you_hit", "fields": {}},
              {"step": 7, "name": "unknown", "fields": {}, "line": self.LINE},
              {"step": 8, "name": "unknown", "fields": {}, "line": self.LINE}]
        hits = differential.interference({}, [], oe, [])
        self.assertEqual(hits, [(7, "orig step 7: cleanup (MBBSEmu nightly auto-cleanup)")])


if __name__ == "__main__":
    unittest.main()
