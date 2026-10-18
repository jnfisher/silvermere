"""T-057: the m4 group's harness pieces: the game-data registry from a JSON export
(world_json.py), the repeated cycle of a statistical scenario, the scenario files themselves,
and m4_stats.py's line counting."""
import gzip
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
sys.path.insert(0, HERE)

import differential  # noqa: E402
import m4_stats  # noqa: E402
import mbbsemu_key  # noqa: E402
import replay  # noqa: E402
import world_json  # noqa: E402

M4 = os.path.join(HERE, "scenarios", "m4")


def export(d):
    """A tiny export: one room, the vine strike message triple, a monster, an item."""
    tables = {
        "WCCMP001": [{"map": 1, "number": 2150, "name": "Newhaven, Arena"}],
        "WCCMSG": [
            {"number": 14, "msgs": ["You cast vine strike at %s for %d damage!",
                                    "%s casts vine strike at you for %d damage!",
                                    "%s casts vine strike at %s!"]},
            {"number": 900, "msgs": ["The ground trembles beneath your feet.", "", ""]},
        ],
        "WCCSPELS": [{"number": 25, "name": "vine strike", "msgnum": 14}],
        "WCCKNMSR": [{"number": 1, "name": "giant rat", "arrivemsg": 0, "deathmsg": 0,
                      "atktypes": [], "hitmsgs": [], "defmsgs": [], "missmsgs": []}],
        "WCCITEMS": [{"number": 1, "hitmsg": 0, "missmsg": 0, "destroymsg": 0}],
    }
    for name, rows in tables.items():
        with gzip.open(os.path.join(d, name + ".jsonl.gz"), "wt", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")


class WorldJson(unittest.TestCase):
    def test_spell_texts_and_room_titles_classify(self):
        _, _, pt = replay.load_play()
        with tempfile.TemporaryDirectory() as d:
            export(d)
            reg = world_json.registry(pt, d)
        p, _ = reg.match_line("You cast vine strike at giant rat for 7 damage!")
        self.assertIsNotNone(p)
        p, _ = reg.match_line("The ground trembles beneath your feet.")
        self.assertIsNotNone(p, "every WCCMSG text is a pattern")
        self.assertEqual(p.name, "wccmsg")
        self.assertIn("Newhaven, Arena", reg.room_names)
        # the static registry alone does not know the second line
        bare = pt.Registry.load(data=False)
        self.assertIsNone(bare.match_line("The ground trembles beneath your feet.")[0])

    def test_a_plain_jsonl_export_is_read_too(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "WCCMSG.jsonl"), "w", encoding="utf-8") as f:
                f.write(json.dumps({"number": 1, "msgs": ["a", "b", "c"]}) + "\n")
            self.assertEqual(world_json.rows(d, "WCCMSG")[0]["number"], 1)
            with self.assertRaises(FileNotFoundError):
                world_json.rows(d, "WCCSPELS")


class Repeat(unittest.TestCase):
    def test_the_cycle_is_played_times_times(self):
        script = {"steps": [{"send": c} for c in "abcde"]}
        out = differential.expand_repeat(script, {"first": 1, "last": 2, "times": 3})
        self.assertEqual("".join(s["send"] for s in out["steps"]), "abcbcbcde")
        self.assertIs(differential.expand_repeat(script, None), script)


