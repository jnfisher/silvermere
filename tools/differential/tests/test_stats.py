"""stats.py (T-040): the statistics agree with crates/silvermere-testkit/src/stats.rs, the
samples come out of events as documented, and the record comparison reads both sides."""
import collections
import glob
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import stats  # noqa: E402

ROOT = os.path.join(HERE, "..", "..", "..")


def ev(name, step=0, **fields):
    return {"name": name, "step": step, "fields": {k: str(v) for k, v in fields.items()}}


class StatisticsTests(unittest.TestCase):
    def test_selftest_numbers(self):
        stats.selftest()

    def test_agrees_with_the_rust_export(self):
        # written by `cargo test -p silvermere-testkit --test stock_ab_sheets`; skipped before a build
        found = glob.glob(os.path.join(ROOT, "target", "**", "ab_sheets.json"), recursive=True)
        if not found:
            self.skipTest("no ab_sheets.json under target/ (run the Rust test first)")
        with open(found[0], encoding="utf-8") as f:
            doc = json.load(f)
        for case in doc["cases"]:
            keys = list(case["expected"])
            t = stats.chi_square_gof([case["observed"][k] for k in keys],
                                     [case["expected"][k] for k in keys])
            rust = case["tests"]["joint"]
            self.assertAlmostEqual(t["statistic"], rust["statistic"], places=9, msg=case["case"])
            self.assertAlmostEqual(t["p_value"], rust["p_value"], places=9, msg=case["case"])

    def test_poisson_rates(self):
        self.assertGreater(stats.poisson_rates(54, 60.0, 50, 60.0)["p_value"], 0.01)
        self.assertLess(stats.poisson_rates(90, 60.0, 30, 60.0)["p_value"], 0.01)
        self.assertEqual(stats.poisson_rates(0, 60.0, 0, 60.0)["p_value"], 1.0)


