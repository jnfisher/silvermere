"""Writes tools/differential/scenarios/m4/ (T-057): the scenario overlays, their scripts and
silvermere's Sysadm preparation. The scripts share their login, logoff, masks and the sheet
steps, so they are generated rather than kept by hand; edit here and run
`python3 -I tools/differential/m4_scenarios.py .` from the repository root."""
import json, os, sys
root = sys.argv[1]
SC = os.path.join(root, "tools/differential/scenarios")
M4 = os.path.join(SC, "m4")
os.makedirs(os.path.join(M4, "scripts"), exist_ok=True)
P = r"\[HP=-?\d+[^\]\n]*\]:"
def rd(p): return json.load(open(p))
# leaving the realm: X starts a silent meditation of 6 ticks of command delay (cmd_quit),
# one dot a tick; under MBBSEmu's load the original has taken more than
# 10 s for it (m4/sysop step 28 on a live run), so every exit to [MAJORMUD]: from inside the
# realm waits as long as the other scripts' (wr checks each script)
EXIT_TIMEOUT = 90
def check_exits(o):
    steps = o.get("steps", [])
    for i, x in enumerate(steps):
        if (x.get("send") == "x" and x.get("until") == r"\[MAJORMUD\]:" and i
                and "HP=" in (steps[i - 1].get("until") or "")):
            assert x.get("timeout", 0) >= EXIT_TIMEOUT, (o.get("scenario"), i, x)
def wr(p, o):
    check_exits(o)
    with open(os.path.join(M4, p), "w") as f:
        json.dump(o, f, indent=1, ensure_ascii=False); f.write("\n")
# the prompt's numbers only, never its labels: HP=, /MA= or /KAI= and the brackets stay
# byte-compared (T-055's script masks the same numbers; its m4 copy compares them too)
PROMPT_DIGITS = {"regex": r"(?<=\[HP=)-?\d+|(?<=/MA=)-?\d+|(?<=/KAI=)-?\d+",
                 "why": "the prompt's hit point and mana numbers (not its labels)"}
SYSOP_SCRIPT = rd(os.path.join(SC, "m4_sysop.script.json"))
T055_MASKS = [m for m in SYSOP_SCRIPT["mask"] if m["regex"] != PROMPT_DIGITS["regex"]]
assert len(T055_MASKS) == len(SYSOP_SCRIPT["mask"]) - 1
MASKS = T055_MASKS + [
    PROMPT_DIGITS,
    {"regex": r"Also here: .*", "why": "monsters that happen to be around"},
]
DEV = ("Needs the developer code on both sides: on the original CODEC_OF_TWINKS=78 in "
       "wccmmud.ini (STAGE_CODEC_OF_TWINKS or docker -e CODEC_OF_TWINKS) "
       "and the BBS key WCCSYSOP on the sysop account (mbbsemu_key.py; "
       "TWEAK, SUMMON, EXPRESS_TRAIN, MONSTER), on silvermere the DEBUG grant (`silvermere account grant sysop DEBUG`); and "
       "the BBS sysop account sysop/sysop with its character Sysadm on both (docs/testing.md, M4).")

def login():
    return [{"op": "connect", "until": "Username:", "timeout": 20},
            {"send": "sysop", "until": "Password:"},
            {"send": "sysop", "until": "Make your selection"},
            {"send": "M", "until": r"\[MAJORMUD\]:", "note": "MajorMUD from the BBS menu"},
            {"send": "E", "until": P, "timeout": 20, "required": True,
             "note": "enter the realm as Sysadm"}]

def out():
    return [{"send": "x", "until": r"\[MAJORMUD\]:", "timeout": 90,
             "note": "the silent meditation takes a few ticks"},
            {"send": "X", "until": "Make your selection"},
            {"send": "X", "until": "Are you sure"},
            {"send": "Y", "until": ""}]

def st(send, note=None, until=P, **kw):
    d = {"send": send, "until": until}
    if note:
        d["note"] = note
    d.update(kw)
    return d

def sheet(cls, level, hp=None):
    s = [st("sysop tweak clear", "a known start: TWEAK CLEAR empties the pack, the worn items "
            "and the keys and zeroes the money, so an earlier scenario's "
            "abort on one side cannot carry over"),
         st(f"sysop tweak class {cls}", "the fixed sheet: class (TWEAK)"),
         st("sysop tweak experience 10000000",
            "experience for any level below 16: a level above the experience is taken down at "
            "the next entry with its character points, and a negative CP total resets the stats "
            "into the editor, even after a run cut short"),
         st(f"sysop tweak level {level}", "and level")]
    if hp:
        s.append(st(f"sysop tweak hp {hp}", "hit points to outlast the scenario"))
        s.append(st("sysop tweak lives 9", "and the lives a death would take"))
    return s

def go(room, mp=1, note=None):
    return st(f"sysop express_train {room} {mp}", note or f"to {mp}:{room} (EXPRESS_TRAIN)")

# Sysadm's scenarios: the BBS host's username, password and menu steps, as m1's scenarios skip
# them, and the realm entry (step 4), whose room and HP are wherever the run before left the
# character on each side (a death there sends it to the temple); the prompt's HP/MA are
# masked throughout (`mask_prompt`), the staged characters never having the same ones
SYSADM_ENTRY = {"ignore_steps": [1, 2, 3, 4], "mask_prompt": True,
                # a step's bytes from its own echo on: before it is the step before's tail
                "bytes_from_echo": True}