class Scenarios(unittest.TestCase):
    def test_every_m4_scenario_resolves(self):
        names = differential.group_scenarios("m4")
        self.assertGreaterEqual(len(names), 12)
        for name in names:
            sc = differential.load_scenario(name)
            self.assertTrue(os.path.isfile(sc["script"]), name)
            self.assertTrue(sc.get("world_patterns"), f"{name}: spell lines need the game data")
            for entry in sc.get("setup", []):
                if entry.get("script"):
                    self.assertTrue(os.path.isfile(os.path.join(HERE, "scenarios", entry["script"])),
                                    f"{name}: {entry}")
            steps = differential.read_json(sc["script"])["steps"]
            rep = sc.get("repeat")
            if rep:
                self.assertTrue(sc.get("statistical"), name)
                self.assertTrue(0 < rep["first"] <= rep["last"] < len(steps) - 4, name)
                self.assertIn("m4", sc, f"{name}: m4_stats.py needs the m4 block")
            for st in steps:
                if "until" in st:
                    __import__("re").compile(st["until"])
            if differential.read_json(sc["script"]).get("character", {}).get("name") == "Sysadm" \
                    or name == "m4/sysop":
                # the realm entry (step 4: `E`) shows wherever the run before left Sysadm
                self.assertEqual(steps[4].get("send"), "E", name)
                masks = [m["regex"] for m in differential.read_json(sc["script"])["mask"]]
                # the prompt's labels are never masked, only (outside m4/sysop) its numbers
                self.assertFalse([m for m in masks if m.startswith(r"\[HP=")], name)
                if name == "m4/sysop":
                    # T-096: the GOTO into the Support Chamber, then an equal start (TWEAK clear,
                    # class, level, hp, mana; steps 5-10): no prompt mask
                    self.assertEqual([s.get("send") for s in steps[5:11]],
                                     ["sysop goto support", "sysop tweak clear", "sysop tweak class 13", "sysop tweak level 1",
                                      "sysop tweak hp 100", "sysop tweak mana 50"])
                    self.assertEqual(sc["ignore_steps"], [1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                    # GOD INFO waits for its last part and answers the original's pauses
                    info = [s for s in steps if s.get("send") == "sysop god sysadm info"]
                    self.assertEqual(len(info), 1)
                    self.assertTrue(info[0]["pager"])
                    self.assertTrue(info[0]["until"].startswith("User abilities:"))
                    self.assertTrue(differential.read_json(sc["script"])["answer_pager"])
                    self.assertEqual([(d["card"], d.get("side") or d["command"])
                                      for d in sc["divergences"]],
                                     [("T-055", "orig"), ("T-055", "new"), ("T-055", "orig"),
                                      ("T-055", "new"), ("T-055", "orig")])
                    # LIST SPELLS needs the original's devmode: not typed
                    self.assertNotIn("sysop list spells", [s.get("send") for s in steps])
                    self.assertNotIn("sysop list spells", sc["deterministic"])
                    self.assertEqual(sc["value_masks"][0]["where"], {"key": "Life for this CHAR"})
                    self.assertFalse(sc.get("mask_prompt"))
                    self.assertFalse([m for m in masks if "HP=" in m])
                else:
                    self.assertEqual(steps[5].get("send"), "sysop tweak clear", name)
                    self.assertEqual(sc["ignore_steps"], [1, 2, 3, 4], name)
                    self.assertTrue(sc.get("mask_prompt"), name)
                    self.assertIn(r"(?<=\[HP=)-?\d+|(?<=/MA=)-?\d+|(?<=/KAI=)-?\d+", masks, name)

    def test_sysop_is_evil_for_rhudaur_and_neutral_before_town(self):
        """T-104: Rhudaur's dark monks attack a non-evil arrival and Silvermere's and Newhaven's
        guardsmen attack an evil one: GOD UNLAWFUL just before `goto rhudaur`, GOD NEUTRAL in the
        Support Chamber after it, and no GOTO to another town in between."""
        sends = [s.get("send") for s in differential.read_json(
            os.path.join(M4, "scripts", "sysop.json"))["steps"]]
        rh = sends.index("sysop goto rhudaur")
        self.assertEqual(sends[rh - 1], "sysop god sysadm unlawful")
        self.assertEqual(sends[rh + 1:rh + 4],
                         ["sysop goto support", "sysop god sysadm neutral", "sysop goto khazarad"])
        evil = rh - 1
        neutral = rh + 2
        for i, s in enumerate(sends):
            if s in ("sysop goto silvermere", "sysop goto newhaven", "sysop goto khazarad",
                     "sysop goto lostcity"):
                self.assertFalse(evil < i < neutral, "an evil character reaches town")
        self.assertEqual(sends.count("sysop god sysadm unlawful"), 1)
        self.assertEqual(sends.count("sysop god sysadm neutral"), 1)

    def test_every_exit_from_the_realm_waits_for_the_meditation(self):
        # X's silent meditation outlasted a 10 s wait on the original (m4/sysop step 28)
        seen = 0
        for d in ("scripts", "setup"):
            for f in sorted(os.listdir(os.path.join(M4, d))):
                steps = differential.read_json(os.path.join(M4, d, f)).get("steps", [])
                for i, x in enumerate(steps):
                    if (x.get("send") == "x" and x.get("until") == r"\[MAJORMUD\]:"
                            and "HP=" in (steps[i - 1].get("until") or "")):
                        seen += 1
                        self.assertGreaterEqual(x.get("timeout", 0), 90, (f, i))
        self.assertGreater(seen, 10)
        sysop = differential.read_json(os.path.join(M4, "scripts", "sysop.json"))["steps"]
        self.assertEqual([x["timeout"] for x in sysop if x.get("send") == "x"], [90, 10],
                         "the realm's exit waits; the sysop menu's X has no meditation")

    def test_quest_and_aoe_run_again(self):
        """T-084 took the `skip` off both: they are played, up to two reruns after interference."""
        for name in differential.group_scenarios("m4"):
            self.assertNotIn("skip", differential.load_scenario(name), name)
        for name in ("m4/quest", "m4/aoe"):
            self.assertEqual(differential.load_scenario(name)["max_reruns"], 2, name)
        self.assertEqual(differential.load_scenario("m4/sysop")["max_reruns"], 3)

    def test_the_aoe_fight_ends_before_the_exit(self):
        """Three giant rats (12 hit points) die to one fireball (at least 33 damage, WCCSPELS
        minval): no survivor can attack during the exit's meditation."""
        sc = differential.load_scenario("m4/aoe")
        steps = differential.read_json(sc["script"])["steps"]
        self.assertEqual([steps[i]["send"] for i in (15, 16, 17)], ["sysop monster 1"] * 3)
        # the wait for the kills searches from the cast on (silvermere answers the whole fight
        # in the cast's own step); then a bare Enter; the three steps are one block
        self.assertEqual((steps[19]["op"], steps[19]["since"]), ("sleep", "last"))
        self.assertIn("{3}", steps[19]["until"])
        self.assertEqual(steps[20]["send"], "")
        self.assertEqual(steps[20]["until"], r"\[HP=-?\d+[^\]\n]*\]:")
        self.assertEqual(sc["merge_steps"], [[18, 20]])
        self.assertEqual(sc["required_events"]["18"], ["spell_cast_room", "combat_off"])
        self.assertNotIn("19", sc["required_events"])
        exit_step = next(i for i, x in enumerate(steps) if x.get("send") == "x")
        self.assertGreater(exit_step, 20)

    def test_m4_stats_counts(self):
        self.assertEqual(m4_stats.selftest(), 0)


class MbbsemuKey(unittest.TestCase):
    """mbbsemu_key.py on a database shaped like MBBSEmu's (Accounts, AccountKeys)."""

    def test_grant_adds_the_key_once(self):
        import sqlite3
        with tempfile.TemporaryDirectory() as d:
            db = os.path.join(d, "mbbsemu.db")
            con = sqlite3.connect(db)
            con.executescript(
                "CREATE TABLE Accounts(accountId INTEGER PRIMARY KEY, userName TEXT, "
                "passwordHash TEXT, createDate TEXT NOT NULL, updateDate TEXT NOT NULL);"
                "CREATE TABLE AccountKeys(id INTEGER PRIMARY KEY, accountId INTEGER NOT NULL, "
                "accountKey TEXT NOT NULL, createDate TEXT NOT NULL, updateDate TEXT NOT NULL);"
                "INSERT INTO Accounts VALUES (1, 'guest', '', 'x', 'x'), (2, 'sysop', '', 'x', 'x');"
                "INSERT INTO AccountKeys(accountId, accountKey, createDate, updateDate) "
                "VALUES (2, 'SYSOP', 'x', 'x');")
            con.commit()
            con.close()
            self.assertIn("granted WCCSYSOP", mbbsemu_key.grant(db, "Sysop", "wccsysop"))
            self.assertIn("already", mbbsemu_key.grant(db, "sysop", "WCCSYSOP"))
            con = sqlite3.connect(db)
            rows = con.execute("SELECT accountId, accountKey FROM AccountKeys ORDER BY id").fetchall()
            con.close()
            self.assertEqual(rows, [(2, "SYSOP"), (2, "WCCSYSOP")])
            with self.assertRaises(SystemExit):
                mbbsemu_key.grant(db, "nobody", "WCCSYSOP")


class EchoAnchored(unittest.TestCase):
    """replay.anchor_on_echo: a stale prompt before a step's own echo no longer ends its wait."""

    def test_in_realm_steps_wait_for_their_echo(self):
        import re
        P = r"\[HP=-?\d+[^\]\n]*\]:"
        steps = [{"op": "connect", "until": "Username:"},
                 {"send": "sysop", "until": "Password:"},
                 {"send": "sysop", "until": "Make your selection"},
                 {"send": "E", "until": P},
                 {"send": "sysop summon torch", "until": P},
                 {"send": "", "until": P},
                 {"send": "x", "until": r"\[MAJORMUD\]:"}]
        replay.anchor_on_echo(steps)
        self.assertEqual([s["until"] for s in steps[:4]], ["Username:", "Password:",
                                                          "Make your selection", P])
        self.assertEqual(steps[5]["until"], P)
        self.assertEqual(steps[6]["until"], r"\[MAJORMUD\]:")
        rx = re.compile(steps[4]["until"])
        self.assertIsNone(rx.search("torch conjured.\r\n[HP=100/MA=50]:"))
        self.assertIsNone(rx.search("[HP=100/MA=50]:sysop summon tor"))
        self.assertTrue(rx.search("[HP=100/MA=50]:Sysop summon torch\r\ntorch conjured.\r\n[HP=100/MA=50]:"))

    def test_the_fights_are_random_steps(self):
        for name, sends in [("m4/spells", ["sysop monster 7", "vine kobold", ""]),
                            ("m4/aoe", ["sysop monster 1"] * 3 + ["cast fbal", "sleep", ""])]:
            sc = differential.load_scenario(name)
            steps = differential.read_json(sc["script"])["steps"]
            self.assertEqual([steps[i].get("send", steps[i].get("op")) for i in sc["random_steps"]], sends, name)
            det = differential.deterministic_steps(sc, [replay.StepCapture(
                i, "send", s.get("send"), 0, b"") for i, s in enumerate(steps)])
            self.assertFalse(set(det) & set(sc["random_steps"]), name)
            # each fight still has its structure checked, and the summons are compared
            self.assertTrue(sc["required_events"], name)
            self.assertTrue(set(map(int, sc["required_events"])) <= set(sc["random_steps"]), name)
            self.assertEqual(sc["event_masks"], {"monster_arrives": ["adj", "mon", "frm"]}, name)

    def test_the_m4_scripts_are_echo_anchored(self):
        for name in differential.group_scenarios("m4"):
            sc = differential.load_scenario(name)
            script = differential.read_json(sc["script"])
            if script.get("character", {}).get("name") == "Sysadm" or name == "m4/sysop":
                self.assertTrue(script.get("echo_anchored"), name)
                self.assertTrue(sc.get("bytes_from_echo"), name)
                # the settle only where the scenario is not a repeated statistical cycle
                self.assertEqual(bool(script.get("settle")), not sc.get("statistical"), name)

    def test_from_echo_cuts_the_step_before_tail(self):
        import diff
        raw = b"lunges at you!\r\n[HP=1]:[HP=1]:rest\r\nYou are now resting.\r\n"
        self.assertEqual(diff.from_echo(raw, "rest"), b"rest\r\nYou are now resting.\r\n")
        self.assertEqual(diff.from_echo(b"no echo here", "rest"), b"no echo here")
        self.assertEqual(diff.from_echo(raw, ""), raw)


class StepPaged(unittest.TestCase):
    """replay.Player.step_paged: the original's "Hit any key" pauses answered with a key."""

    class Fake:
        def __init__(self, chunks):
            self.text, self.chunks, self.sent = "", list(chunks), []

        def pos(self):
            return len(self.text)

        def stream(self):
            return self.text

        def step(self, st):
            import re
            off = self.pos()
            self.sent.append(st["send"])
            if self.chunks:
                self.text += self.chunks.pop(0)
            return {"offset": off, "matched": bool(re.search(st["until"], self.text[off:]))}

    def test_each_pause_is_answered_until_the_last_part(self):
        P = r"\[HP=-?\d+[^\]\n]*\]:"
        pager = r"Hit any key to continue\.\.\."
        st = {"send": "sysop god sysadm info", "pager": True,
              "until": r"User abilities:(?s:.*)" + P}
        replay.anchor_on_echo([{"send": "E", "until": P}, st])
        fake = self.Fake(["sysop god sysadm info\r\nName: Sysadm\r\n[HP=1]:Spells\r\nHit any key to continue...",
                          "\rSettings\r\nHit any key to continue...",
                          "\rUser abilities: Nothing special\r\n[HP=1]:"])
        player = replay.Player.__new__(replay.Player)
        player.bridge = type("B", (), {"plain": staticmethod(lambda t: t)})
        res = player.step_paged(fake, st, pager)
        self.assertEqual(fake.sent, ["sysop god sysadm info", " ", " "])
        self.assertTrue(res["matched"])
        self.assertEqual(res["paged"], 2)

    def test_no_pause_no_key(self):
        P = r"\[HP=-?\d+[^\]\n]*\]:"
        st = {"send": "sysop god sysadm info", "pager": True, "until": r"User abilities:(?s:.*)" + P}
        fake = self.Fake(["sysop god sysadm info\r\nUser abilities: x\r\n[HP=1]:"])
        player = replay.Player.__new__(replay.Player)
        player.bridge = type("B", (), {"plain": staticmethod(lambda t: t)})
        res = player.step_paged(fake, st, r"Hit any key to continue\.\.\.")
        self.assertEqual(fake.sent, ["sysop god sysadm info"])
        self.assertTrue(res["matched"])


class PagerKnownGap(unittest.TestCase):
    """The T-082 rule removes the original's pause and its erase, not the next line's colour."""

    def test_the_pause_goes_and_the_next_colour_stays(self):
        import known_gaps
        rules = [r for r in known_gaps.load() if r["card"] == "T-082"]
        text = ("line19\r\n\x1b[0;37mHit any key to continue...\x1b[0m"
                "\r\x1b[79D\x1b[K\x1b[1;32mGreen line20\r\n")
        self.assertEqual(known_gaps.mask_bytes(text, "orig", rules),
                         "line19\r\n\x1b[1;32mGreen line20\r\n")
        self.assertEqual(known_gaps.mask_bytes(text, "new", rules), text)
        kept, dropped = known_gaps.apply(
            [{"name": "unknown", "step": 3, "line": "Hit any key to continue...", "fields": {}}],
            "orig", rules)
        self.assertEqual((kept, dropped), ([], {"T-082": 1}))


class ClrprfKnownGap(unittest.TestCase):
    """The T-083 shape on the bytes of live runs 38823959731 and 38835117564: silvermere's
    ANSI_NORMAL (with the CLRLINE the original clears with it) directly after the echo, nothing else."""

    N, CL, RED = "\x1b[0;37;40m", "\x1b[79D\x1b[K", "\x1b[1;31;40m"

    def test_the_shape_accepts_exactly_the_prefix(self):
        import diff
        N, CL, RED = self.N, self.CL, self.RED
        tail = "\r\n" + CL + "\x1b[0;37m[HP=<masked>]:"
        # (send, the original after the echo, silvermere after the echo), as captured
        cases = [
            ("cast vine", CL + RED + "You must specify a target for that spell!",
             N + CL + RED + "You must specify a target for that spell!"),
            ("cast vine nobody", RED + "You do not see nobody here!",
             N + RED + "You do not see nobody here!"),
            ("read scroll", "You realize that you already know this scroll!",
             N + CL + "You realize that you already know this scroll!"),
            ("light torch", "You lit the torch.", N + "You lit the torch."),
            ("greet boatman", CL + "\x1b[0;32mThe boatman says", N + CL + "\x1b[0;32mThe boatman says"),
            ("sysop god sysadm dance", "Valid options: ADD LIFE", N + "Valid options: ADD LIFE"),
        ]
        for send, o, n in cases:
            self.assertTrue(diff.ansi_normal_after_echo(send + "\r\n" + o + tail,
                                                        send + "\r\n" + n + tail), send)

    def test_any_other_colour_difference_still_fails(self):
        import diff
        N, CL = self.N, self.CL
        o = "light torch\r\nYou lit the torch.\r\n[HP=1]:"
        for n in ("light torch\r\n\x1b[1;32mYou lit the torch.\r\n[HP=1]:",      # another colour
                  "light torch\r\nYou lit the torch.\r\n" + N + "[HP=1]:",          # not after the echo
                  "light torch\r\n" + N + N + "You lit the torch.\r\n[HP=1]:",      # two of them
                  "light torch\r\n" + N + "You lit the lamp.\r\n[HP=1]:",           # and a text change
                  "light torch\r\n" + CL + "You lit the torch.\r\n[HP=1]:"):        # CLRLINE alone
            self.assertFalse(diff.ansi_normal_after_echo(o, n), repr(n))
        # the original's side with the prefix and the new without: not this gap
        self.assertFalse(diff.ansi_normal_after_echo("x\r\n" + N + "a", "x\r\na"))

    def test_counted_under_t083_in_compare(self):
        import normalise
        masks = normalise.load_masks()
        mk = lambda raw: [replay.StepCapture(0, "connect", None, 0, b"Hi\r\n"),
                          replay.StepCapture(1, "send", "light torch", 4, raw)]
        # the rule left known_gaps.json when T-083 added the clear; a scenario divergence
        # carries the same shape so the classifier stays tested
        rule = {"card": "T-083", "why": "test copy", "side": "new",
                "shape": "ansi_normal_after_echo"}
        sc = {"name": "t/t083", "script": "/nonexistent.json", "ignore_host_banner": True,
              "deterministic": ["light torch"], "divergences": [rule]}
        o = b"light torch\r\nYou lit the torch.\r\n[HP=10]:"
        n = b"light torch\r\n\x1b[0;37;40mYou lit the torch.\r\n[HP=10]:"
        res = differential.compare(sc, mk(o), mk(n), masks)[0]
        self.assertEqual(res["status"], "pass", res)
        self.assertEqual(res["known_gaps"]["T-083"].get("new_bytes"), 1)
        other = b"light torch\r\n\x1b[1;32mYou lit the torch.\r\n[HP=10]:"
        self.assertEqual(differential.compare(sc, mk(o), mk(other), masks)[0]["status"], "fail")
        # without the rule (known_gaps.json no longer has it) the prefix fails
        del sc["divergences"]
        self.assertEqual(differential.compare(sc, mk(o), mk(n), masks)[0]["status"], "fail")


if __name__ == "__main__":
    unittest.main()