class SampleTests(unittest.TestCase):
    def test_swings_damage_blows_regen_arrivals(self):
        events = [
            ev("combat_engaged", 1),
            ev("you_hit", 2, dmg=7, target="giant rat", verb="smack"),
            ev("prompt", 2, hp=40),
            ev("you_miss", 3, target="giant rat", verb="swipe at"),
            ev("you_hit", 3, dmg=9, target="giant rat", verb="beat"),
            ev("prompt", 3, hp=40),
            ev("combat_off", 4),
            ev("prompt", 5, hp=40),
            ev("prompt", 6, hp=42),   # a slow tick
            ev("prompt", 7, hp=48),   # a rest tick
            {"name": "unknown", "step": 8, "fields": {},
             "line": "A giant rat creeps into the room from nowhere."},
            ev("monster_arrives_generic", 8, mon="lashworm", dir="the east"),
            ev("prompt", 8, hp=48),
            ev("combat_engaged", 9),
            ev("prompt", 9, hp=60),   # not after a prompt: no gain, but the run's top
        ]
        s = stats.samples(events, round_steps=[2, 3])
        self.assertEqual(s["swings"], collections.Counter(hit=2, miss=1))
        self.assertEqual(s["damage"], collections.Counter({7: 1, 9: 1}))
        self.assertEqual(s["blows"], collections.Counter({1: 1, 2: 1}))
        self.assertEqual(s["regen"], collections.Counter({2: 1, 6: 1}))
        self.assertEqual((s["arrivals"], s["wanders"]), (1, 1))  # a spawn, a walk-in

    def test_a_gain_clipped_at_the_maximum_is_left_out(self):
        """Run 39035493410's rest: 33 -> 35 is a +3 clipped at maxhp 35,
        maxhp from the `st` line after the rest; only a gain ending on it is left out."""
        st = ev("st_hits", 2, hp="35", maxhp="35", ac="11", dr="0")
        events = [ev("prompt", 1, hp=h) for h in (29, 32, 33, 35, 35)] + [st]
        s = stats.samples(events)
        self.assertEqual(s["regen"], collections.Counter({3: 1, 1: 1}))
        self.assertEqual(s["regen_cap_from_top"], 0)

    def test_a_run_that_never_reaches_full_keeps_its_last_gain(self):
        """maxhp 40 on the `st` line: 35 is not the cap, so the last +2 is a real gain."""
        st = ev("st_hits", 2, hp="35", maxhp="40", ac="11", dr="0")
        events = [ev("prompt", 1, hp=h) for h in (29, 32, 33, 35)] + [st]
        self.assertEqual(stats.samples(events)["regen"], collections.Counter({3: 1, 1: 1, 2: 1}))

    def test_without_an_st_line_the_top_stands_in_with_a_note(self):
        events = [ev("prompt", 1, hp=h) for h in (29, 32, 33, 35)]
        s = stats.samples(events)
        self.assertEqual(s["regen"], collections.Counter({3: 1, 1: 1}))
        self.assertEqual(s["regen_cap_from_top"], 3)
        row = {c["what"]: c for c in stats.compare_samples(s, s, min_swings=0)}["regen per tick"]
        self.assertIn("no `st` Hits line", row["note"])

    def test_wanderers_and_noise_are_advisory(self):
        o = stats.load_samples({"arrivals": 10, "wanders": 2, "noise": 3})
        n = stats.load_samples({"arrivals": 11, "wanders": 40, "noise": 60})
        checks = {c["what"]: c for c in stats.compare_samples(o, n, (600.0, 600.0))}
        self.assertTrue(checks["spawns per minute"]["pass"])
        for what in ("wanderers per minute", "monster noise per minute"):
            self.assertTrue(checks[what]["pass"])
            self.assertTrue(checks[what]["advisory"])
            self.assertTrue(checks[what]["flag"])
        n = stats.load_samples({"arrivals": 60, "wanders": 2, "noise": 3})
        checks = {c["what"]: c for c in stats.compare_samples(o, n, (600.0, 600.0))}
        self.assertFalse(checks["spawns per minute"]["pass"])

    def side(self, hits, misses, dmg):
        return {"swings": collections.Counter(hit=hits, miss=misses),
                "damage": collections.Counter(dmg), "blows": collections.Counter({1: 300, 2: 700}),
                "regen": collections.Counter({2: 3, 6: 5}), "arrivals": 50}

    def test_equal_sides_pass(self):
        o = self.side(1980, 1020, {d: 180 for d in range(2, 13)})
        n = self.side(1995, 1005, {d: 181 for d in range(2, 13)})
        checks = stats.compare_samples(o, n, (600.0, 600.0))
        self.assertTrue(all(c["pass"] for c in checks), checks)
        self.assertEqual([c["what"] for c in checks],
                         ["hit rate", "outcome mix", "damage histogram", "blows per round",
                          "regen per tick", "spawns per minute"])

    def test_a_shifted_hit_rate_fails_and_so_does_a_small_sample(self):
        o = self.side(1980, 1020, {5: 100})
        n = self.side(2160, 840, {5: 100})
        hit = stats.compare_samples(o, n)[0]
        self.assertFalse(hit["pass"])
        small = stats.compare_samples(self.side(60, 40, {5: 1}), self.side(60, 40, {5: 1}))[0]
        self.assertFalse(small["enough"])
        self.assertFalse(small["pass"])

    def test_a_short_scenario_reports_not_enough_instead_of_failing(self):
        # the live fight-arena run: 11 of 12 against 10 of 10, regen [] against [1]
        o = {"swings": collections.Counter(hit=11, miss=1), "damage": collections.Counter({5: 11}),
             "blows": collections.Counter(), "regen": collections.Counter(), "arrivals": 0}
        n = dict(o, swings=collections.Counter(hit=10), damage=collections.Counter({5: 10}),
                 regen=collections.Counter({1: 1}))
        checks = {c["what"]: c for c in stats.compare_samples(o, n, min_swings=0)}
        self.assertTrue(checks["hit rate"]["pass"])
        self.assertFalse(checks["hit rate"]["enough"])
        self.assertTrue(checks["regen per tick"]["pass"])
        self.assertFalse(checks["regen per tick"]["enough"])
        # a gross divergence still fails, even short
        far = dict(o, swings=collections.Counter(hit=10, miss=290))
        self.assertFalse(stats.compare_samples(o, far, min_swings=0)[0]["pass"])
        # the rest scenario requires its gains
        req = {c["what"]: c for c in stats.compare_samples(o, n, min_swings=0, min_regen=5)}
        self.assertFalse(req["regen per tick"]["pass"])

    def test_regen_sets_must_be_identical(self):
        o = self.side(1980, 1020, {5: 100})
        n = dict(o, regen=collections.Counter({2: 3, 7: 5}))
        regen = next(c for c in stats.compare_samples(o, n) if c["what"] == "regen per tick")
        self.assertFalse(regen["pass"])