# seconds of quiet after each in-realm step (replay `settle`): the original sends some
# replies in two parts around a prompt (a used-up item's destroy message after the One
# Time Cast's prompt), which must not fall into the next step
SETTLE = 0.5

def script(name, desc, steps, desc_tail="", masks=()):
    # back to level 1 before leaving: a level above the experience is taken down at the
    # next entry with its character points, and a negative CP total resets the stats into
    # the editor, which the next scenario's `E` would meet
    tail = [st("sysop tweak level 1", "the level the experience allows, for the next entry")]
    return {"scenario": f"m4/{name}", "port": 2323, "description": desc + " " + DEV + desc_tail,
            "character": {"user": "sysop", "password": "sysop", "name": "Sysadm"},
            "mask": MASKS + list(masks), "echo_anchored": True,
            "steps": login() + steps + tail + out()}

def scenario(name, steps, desc, deterministic, **kw):
    sc = script(name, desc, steps, kw.pop("desc_tail", ""), kw.pop("masks", ()))
    if not kw.get("statistical"):
        sc["settle"] = SETTLE
    wr(f"scripts/{name}.json", sc)
    # steps marked `random` (st(..., random=True)): their events are not diffed
    rnd = [i for i, s in enumerate(sc["steps"]) if s.get("random")]
    if rnd:
        kw.setdefault("random_steps", rnd)
    # and the events such a step must still show on both sides (`required_events`)
    req = {str(i): s["needs"] for i, s in enumerate(sc["steps"]) if s.get("needs")}
    if req:
        kw.setdefault("required_events", req)
    # a summoned monster's adjective (and so its name) and the side it comes from are random
    if any(s.get("send", "").startswith("sysop monster") for s in sc["steps"]):
        kw.setdefault("event_masks", {"monster_arrives": ["adj", "mon", "frm"]})
    o = {"scenario": f"m4/{name}", "script": f"m4/scripts/{name}.json",
         "deterministic": deterministic, "statistical": kw.pop("statistical", False),
         "setup": [], "ignore_host_banner": True, "world_patterns": True,
         **SYSADM_ENTRY}
    o.update(kw)
    wr(f"{name}.json", o)
    return o

# ---------------------------------------------------------------- use (T-053 m4_use)
use = sheet(13, 2, 100) + [st("sysop tweak mana 50", "mana, current and maximum")] + [go(2140)] + [
    st("sysop summon scroll of vine strike", "SUMMON: an item into the pack"),
    st("use scroll of vine strike", "learn_scroll, or already_known when an earlier run left the spell in the book"),
    st("spells", "vine strike in the book"),
    st("sysop summon scroll of vine strike"),
    st("read scroll of vine strike", "already_known: the scroll stays"),
    st("drop scroll", "the second scroll goes"),
    st("sysop summon torch"),
    st("light torch", "light_ok"),
    st("light", "light_level"),
    st("sysop summon torch"),
    st("use torch", "a second light source while one burns"),
    st("sysop summon minor healing potion"),
    st("drink minor healing potion", "receive_hp: the amount is random, events only"),
    st("sysop summon iron ration"),
    st("eat iron ration", "the line, the One Time Cast, the destroy message"),
    # the key door at the Marble Passage (1:2225), its Key exit south to the Large Chamber
    # (iron key, item 338): mongroup 0, maxlevel 0, genmonster 0 and no specmon, so nothing is
    # generated there, and few wanderers: only montype-0 monsters (the
    # dummies and named NPCs), Angels (montype 37, which a room without flags lets in) and Quest monsters (38) may come in; the Chamber's troll
    # never wanders (followpct 100) and is of another montype. The Crypt's
    # Sealed Tomb (1:1309) is the mummies' (mongroup 7) and a mummy came in on a live run
    go(2225, 1, "Marble Passage (1:2225): the Key exit south (item 338); no generation, few wanderers"),
    st("open s", "locked"),
    st("sysop summon iron key"),
    st("use iron key s", "You successfully unlocked the door."),
    st("use iron key s", "The door was not locked."),
    st("i"),
    # both torches go, the lit one with its burn count (timing): the next scenarios, and
    # m4/sysop's GOD INFO inventory, start from the same pack on both servers
    st("drop torch", "one of the two torches"),
    st("drop torch", "the other"),
    st("i", "no torch left"),
    go(2140),
]
scenario("use", use,
         "T-057 (T-053's m4_use) as the sysop's character made a level-2 druid: a scroll of vine "
         "strike read and a second one already known, a torch lit and the light level, a second "
         "light refused, a potion drunk, a ration eaten, the iron key on the Marble Passage's door.",
         ["sysop summon scroll of vine strike", "read scroll of vine strike", "sysop summon torch", "light torch",
          "light", "use torch", "sysop summon minor healing potion", "sysop summon iron ration",
          "eat iron ration", "open s", "sysop summon iron key", "use iron key s", "i"])

# ---------------------------------------------------------------- spells (T-050)
# staged at the Village Entrance (1:2140): mongroup 0, maxlevel 0, genmonster 0, so nothing
# is generated there (generate_monster), and few wanderers: a monster
# enters only a room of its own montype, and montype 0 is the dummies and
# named NPCs, besides Angels (37, which a room without flags lets in) and Quest
# monsters (38), which go anywhere. The Arena (1:2150) is mongroup 6 and an arena room (90%)
spells = sheet(13, 2, 300) + [go(2140, 1, "Newhaven, Village Entrance: no generation, few wanderers"),
    st("sysop summon scroll of vine strike", "SUMMON: the scroll (an item name wins over a spell's)"),
    st("use scroll of vine strike", "learned, or already known after m4/use"),
    st("sysop summon mend", "SUMMON: a spell into the book (no item is called mend)"),
    st("cast", "CAST alone out of combat: Syntax"),
    st("invoke vine", "a druid may not invoke"),
    st("cast vine", "vine strike needs a target"),
    st("cast vine nobody", "no such target"),
    # the fight is random on both servers (the monster's adjective, cast failures, its
    # attacks): its events are not diffed (`random_steps`), its lines must still be known;
    # the rates are the cast-* scenarios' (m4_stats.py)
    # the summoned monster attacks on its own timing (random): its arrival is required
    st("sysop monster 7", "a kobold thief here (MONSTER)", random=True,
       needs=["monster_arrives"]),
    st("vine kobold", "spell_engage: `*Combat Engaged*`, then the recast each round", random=True,
       needs=["combat_engaged"]),
    st("", "rounds until the kill or the end of the fight",
       until=r"(?:You gain \d+ experience\.|\*Combat Off\*)(?s:.*)\[HP=", timeout=180, random=True,
       needs=["spell_cast_you", "combat_off"]),
    st("rest", "spell_rest: rest_start"),
    st("cast mend", "casting ends resting: the prompt has no (Resting)"),
    st("sysop tweak mana 0", "spell_no_mana"),
    st("cast mend", "no_mana"),
    go(2140),
]
scenario("spells", spells,
         "T-057 (T-050) as a level-2 druid: the CAST refusals byte-compared, vine strike at a kobold "
         "thief at the Village Entrance (no generation, few wanderers), rest then mend, no mana.",
         ["cast", "invoke vine", "cast vine", "cast vine nobody", "rest", "sysop tweak mana 0"])

# ---------------------------------------------------------------- aoe
# hit points the three goblins cannot take in a fight however long the casts fail: at 0 the
# original refuses the exit
aoe = sheet(12, 15, 30000) + [go(2489, 1, "Cleared Fields (1:2489): a normal room, no lair"),
    st("sysop summon fireball", "SUMMON finds the scroll of fireball first"),
    st("use scroll of fireball", "learned, or already known"),
    st("sysop tweak mana 2000", "fireball costs 34: enough for the whole fight (short of mana the "
       "auto-combat goes on silently without a cast and may never end)"),
    # random on both servers (adjectives, cast failures, attacks): not diffed as events
    # the summons: the goblins attack on their own timing (random), each arrival required
    st("sysop monster 1", random=True, needs=["monster_arrives"]),
    st("sysop monster 1", random=True, needs=["monster_arrives"]),
    st("sysop monster 1", 'three giant rats (12 hit points each: one fireball kills all three)',
       random=True, needs=["monster_arrives"]),
    st("cast fbal", "by hand an area attack spell engages; the cast comes with the round",
       random=True),
    {"op": "sleep", "seconds": 120, "since": "last", "until": '(?:\\*Combat Off\\*(?s:.*?)){3}\\[HP=',
     "random": True, "note": "all three rats dead: one *Combat Off* per kill, searched from the cast on. silvermere answers the whole fight within the cast's own step, the original a tick later, so the wait cannot be for text after this step starts (run 38918299935: 'explodes' had already come)"},
    st("", 'a bare Enter once the output has been quiet for 2 s: the three rats died to one fireball, so the room is empty and no attack can reach the exit. Their deaths and the *Combat Off* of each kill come in the same burst as the explosion (step 19) or just after it, so steps 19-20 are one block (merge_steps) and the structure is required of the block',
       timeout=30, random=True, needs=["combat_off"]),
    go(2140),
]
# T-084: three giant rats (12 hit points) die to the one fireball that lands, so the fight is over
# before the exit and the steps from the cast on are one block; with goblins the first kill ended
# the caster's autocombat while survivors went on hitting him, and an attack during the exit's
# meditation cancels it (stop_users_exit)
scenario("aoe", aoe,
         "T-057 spell_aoe: a level-15 mage casts fireball with three giant rats in the room; never resisted; events compared with the amounts masked. T-084: giant rats (monster 1, 12 hit points), not goblins (90): a fireball does at least 33 damage (WCCSPELS fireball minval 33) and hits every monster in the room, so the one cast that lands kills all three together and the fight is over before the exit. With goblins the first kill ended the caster's autocombat (distribute_experience; *Combat Off*) while survivors went on hitting him, and an attack during the exit's meditation cancels the exit (stop_users_exit).",
         [], desc_tail=' Steps 18-20 are one block (merge_steps): the cast, the wait for the three kills, a bare Enter once the output is quiet.',
         random_steps=[15, 16, 17, 18, 19, 20],
         required_events={"15": ["monster_arrives"], "16": ["monster_arrives"],
                          "17": ["monster_arrives"], "18": ["spell_cast_room", "combat_off"]},
         event_masks={"monster_arrives": ["adj", "mon", "frm"]},
         merge_steps=[[18, 20]],
         note="T-084: three giant rats die to one fireball (see the script), so the fight is over before the exit; steps 18-20 are one block",
         max_reruns=2)

# ---------------------------------------------------------------- monster casts (statistical)
mc = sheet(13, 10, 30000) + [go(439, 9, "Torchlit Cave (9:439)"),
    st("sysop monster 481", "an orc shaman"),
    st("attack orc"),
    st("", "one round", until=P, timeout=10, round=True),
    go(2140),
]
# login 5, sheet 5, express_train, monster, attack: the round is step 13
scenario("monster-cast", mc,
         "T-057 monster_cast: an orc shaman's casts at the player over 600 rounds: the rates of "
         "cast_on_you_generic, you_resisted and cast_at_you_failed compared (m4_stats.py).",
         [], statistical=True, repeat={"first": 13, "last": 13, "times": 600},
         m4={"rates": ["cast_on_you", "you_resisted", "cast_at_you_failed"]})