class RandomEventTests(unittest.TestCase):
    def test_noise_is_a_random_event_not_a_known_gap(self):
        import known_gaps
        # no known-gap rule drops the noise any more (the other cards' rules stay)
        self.assertFalse([r for r in known_gaps.load()
                          if r.get("card") == "T-040" or "noise" in r.get("events", [])])
        rules = known_gaps.load_random()
        events = [ev("echo", 2, cmd="n"), ev("noise", 2, dir="north"), ev("prompt", 2, hp=35),
                  ev("room_exits", 2, exits="up")]
        kept, dropped = known_gaps.apply(events, "new", rules)
        self.assertEqual([e["name"] for e in kept], ["echo", "room_exits"])
        self.assertEqual(dropped, {"random: monster noise": 2})

    def test_noise_rate_is_compared_as_advisory(self):
        base = {"swings": collections.Counter(), "damage": collections.Counter(),
                "blows": collections.Counter(), "regen": collections.Counter(), "arrivals": 0}
        o, n = dict(base, noise=30), dict(base, noise=28)
        checks = stats.compare_samples(o, n, (600.0, 600.0), min_swings=0)
        self.assertEqual([c["what"] for c in checks], ["monster noise per minute"])
        self.assertTrue(checks[0]["pass"])
        self.assertFalse(checks[0]["flag"])
        far = stats.compare_samples(o, dict(base, noise=2), (600.0, 600.0), min_swings=0)[0]
        self.assertEqual((far["pass"], far["flag"]), (True, True))  # advisory: flagged only


class ShardTests(unittest.TestCase):
    def test_samples_round_trip_and_pool(self):
        a = {"swings": collections.Counter(hit=3, miss=1), "damage": collections.Counter({5: 3}),
             "blows": collections.Counter({1: 2}), "regen": collections.Counter({2: 1}),
             "arrivals": 1, "wanders": 0, "noise": 2,
             "mon_swings": collections.Counter(hit=2, miss=5), "mon_damage": collections.Counter({4: 2}),
             "mon_rounds": collections.Counter({1: 3, 2: 1})}
        back = stats.load_samples(json.loads(json.dumps(stats.dump_samples(a))))
        self.assertEqual(back, a)
        pooled = stats.pool_samples([a, back])
        self.assertEqual(pooled["swings"], collections.Counter(hit=6, miss=2))
        self.assertEqual(pooled["damage"], collections.Counter({5: 6}))
        self.assertEqual((pooled["arrivals"], pooled["noise"]), (2, 4))
        self.assertEqual(pooled["mon_swings"], collections.Counter(hit=4, miss=10))


class MonsterSwingTests(unittest.TestCase):
    """The monster sheet's swings at the player (game-data texts) and the stop rule."""

    def test_monster_swings_counted_from_the_you_events_only(self):
        evs = [ev("mon_hits_you", 1, mon="giant rat", dmg="4"), ev("mon_miss_you", 1),
               ev("mon_dodge_you", 1), ev("mon_deflect_you", 1),
               ev("mon_hits_other", 1, dmg="7"), ev("monster_arrives", 1, mon="rat", frm="nowhere"),
               ev("monster_arrives", 1, mon="rat", frm="the north")]
        sm = stats.samples(evs)
        self.assertEqual(sm["mon_swings"],
                         collections.Counter(hit=1, miss=1, dodge=1, deflect=1))
        self.assertEqual(sm["mon_damage"], collections.Counter({4: 1}))
        self.assertEqual((sm["arrivals"], sm["wanders"]), (1, 1))

    def base(self, mon):
        return {"swings": collections.Counter(), "damage": collections.Counter(),
                "blows": collections.Counter(), "regen": collections.Counter(), "arrivals": 0,
                "wanders": 0, "noise": 0, "mon_swings": collections.Counter(mon),
                "mon_damage": collections.Counter()}

    def test_monster_hit_rate_needs_its_sample(self):
        o = self.base({"hit": 1000, "miss": 1100})
        n = self.base({"hit": 1010, "miss": 1090})
        c = next(c for c in stats.compare_samples(o, n) if c["what"] == "monster hit rate")
        self.assertTrue(c["pass"] and c["enough"])
        short = self.base({"hit": 100, "miss": 110})
        c = next(c for c in stats.compare_samples(short, short) if c["what"] == "monster hit rate")
        self.assertFalse(c["pass"])   # 210 swings: short of 2,000, the comparison fails
        self.assertIn("sample short (orig 210, new 210; 2000 needed a side)", c["note"])

    def test_option_c_bounds_the_players_rows_only(self):
        # John, 2026-10-17: the player's hit rate keeps the 2-point bound; the monster's fails
        # only on a significant difference. 3 points apart, p about 0.06 at 2,100 swings a side
        o = self.base({"hit": 1000, "miss": 1100})
        n = self.base({"hit": 1063, "miss": 1037})
        o["swings"] = collections.Counter(hit=1000, miss=1100)
        n["swings"] = collections.Counter(hit=1063, miss=1037)
        rows = {c["what"]: c for c in stats.compare_samples(o, n)}
        mon, you = rows["monster hit rate"], rows["hit rate"]
        self.assertGreater(mon["p_value"], 0.01)
        self.assertTrue(mon["pass"] and mon["held"])
        self.assertNotIn("bound", mon)
        self.assertFalse(you["pass"] or you["held"])

    def test_holm_steps_down_over_the_family(self):
        rows = [{"what": w, "pass": True, "tested": True, "held": True, "p_value": p}
                for w, p in (("a", 0.001), ("b", 0.003), ("c", 0.009), ("d", 0.5))]
        rows.append({"what": "adv", "pass": True, "tested": True, "advisory": True,
                     "p_value": 0.0})
        rows.append({"what": "regen", "pass": True, "p_value": 1.0})
        self.assertEqual(stats.holm(rows), 4)
        by = {c["what"]: c for c in rows}
        # thresholds 0.01/4, 0.01/3, 0.01/2, 0.01: a and b rejected, c is above 0.005 so the
        # step-down stops and d is kept too
        self.assertEqual([by[w]["pass"] for w in "abcd"], [False, False, True, True])
        self.assertAlmostEqual(by["a"]["holm_alpha"], 0.0025)
        self.assertAlmostEqual(by["c"]["holm_p"], 0.018)
        self.assertAlmostEqual(by["b"]["holm_p"], 0.009)
        self.assertNotIn("holm_alpha", by["adv"])
        self.assertNotIn("holm_alpha", by["regen"])
        # ties: equal raw p get the same adjusted p and the same verdict, rejected together
        # (0.004 <= 0.01/2) or kept together (0.006 > 0.01/2, the step-down stops at once)
        for p, rejected in ((0.004, True), (0.006, False)):
            tie = [{"what": w, "pass": True, "tested": True, "held": True, "p_value": p}
                   for w in "xy"]
            stats.holm(tie)
            self.assertEqual([c["rejected"] for c in tie], [rejected, rejected])
            self.assertEqual([c["pass"] for c in tie], [not rejected, not rejected])
            self.assertAlmostEqual(tie[0]["holm_p"], tie[1]["holm_p"])
            self.assertAlmostEqual(tie[1]["holm_p"], 2 * p)
        # a row whose bound failed fails whatever its p
        bad = [{"what": "x", "pass": True, "tested": True, "held": False, "p_value": 0.9}]
        stats.holm(bad)
        self.assertFalse(bad[0]["pass"])

    def test_a_short_check_run_with_no_real_difference_passes(self):
        # the orchestrator's short rat run (38992586034): ~130 swings a session, failed on the
        # sample; with the cap lowered the sample is not required and Holm decides
        o, n = self.base({"hit": 70, "miss": 60}), self.base({"hit": 64, "miss": 66})
        o["swings"] = collections.Counter(hit=120, miss=10)
        n["swings"] = collections.Counter(hit=124, miss=6)
        m2 = {"min_swings": 2000, "monster": "giant rat", "rates": False, "regen": False}
        rows = lambda: {c["what"]: c for c in stats.checks_of(m2, o, n, (600.0, 600.0))}  # noqa: E731
        self.assertFalse(rows()["hit rate"]["pass"])   # the real run: short, fails
        with mock.patch.dict(os.environ, {"SILVERMERE_M2_MINUTES": "10"}):
            got = rows()
        self.assertTrue(all(c["pass"] for c in got.values()), got)
        self.assertFalse(got["hit rate"]["enough"])

    def test_walk_ins_gate_only_when_the_scenario_says(self):
        o, n = self.base({}), self.base({})
        o["wanders"], n["wanders"] = 40, 2
        adv = next(c for c in stats.compare_samples(o, n, (1500.0, 1500.0))
                   if c["what"] == "wanderers per minute")
        self.assertTrue(adv["pass"] and adv["advisory"])
        gated = next(c for c in stats.compare_samples(o, n, (1500.0, 1500.0), gate_wanders=True)
                     if c["what"] == "wanderers per minute")
        self.assertFalse(gated["pass"])

    def test_a_side_without_a_watch_is_refused_not_passed(self):
        # the live failure: the original never got in (0.01 s), its spawn row passed on nothing
        o, n = self.base({}), self.base({})
        n["arrivals"], n["noise"], n["wanders"] = 35, 12, 4
        checks = {c["what"]: c for c in stats.compare_samples(o, n, (0.01, 236.0))}
        self.assertFalse(checks["spawns per minute"]["pass"])
        self.assertEqual(checks["spawns per minute"]["no_sample"], ["orig"])
        self.assertTrue(checks["monster noise per minute"]["flag"])   # advisory: flagged
        gated = {c["what"]: c for c in stats.compare_samples(o, self.base({}), (0.01, 236.0),
                                                              gate_wanders=True)}
        # the arena watch needs both rows even when nothing came
        self.assertFalse(gated["spawns per minute"]["pass"])
        self.assertFalse(gated["wanderers per minute"]["pass"])
        both = {c["what"]: c for c in stats.compare_samples(self.base({}), self.base({}),
                                                             (1500.0, 1500.0), gate_wanders=True)}
        # 0 against 0 over a full watch compares nothing: the gated rows refuse
        for row in ("spawns per minute", "wanderers per minute"):
            self.assertFalse(both[row]["pass"])
            self.assertEqual(both[row]["note"], "no events on either side, not compared")

    def test_monster_swings_per_round_are_advisory(self):
        # John, 2026-10-17: an MBBSEmu artifact (21-slot cache, maxactive 27), reported and
        # flagged, never failing, outside the Holm family
        o, n = self.base({}), self.base({})
        o["mon_rounds"] = collections.Counter({0: 120, 1: 180})
        n["mon_rounds"] = collections.Counter({1: 300})
        rows = stats.checks_of({"min_swings": 0, "monster": "giant rat", "rates": False},
                               o, n, (0.0, 0.0))
        row = next(c for c in rows if c["what"] == "monster swings per round")
        self.assertTrue(row["pass"] and row["advisory"] and row["flag"])
        self.assertIn("MBBSEmu artifact", row["note"])
        self.assertNotIn("holm_alpha", row)

    def test_a_sheet_run_without_swings_fails(self):
        # a local run that never connected came out PASS with no rows at all
        rows = {c["what"]: c for c in stats.checks_of(
            {"min_swings": 2000, "monster": "acid slime", "rates": False},
            self.base({}), self.base({}), (0.0, 0.0))}
        self.assertFalse(rows["hit rate"]["pass"])
        self.assertFalse(rows["monster hit rate"]["pass"])

    def test_which_rows_each_kind_of_scenario_gates(self):
        o, n = self.base({}), self.base({})
        o["swings"], n["swings"] = collections.Counter(hit=90, miss=10), collections.Counter(hit=50, miss=50)
        o["arrivals"], n["arrivals"], o["wanders"], n["wanders"] = 5, 40, 3, 30
        rows = lambda m2: {c["what"]: c for c in stats.checks_of(m2, o, n, (600.0, 600.0))}  # noqa: E731
        short = rows({"min_swings": 0})   # a short scenario: mixed monsters, a watch that ends
        self.assertTrue(short["hit rate"]["pass"] and short["hit rate"]["flag"])
        self.assertTrue(short["spawns per minute"]["advisory"] and short["spawns per minute"]["flag"])
        sheet = rows({"min_swings": 0, "monster": "giant rat", "rates": False})
        self.assertFalse(sheet["hit rate"]["pass"])
        self.assertNotIn("wanderers per minute", sheet)
        self.assertNotIn("spawns per minute", sheet)
        watch = rows({"min_swings": 0, "rates": True, "gate_wanders": True})
        self.assertFalse(watch["spawns per minute"]["pass"])
        self.assertFalse(watch["wanderers per minute"]["pass"])

    def test_stop_on_count_pooled_or_on_time(self):
        import replay

        class R:
            def __init__(self, text):
                self.text = text

            def stream(self):
                return self.text
        rx = r"You \w+ giant rat"
        with tempfile.TemporaryDirectory() as d:
            mk = lambda me: {"count": rx, "at_least": 3, "minutes": 30,  # noqa: E731
                             "pool_file": os.path.join(d, f"s.new.{me}"),
                             "pool_glob": os.path.join(d, "s.new.*")}
            a, b = mk(1), mk(2)
            self.assertIsNone(replay.stop_reached(a, R("You hit giant rat\n"), str, now=0))
            self.assertIsNone(replay.stop_reached(b, R("You miss giant rat\n"), str, now=0))
            got = replay.stop_reached(a, R("You hit giant rat\nYou hit giant rat\n"), str, now=5)
            self.assertEqual((got["reason"], got["count"], got["pooled"]), ("count", 2, 3))
        t = {"count": rx, "at_least": 2000, "minutes": 30}
        self.assertIsNone(replay.stop_reached(t, R(""), str, now=100))
        got = replay.stop_reached(t, R("You hit giant rat"), str, now=100 + 1800)
        self.assertEqual((got["reason"], got["count"], got["elapsed"]), ("minutes", 1, 1800))

    def test_report_pools_the_shards_of_one_scenario(self):
        with tempfile.TemporaryDirectory() as d:
            side = {"swings": {"hit": 700, "miss": 300}, "damage": {"5": 700}}
            for k in (1, 2, 3):
                os.makedirs(os.path.join(d, f"r{k}", "m2", "fight-loop"))
                with open(os.path.join(d, f"r{k}", "m2", "fight-loop", "summary.json"), "w") as f:
                    json.dump({"status": "fail", "stats": [],
                               "samples": {"orig": side, "new": side,
                                           "durations": [600.0, 600.0]}}, f)
            summary = stats.collect([os.path.join(d, f"r{k}") for k in (1, 2, 3)], "m2")
        s = summary["scenarios"][0]
        self.assertTrue(s["pooled"])
        hit = s["checks"][0]
        self.assertEqual(hit["what"], "hit rate")
        self.assertEqual(hit["orig"], [2100, 3000])
        self.assertTrue(hit["enough"] and hit["pass"])

    def test_the_report_says_whether_the_originals_start_up_finished(self):
        for init, want in (("not finished: State: 2 (Rooms)",
                            "orig start-up cycle not finished: State: 2 (Rooms)"),
                           ("finished: State: 9 (Finished), 41 min after the original's start",
                            "orig start-up cycle finished: State: 9 (Finished)")):
            with tempfile.TemporaryDirectory() as d:
                stats.write_summary(d, {"group": "m2", "alpha": 0.01, "pass": True,
                                        "scenarios": [], "orig_init": init})
                with open(os.path.join(d, "stats.md"), encoding="utf-8") as f:
                    self.assertIn(want, f.read())

    def test_the_repeat_override(self):
        script = {"steps": [{"op": "a"}, {"op": "b"}, {"op": "c"}]}
        m2 = {"repeat": {"first": 1, "last": 1, "times": 400}}
        os.environ["SILVERMERE_M2_REPEAT"] = "3"
        try:
            self.assertEqual(len(stats.expand_script(script, m2)["steps"]), 5)
        finally:
            del os.environ["SILVERMERE_M2_REPEAT"]
        self.assertEqual(len(stats.expand_script(script, m2)["steps"]), 402)