# ---------------------------------------------------------------- room spell (statistical)
room = sheet(13, 10, 30000) + [go(991, 7, "Lava Pool (7:991): roomspell 526, magma heat"),
    st("", "6 s: one room-spell line per two medium cycles",
       until=r"damage", timeout=15),
    go(2140),
]
scenario("room", room,
         "T-057 room_spell: the Lava Pool's magma heat once every 6 seconds, 200 times: the "
         "interval and the damage histogram compared (m4_stats.py).",
         [], statistical=True, repeat={"first": 11, "last": 11, "times": 200},
         m4={"room_spell": True})

# ---------------------------------------------------------------- stealth (T-103)
# TRACK, SNEAK and HIDE with their refusals, and the other T-103 commands, where no wanderer
# comes (the Sysop Support Chamber, 1:164). The steps without a delay come first and are
# byte-compared. Then the three that cost a tick of delay and answer 2 (RC_NOPROMPT: no
# prompt follows), so each waits for its own line; each is followed by a bare Enter, which is
# queued until the delay runs out and then prompts, so the next attempt is not queued behind
# it and its line stays in its own step. SNEAK and HIDE roll against each server's own
# random numbers (the attempt, then on a miss the perception roll that decides whether "You don't
# think ..." is printed): they are random steps that must show the attempt on both sides.
# MAP is in m4/assets (T-107), which runs with the assets import unpacks.
WAIT_DELAY = "a bare Enter: queued until the delay has run out, then the prompt"
stealth_quiet = [
    st("track", "the syntax (cmd_track)"),
    st("sneak to the north", "more words than SNEAK x: the syntax"),
    st("feedback", "the syntax"),
    st("disarm trap north", "no trap to the north (cmd_disarm)"),
    st("tame sysadm", "not implemented in 1.11p (cmd_tame): not a command"),
    st("identity sysadm", "not implemented (cmd_identity)"),
    st("track nosuchname", "no one of that name: the tracking fails with no roll, a tick of "
       "delay, no prompt", until=r"tracking skills fail you"),
]
stealth_rolls = [
    st("", WAIT_DELAY, timeout=20),
    st("sneak", "the attempt, a tick of delay, no prompt; its rolls are "
       "each server's own", until=r"Attempting to sneak\.\.\.", timeout=20,
       random=True, needs=["sneak_attempt"]),
    st("", WAIT_DELAY, timeout=20),
    st("hide", "the attempt, no prompt (cmd_hide); its rolls are each "
       "server's own", until=r"Attempting to hide\.\.\.", timeout=20,
       random=True, needs=["hide_attempt"]),
    st("", WAIT_DELAY, timeout=20),
]
stealth_pre = sheet(13, 2, 100) + [
    st("sysop goto support", "the Sysop Support Chamber (1:164), where no wanderer comes")]
scenario("stealth", stealth_pre + stealth_quiet + stealth_rolls,
         "T-103: TRACK, SNEAK and HIDE with their syntax lines, refusals and attempts, the "
         "FEEDBACK and DISARM TRAP refusals, and TAME and IDENTITY, which 1.11p never "
         "implemented.",
         [x["send"] for x in stealth_quiet])

# ---------------------------------------------------------------- assets (T-107)
# The text files the engine reads from the assets directory, which `silvermere import` now
# unpacks (on the original, stage.sh unzips the same archives into the module directory, as
# WCCMISC.BAT does): MAP in the Sysop Support Chamber sends its map file WCCMAP01.ANS line by
# line (cmd_map), and a gang house room (15:857, mongroup 0, no generated
# monster) is a FILE DESCRIPTION room naming WCC85715.HSE: the arrival shows the file's first
# line as the title and LOOK the whole file. Not 15:850,
# the house's entrance: its placed white banner (item 1058) is put back by a cold start's preload
# and swept by every midnight cleanup of a closed house, so whether it shows depends on each server's uptime; 15:857 has no
# placed or floor items.
assets = [
    st("sysop goto support", "the Sysop Support Chamber (1:164), whose map file is WCCMAP01.ANS"),
    st("map", "WCCMAP01.ANS from the assets, then an empty line"),
    go(857, 15, "to a gang house room (15:857): its title is WCC85715.HSE's first line"),
    st("look", "the room's description: all of WCC85715.HSE"),
    st("sysop goto support", "back to the quiet room"),
]
scenario("assets", sheet(13, 2, 100) + assets,
         "T-107: MAP and a gang house FILE DESCRIPTION room, both read from the text files "
         "import unpacks into the assets directory. The room is 15:857, which has no placed "
         "items: a placed item such as 15:850's white banner is put back by a cold start and "
         "swept by each midnight cleanup of a closed house, so it would depend on each "
         "server's uptime.",
         [x["send"] for x in assets])