class ScriptTests(unittest.TestCase):
    def test_the_sheet_run_expands_to_its_cycles(self):
        sc = os.path.join(HERE, "..", "scenarios", "m2")
        with open(os.path.join(sc, "swings-rat.json"), encoding="utf-8") as f:
            overlay = json.load(f)
        with open(os.path.join(sc, "scripts", "swings.json"), encoding="utf-8") as f:
            script = json.load(f)
        rep = overlay["m2"]["repeat"]
        full = stats.expand_script(script, overlay["m2"])
        cycle = rep["last"] - rep["first"] + 1
        self.assertEqual(len(full["steps"]), len(script["steps"]) + cycle * (rep["times"] - 1))
        self.assertTrue(script["steps"][rep["first"]]["stop_check"])
        self.assertEqual(script["steps"][rep["last"]]["until"], "\\*Combat Off\\*")
        self.assertEqual(len(stats.round_steps(full)), 3 * rep["times"])
        self.assertIs(stats.expand_script(script, {}), script)

    def test_every_m2_overlay_names_a_script_and_a_sequence(self):
        sc = os.path.join(HERE, "..", "scenarios")
        for name in sorted(os.listdir(os.path.join(sc, "m2"))):
            if not name.endswith(".json"):
                continue
            with open(os.path.join(sc, "m2", name), encoding="utf-8") as f:
                o = json.load(f)
            self.assertEqual(o["scenario"], "m2/" + name[:-5])
            self.assertTrue(o["statistical"])
            self.assertIn("sequence", o["m2"])
            self.assertTrue(os.path.isfile(os.path.join(sc, o["script"])), o["script"])