# ---------------------------------------------------------------- the 5,000-cast runs
def cast_run(name, cls, level, spell, short, room, mp, monster, target, cycles, desc):
    steps = sheet(cls, level, 30000) + [go(room, mp), st(f"sysop summon {spell}"),
        st(f"use scroll of {spell}", "SUMMON finds the scroll first: learned, or already known"),
        st(f"sysop tweak level {level}", "the cycle: the fixed sheet again (level, mana), a monster, the cast, the fight"),
        st("sysop tweak mana 200"),
        st("sysop tweak hp 30000"),
        st(f"sysop monster {monster}"),
        st(f"{short} {target}" if short != "fbal" else "cast fbal"),
        st("", "the fight until the kill (or the fight's end)" if short != "fbal" else
           "one fireball: its explosion line or its failure (an area cast per cycle)",
           until=(r"(?:You gain \d+ experience\.|\*Combat Off\*|You have been killed|You do not see)(?s:.*)\[HP="
                  if short != "fbal" else
                  r"(?:explodes, causing \d+ damage!|You attempt to cast fireball, but fail\.)(?s:.*)\[HP="),
           timeout=300),
        go(2140)]
    first = 5 + 5 + 1 + 1 + 1  # login 5, sheet 5, express_train, summon, use
    scenario(name, steps, desc, [], statistical=True,
             repeat={"first": first, "last": first + 5, "times": cycles},
             m4={"spell": spell, "casts": 5000})

for mon, mname, tname in [(1, "rat", "rat"), (7, "kobold", "kobold")]:
    cast_run(f"cast-vine-{mname}", 13, 2, "vine strike", "vine", 2150, 1, mon, tname, 1700,
             f"T-057 statistical: vine strike from the fixed level-2 druid at the {mname} until "
             "5,000 casts: damage histogram (chi-square homogeneity) and fail rate (two proportions) "
             "between the servers (m4_stats.py).")
    cast_run(f"cast-fbal-{mname}", 12, 15, "fireball", "fbal", 2489, 1, mon, tname, 5100,
             f"T-057 statistical: fireball from the fixed level-15 mage at the {mname} until "
             "5,000 casts: damage histogram and fail rate, and no resisted line (m4_stats.py).")

# ---------------------------------------------------------------- the T-054/T-055/T-056 scripts
aldric = [{"side": "orig", "scenario": "entry-new-account"},
          {"side": "orig", "scenario": "creation-fsd"},
          {"side": "new", "script": "m1/setup/new-aldric.json"}]
for name, base, setup in [("quest", "m4-quest", aldric), ("sysop", "m4_sysop", []),
                          ("hangup", "hangup-next-login", aldric),
                          ("cleanup", "midnight-cleanup", aldric)]:
    o = rd(os.path.join(SC, f"{base}.json"))
    o["scenario"] = f"m4/{name}"
    o["setup"] = setup
    o["ignore_host_banner"] = True
    o["world_patterns"] = True
    o["note"] = f"T-057: the {base} scenario (same script) in the m4 group."
    if name == "sysop":
        # T-055's script with an equal start after the entry: TWEAK hp/mana set the current
        # and maximum value on both servers, so at full HP and mana
        # nothing regenerates and the prompts are byte-compared, numbers too (no prompt
        # mask, no mask_prompt). The setup steps print the prompt from before them: ignored.
        sc = json.loads(json.dumps(SYSOP_SCRIPT))
        assert sc["steps"][4]["send"] == "E"
        sc["mask"] = T055_MASKS
        sc["echo_anchored"] = True   # each step waits for its own echo (replay.anchor_on_echo)
        setup = [st(f"sysop {'' if f.startswith('goto') else 'tweak '}{f} {v}".strip(), n, until=sc["steps"][4]["until"], timeout=10)
                 for f, v, n in [("goto support", "", "T-096: first of all into the Sysop Support Chamber "
                                  "(1:164: mongroup 0, no room spell), so the sheet's steps and the help "
                                  "list are played where no wanderer comes"),
                                 ("clear", "", "an equal start: nothing carried (TWEAK CLEAR)"),
                                 ("class", 13, "an equal start: a druid (MA in the prompt)"),
                                 ("level", 1, "level 1, which any experience allows"),
                                 ("hp", 100, "hit points, current and maximum"),
                                 ("mana", 50, "mana, current and maximum")]]
        sc["steps"][5:5] = setup
        # GOD INFO shows the status, the BBS userid and the inventory,
        # then one part a poll up to the abilities and a prompt: the
        # step waits for that last part on both servers. The original asks the host for page mode
        # (`btuxnf`, "Hit any key to continue..."), but MBBSEmu at
        # the pinned tag ignores btuxnf (MBBSEmu v1.0-alpha-090826,
        # HostProcess/ExportedModules/Galgsbl.cs:350-354), so the live original never pauses
        # (no live run has counted the T-082 gap). replay's answer
        # to a pause (`answer_pager`, replay.step_paged) and the T-082 rule (known_gaps.json)
        # are kept for a host that pages; whether silvermere pages is an open question
        info = [i for i, x in enumerate(sc["steps"]) if x.get("send") == "sysop god sysadm info"]
        assert len(info) == 1
        sc["steps"][info[0]].update(
            pager=True, until=r"User abilities:(?s:.*)" + sc["steps"][4]["until"], timeout=30,
            note="status, userid, inventory; then the spells, profile and abilities a part a poll")
        sc["answer_pager"] = r"Hit any key to continue\.\.\."
        sc["settle"] = SETTLE
        # LIST SPELLS needs the original's developer mode, set only on the developers' own BBS
        # registration numbers, which MBBSEmu is not: there the original
        # answers "No list command done." silvermere folds developer mode into the DEBUG grant
        # (docs/differences.md, the developer gate), so the step cannot compare like with
        # like; the layer-3 test (m4_sysop.rs) has it
        ls = [i for i, x in enumerate(sc["steps"]) if x.get("send") == "sysop list spells"]
        assert len(ls) == 1
        del sc["steps"][ls[0]]
        sc["scenario"] = "m4/sysop"
        sc["description"] += (" T-057: an equal start after the entry (class, level, hp, mana);"
                              " GOD INFO waits for its last part, the original's pauses answered.")
        # T-055's exit from the realm waits 10 s, too short for the original's meditation
        ex = [i for i, x in enumerate(sc["steps"]) if x.get("send") == "x"
              and x.get("note") == "leave the realm"]
        assert len(ex) == 1
        sc["steps"][ex[0]]["timeout"] = EXIT_TIMEOUT
        # T-096: no step but a GOTO's own is played where wanderers or the Temple's ambient
        # lines (WCCMSG 2676-2678, room spell 925) can reach it. GOTO NOWHERE (its screen is
        # the same from any room) moves up to the start room, and the trip back to town ends
        # in the Sysop Support Chamber (1:164: mongroup 0, no room spell), not the Temple of
        # Silvermere or the Newhaven Arena (1:2150, the busiest spawn room): the 15 steps after it, X's silent meditation among them, no
        # longer stand in a Temple. Every step keeps its send text, so its comparison stays.
        steps = sc["steps"]
        gotos = [i for i, x in enumerate(steps) if x.get("send") == "sysop goto nowhere"]
        assert len(gotos) == 1
        nowhere = steps.pop(gotos[0])
        nowhere["note"] += " (T-096: played in the start room, not in Lost City)"
        helps = [i for i, x in enumerate(steps) if x.get("send") == "sysop"
                 and "help list" in x.get("note", "")]
        assert len(helps) == 1
        steps.insert(helps[0] + 1, nowhere)
        back = [i for i, x in enumerate(steps) if x.get("send") == "sysop goto silvermere"
                and x.get("note") == "back to town"]
        assert len(back) == 1
        steps[back[0]].update(
            send="sysop goto support",
            note="T-096: back to town's quiet room, the Sysop Support Chamber (1:164): "
                 "no monster group, no room spell")
        # T-104: Rhudaur's dark monks attack a character who is not evil (m4 run 39039053836:
        # three attacked the sysop on arrival, on the original only), and Silvermere's guardsmen
        # attack an evil one. GOD <user> UNLAWFUL sets lawful 0 and evil to at least 40, NEUTRAL
        # sets evil 0: evil just before the Rhudaur GOTO, neutral again
        # in the Support Chamber right after leaving it. Same steps on both servers.
        steps = sc["steps"]
        rh = [i for i, x in enumerate(steps) if x.get("send") == "sysop goto rhudaur"]
        assert len(rh) == 1 and steps[rh[0] + 1]["send"] == "sysop goto khazarad"
        prompt = steps[rh[0]]["until"]
        steps.insert(rh[0] + 1, {"send": "sysop goto support", "until": prompt, "timeout": 10,
                                 "note": "T-104: out of Rhudaur into the quiet Sysop Support Chamber "
                                         "(1:164), so no town sees the sysop evil"})
        steps.insert(rh[0] + 2, {"send": "sysop god sysadm neutral", "until": prompt, "timeout": 10,
                                 "note": "T-104: neutral again, in the quiet room, before any GOTO or walk "
                                         "can reach Khazarad, Silvermere or Newhaven (guardsmen attack evil)"})
        steps.insert(rh[0], {"send": "sysop god sysadm unlawful", "until": prompt, "timeout": 10,
                             "note": "T-104: evil (40+) for Rhudaur, whose dark monks attack "
                                     "non-evil arrivals (GOD UNLAWFUL)"})
        wr("scripts/sysop.json", sc)
        o["script"] = "m4/scripts/sysop.json"
        o["note"] = ("T-057: the m4_sysop scenario in the m4 group, its script with an equal "
                     "start after the entry. T-084: up to three reruns after interference (the six "
                     "GOTO towns draw wanderers, and an ambient Temple line can come to one side at "
                     "the exit). T-096: GOTO NOWHERE is played in the start room and the trip back "
                     "ends in the Sysop Support Chamber, not the Temple, so only each GOTO's own step "
                     "is open to wanderers and the Temple's ambient lines")
        o["max_reruns"] = 3
        # the sysop screens byte-compared (T-055); not the GOD STATUS/INFO screens of a
        # character whose sheet the other scenarios change, nor CONFIGURE SHOW
        o["deterministic"] = ["sysop", "sysop goto nowhere", "sysop god", "sysop god sysadm dance",
                              "sysop list", "sysop arena status",
                              "sysop tweak", "sysop frobnicate"]
        o["ignore_steps"] = [1, 2, 3, 4] + list(range(5, 5 + len(setup)))
        o["bytes_from_echo"] = True
        # the SYSOP key's part of the bare SYSOP list: silvermere leaves out INITIALIZE
        # with the sub-command (no caches; catalogue.toml)
        o["divergences"] = [{
            "card": "T-055", "side": "orig",
            "why": "INITIALIZE is dropped with the sub-command: no caches",
            "unknown_lines": [r"^\s*INITIALIZE-- Re-initialize MajorMUD buffers\s*$"],
            "byte_patterns": [r"[^\r\n]*INITIALIZE-- Re-initialize MajorMUD buffers[^\r\n]*\r?\n?"]}, {
            "card": "T-055", "side": "new",
            "why": "LIST's developer line needs developer mode, which the original "
                   "has only on the developers' registration numbers; "
                   "silvermere's DEBUG grant stands for it (docs/differences.md)",
            "unknown_lines": [r"^\s*ITEMS, KNOWN_MONSTERS, ACTIVE_MONSTERS, SPELLS, TEXT, RESTOCK\s*$"],
            "byte_patterns": [r"[^\r\n]*ITEMS, KNOWN_MONSTERS, ACTIVE_MONSTERS, SPELLS, TEXT, RESTOCK[^\r\n]*\r?\n?"]}, {
            "card": "T-055", "side": "orig",
            "why": "CONFIGURE SHOW's three credit lines are not shown in "
                   "silvermere: no BBS credits (docs/differences.md)",
            "unknown_lines": [r"^\s*Normal Credit Consumption:\s+\d+\s*$",
                              r"^\s*Exempt Credit Conusmption:\s+\d+\s*$",
                              r"^\s*Free time for exempt users:\s+\d+\s*$"],
            "byte_patterns": [r"[^\r\n]*(?:Normal Credit Consumption|Exempt Credit Conusmption|"
                              r"Free time for exempt users):\s+\d+[^\r\n]*\r?\n?"]}, {
            "card": "T-055", "side": "new",
            "why": "CONFIGURE SHOW's developer pair needs developer mode, which the "
                   "original has only on the developers' registration numbers; "
                   "silvermere's DEBUG grant stands for it (docs/differences.md)",
            "unknown_lines": [r"^\s*Monster Generation Rate:\s+\d+ seconds\s*$",
                              r"^\s*Minutes before regenerating:\s+\d+ minutes\s*$"],
            "byte_patterns": [r"[^\r\n]*Monster Generation Rate:\s+\d+ seconds[^\r\n]*\r?\n?",
                              r"[^\r\n]*Minutes before regenerating:\s+\d+ minutes[^\r\n]*\r?\n?"]}, {
            "card": "T-055", "side": "orig",
            "why": "the MajorMUD Sysop menu's U (the importer CLI), B and E (the kernel's editors) "
                   "and A (licensing) lines are not shown in silvermere (docs/differences.md, "
                   "the sysop menu rows)",
            "unknown_lines": [r"^\s*(?:U - Apply an update|B - Edit the sysop bulletin file|"
                              r"E - Edit the wccmmud\.ini file \(only if it is small\)|"
                              r"A - Addon module activation codes)\s*$"],
            "byte_patterns": [r"[^\r\n]*(?:U - Apply an update|B - Edit the sysop bulletin file|"
                              r"E - Edit the wccmmud\.ini file \(only if it is small\)|"
                              r"A - Addon module activation codes)[^\r\n]*\r?\n?"]}]
        # GOD INFO's play time: the minutes Sysadm spent in the realm over the earlier
        # scenarios (sesstime/2 + lifemins), the two servers' timing
        o["value_masks"] = [{"event": "settings_line", "where": {"key": "Life for this CHAR"},
                             "fields": ["val"]}]
        # CONFIGURE SHOW's pollrate, initrate and Logfile: the labels exist on both sides, the
        # values do not (no record caches or log file here; docs/differences.md)
        o["unknown_value_masks"] = [r"^(Current pollrate:\s+)\S+\s*$",
                                    r"^(Current initrate:\s+)\S+\s*$",
                                    r"^(Logfile:\s+)\S+\s*$"]
        o.pop("mask_prompt", None)
    if name == "quest":
        # T-054's script with each in-realm step waiting for its own echo and a settle: a
        # wandering monster's attack prompts on one side must not shift the later steps
        qs = rd(os.path.join(SC, o["script"].replace("/", os.sep)))
        qs["scenario"] = "m4/quest"
        qs["echo_anchored"] = True
        qs["settle"] = SETTLE
        # T-084: no Arena detour (the closed door, `open`, `exits` and `search` steps are
        # m1/room-walk's): the Arena is the only room on the walk that spawns wanderers
        assert [x.get("send") for x in qs["steps"][10:20]] == [
            "w", "w", "d", "n", "open n", "exits", "search", "u", "e", "e"]
        del qs["steps"][10:20]
        qs["description"] += " T-084: the walk no longer goes down to the Arena (1:2150) for the closed door, `open`, `exits` and `search` steps (m1/room-walk has them): the Arena is type 2 with mongroup 6, a lone player in it gets a monster on 90% of the generation cycles, and run 38823959731's kobold thief (montype 6) came from there. Every room left on the walk has mongroup 0 (the Pier 2): monsters keep to rooms of their own group and a mongroup-0 room generates only montype-0 monsters (the stock ones are stationary, a dummy or an NPC), apart from Angels and Quest monsters."
        wr("scripts/quest.json", qs)
        o["script"] = "m4/scripts/quest.json"
        o["bytes_from_echo"] = True
        o["note"] = 'T-057: the m4-quest scenario in the m4 group, its script echo-anchored and settled. T-084: no Arena detour (the only room on the walk that spawns wanderers), up to two reruns after interference, and a restore that checks its end state'
        # a rerun after interference starts Aldric where the first attempt left him: at the
        # Pier (1:1813, no hang-up room), with the hit points a wanderer took. Before it each
        # side rests him to his full 35 (the m1 sheet, as every first attempt shows) and walks
        # him back to the script's start, the Narrow Road (1:2146), two rooms west of the
        # Village Entrance: Pier n Boat Launch (1814), n Silvermere Docks (2342), w North of
        # Silvermere (2363), n Newhaven Docks (2149; a level exit, levels 0-3), n Forest Path
        # (2148), nw Village Entrance (2140), w Narrow Path (2143), w Narrow Road (2146)
        assert [x.get("send") for x in qs["steps"][5:7]] == ["e", "e"]
        assert qs["steps"][-4]["send"] == "x"
        walk = [st(d, until=P, timeout=10) for d in ["n", "n", "w", "n", "n", "nw", "w"]]
        restore = dict(qs, scenario="m4/quest-restore",
                       description="Before m4/quest's rerun: Aldric rested to full hit points and walked from the Pier back to the Narrow Road (1:2146), by the Boat Launch (1814), the Silver River (2342, 2363; the Level exit north to the Newhaven Docks has no minimum, exitpar1s 0), the Docks (2149), the Forest Path (2148), the Village Entrance (2140) and the Narrow Path (2143). T-084: it checks what it restores. The rest waits for `You are now resting.` and then the full-HP prompt, both after its own echo (a prompt from before the command cannot satisfy it); `st` must then show Hits 35/35; and the last step needs the Narrow Road with the full-HP prompt. A restore that does not restore times out and fails the scenario.",
                       steps=qs["steps"][:5]
                       + [st("rest", 'rest to the full 35, after its own echo and "You are now resting."', until='You are now resting\\.(?s:.*)\\[HP=35\\]:', timeout=900),
                          {"send": "st", "until": 'Hits:\\s+35/35(?s:.*)\\[HP=35\\]:', "timeout": 10,
                           "note": 'full hit points, from the status screen (the prompt alone could be a stale one)'}]
                       + walk
                       + [st("w", 'the Narrow Road, where the script starts, at full hit points',
                             until='Narrow Road(?s:.*)\\[HP=35\\]:', timeout=10, required=True)]
                       + qs["steps"][-4:])
        wr("scripts/quest-restore.json", restore)
        o["rerun_restore"] = "m4/scripts/quest-restore.json"
        o["max_reruns"] = 2
    if name == "cleanup":
        o["note"] += (" Statistical (the job's timing): it runs only with --include-statistical,"
                      " compared as events; m4_stats.py has no check of its own for it.")
    wr(f"{name}.json", o)