class RecordTests(unittest.TestCase):
    def test_new_player_and_monsters_from_sqlite(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "new.db")
            c = sqlite3.connect(path)
            cols = ", ".join(f"{f} INTEGER" for f in stats.PLAYER_FIELDS)
            c.execute(f"create table characters (id INTEGER, firstname TEXT, {cols})")
            c.execute("create table character_inventory (character_id INTEGER, slot INTEGER, "
                      "item INTEGER, uses INTEGER)")
            c.execute("create table character_worn (character_id INTEGER, slot INTEGER, "
                      "item INTEGER, uses INTEGER)")
            c.execute("create table active_monsters (number INTEGER, map INTEGER, room INTEGER)")
            vals = {f: i for i, f in enumerate(stats.PLAYER_FIELDS)}
            c.execute(f"insert into characters values (1, 'Aldric', "
                      f"{', '.join(str(vals[f]) for f in stats.PLAYER_FIELDS)})")
            c.executemany("insert into character_inventory values (1, ?, ?, ?)",
                          [(0, 100, -1), (1, 0, 0), (2, 67, 3)])
            c.execute("insert into active_monsters values (1, 1, 2150)")
            c.commit()
            c.close()
            player, mons = stats.load_new_records(path, "aldric")
        orig = dict(vals, inventory=[(67, 3), (100, -1)], worn=[])
        self.assertEqual(stats.compare_records(orig, player), [])
        orig["hp"] = 99
        self.assertEqual(stats.compare_records(orig, player),
                         [{"field": "hp", "orig": 99, "new": vals["hp"]}])
        self.assertEqual(stats.compare_records(orig, player, {"hp"}), [])
        self.assertEqual(stats.compare_monsters([{"number": 4, "map": 1, "room": 2150}], mons), [])
        self.assertEqual(stats.compare_monsters([], mons),
                         [{"room": "1:2150", "orig": 0, "new": 1}])
        maps, gaps = stats.monster_maps([{"number": 4, "map": 1, "room": 2149},
                                         {"number": 5, "map": 17, "room": 3211}], mons)
        self.assertEqual(maps, [{"map": 1, "orig": 1, "new": 1}, {"map": 17, "orig": 1, "new": 0}])
        self.assertEqual(gaps, [{"map": 17, "orig": 1, "new": 0}])  # a room off: no gap on map 1

    def test_not_carried_by_design(self):
        """Run 39030247260's records: PS_PAYING (SAVEKEY is
        dropped) and a worn item's uses (uninitialised on the original)
        are not compared; any other settings bit and the worn item numbers still are."""
        orig = {"settings": 42004, "worn": [(332, 23), (333, 23)]}
        new = {"settings": 9236, "worn": [(332, -1), (333, -1)]}
        self.assertEqual(stats.compare_records(orig, new), [])
        self.assertEqual(stats.compare_records(dict(orig, settings=42005), new),
                         [{"field": "settings", "orig": 9237, "new": 9236}])
        self.assertEqual(stats.compare_records(dict(orig, worn=[(332, 23)]), new),
                         [{"field": "worn", "orig": [332], "new": [332, 333]}])

    def test_orig_player_from_a_decoded_record(self):
        rec = {f: 1 for f in stats.PLAYER_FIELDS}
        rec.update(items=[100, 0, 67], itemuses=[-1, 0, 3], worn=[0] * 20, wornuses=[0] * 20)
        p = stats.orig_player(rec)
        self.assertEqual(p["inventory"], [(67, 3), (100, -1)])
        self.assertEqual(p["worn"], [])

    def test_the_vendored_decoder_knows_the_two_tables(self):
        sys.path.insert(0, os.path.join(HERE, "..", "vendor", "play"))
        import mmud_data
        fields = [f[0] for f in mmud_data.RECORDS["WCCUSERS"][2]]
        for f in stats.PLAYER_FIELDS + ["items", "itemuses", "worn", "wornuses", "firstname"]:
            self.assertIn(f, fields)
        acm = [f[0] for f in mmud_data.RECORDS["WCCACMSR"][2]]
        for f in ("number", "map", "room"):
            self.assertIn(f, acm)


if __name__ == "__main__":
    unittest.main()