print("ok")

# ---------------------------------------------------------------- silvermere: Sysadm
os.makedirs(os.path.join(M4, "setup"), exist_ok=True)
base = rd(os.path.join(SC, "m1/setup/new-aldric.json"))
steps = [{"op": "connect", "until": "Username:", "timeout": 20},
         {"send": "sysop", "until": "Password:"},
         {"send": "sysop", "until": "Make your selection"},
         {"send": "M", "until": r"\[MAJORMUD\]:"}]
i = next(k for k, s in enumerate(base["steps"]) if s.get("send") == "E")
for s in base["steps"][i:]:
    s = dict(s)
    if s.get("send") == "" and s.get("until", "").startswith("Family"):
        # the offered given name is the user id, "sysop", which the editor refuses (SYSOP is
        # a game command): the original's sysop character is Sysadm
        s["send"] = "Sysadm"
    elif s.get("send") == "Stonebrook":
        s["send"] = "Operator"
    steps.append(s)
wr("setup/new-sysadm.json", {
    "scenario": "m4/setup/new-sysadm",
    "naws": [79, 24],
    "description": "T-057, silvermere only: the character of the BBS sysop account (sysop/sysop, "
                   "created with `silvermere account create sysop --sysop` and granted DEBUG), a "
                   "Human Warrior with the given name the editor offers and the family name "
                   "Operator, through the line-mode editor, then a clean logoff. Played once per "
                   "instance: `differential.py replay --script m4/setup/new-sysadm.json --to "
                   "127.0.0.1:2325` (tools/differential/m4_live.sh does it).",
    "character": {"user": "sysop", "password": "sysop", "name": "Sysadm Operator",
                  "race": "Human", "class": "Warrior", "level": 1},
    "steps": steps})
# the original: the same character through the full-screen editor (creation-fsd's keystrokes)
cr = rd(os.path.join(root, "tests/captures/creation-fsd/script.json"))
k = next(j for j, x in enumerate(cr["steps"]) if x.get("note", "").startswith("accept the given name"))
# the field holds the offered "sysop" (refused: SYSOP is a game command):
# rub it out and type Sysadm before the Enter that leaves the field
cr["steps"].insert(k, {"send": "\b" * 5 + "Sysadm", "note": "the given name Sysadm"})
cr["steps"][k + 1]["note"] = "leave the given name"
for st_ in cr["steps"]:
    v = st_.get("send")
    if v in ("Aldric", "aldric1"):
        st_["send"] = "sysop"
    elif v == "Stonebrook":
        st_["send"] = "Operator"
cr["scenario"] = "m4/setup/orig-sysadm"
cr["description"] = ("T-057, original only: the character of the BBS sysop account (sysop/sysop, made "
                     "by MBBSEmu's -DBRESET sysop), a Human Warrior named Sysadm Operator, with creation-fsd's keystrokes (Strength "
                     "and Health 50), then a clean logoff. Played once per instance: "
                     "`differential.py replay --script m4/setup/orig-sysadm.json --to 127.0.0.1:2323`.")
cr["character"] = {"user": "sysop", "password": "sysop", "name": "Sysadm Operator",
                   "race": "Human", "class": "Warrior", "level": 1}
wr("setup/orig-sysadm.json", cr)
for name in os.listdir(M4):
    p = os.path.join(M4, name)
    if name.endswith(".json") and name not in ("quest.json", "hangup.json", "cleanup.json"):
        o = rd(p)
        # Sysadm is made once per silvermere instance (docs/testing.md): a setup entry
        # would run again on every new harness process and find the character there
        o["setup"] = []
        wr(name, o)
print("sysadm ok")
