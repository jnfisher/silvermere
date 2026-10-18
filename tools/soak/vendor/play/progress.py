# Copied from the project's MBBSEmu test harness; edit here (see vendor/SOURCE.md).
#!/usr/bin/env python3
"""
General-purpose MajorMUD progression bot.  Plays through the bridge (so it shows in the viewer)
and uses the game data (mmud_data) for routes, trainers and hunting grounds.

  progress.py --class-num 2 --minutes 60 [--start 1:2150] [--stats str,hea,agi] ...

Loop:
  * know where we are (room names checked against the data after every move)
  * when experience reaches the next level: walk to the nearest trainer for our class and level,
    TRAIN, then spend character points on the chosen stats
  * otherwise pick the best hunting ground for our hit points (exp per monster HP, safe maximum
    hit, close by), walk there and roam it: fight what's there, loot coins, rest when hurt
  * re-pick the hunting ground after each level
"""
import argparse
import collections
import json
import os
import random
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mmud_data as md  # noqa: E402
import party  # noqa: E402
import patterns  # noqa: E402
from bridge import call  # noqa: E402

COINS = ("runic", "platinum", "gold", "silver", "copper")
HP_RE = re.compile(r"\[HP=(-?\d+)")
MA_RE = re.compile(r"\[HP=-?\d+/(?:MA|KAI)=(\d+)")
NOMANA_RE = r"(?i)You do not have enough (mana|kai)"
ATTACKED_RE = re.compile(r"(?i)\b(at you|you!|attacks you|lunges at you|\*Combat Engaged\*)")
# any blow aimed at us: every mon_*_you text says "at you" or "you ... for N damage!"
BLOW_RE = (r"(?i)\b(at (?:\w+ )?you\b|you!|attacks you|\*Combat Engaged\*"
           r"|you\b[^\n!.]{0,40}? for \d+ damage!)")
COMBAT_END_RE = re.compile(r"(?i)\*Combat Off\*")
DARK_RE = r"(?i)too dark|pitch black|very dark|can't see anything|cannot see anything"


class Died(Exception):
    pass


class Bot:
    def __init__(self, world, key, class_num, log=print):
        self.w = world
        self.key = key
        self.cands = set(world.rooms)  # unknown until the first look
        self.class_num = class_num
        self.hp = self.max_hp = 1
        self.level = 1
        self.exp = self.need = 0
        self.names = {r.name for r in world.rooms.values()}
        self.reg = patterns.Registry.load(world=world)  # screen pattern registry
        # wake on any monster arrival line (449 texts from the game data, e.g. "A giant rat
        # creeps into the room from nowhere."), so we strike first, or on any blow aimed at us.
        # The per-monster blow texts would make this ~40x slower, and the bridge searches it
        # every 0.1 s, so blows use the generic BLOW_RE instead.
        alts = sorted({re.sub(r"\(\?P<\w+>", "(?:", p.rx.pattern) for p in self.reg.patterns
                       if p.name in self.ARRIVALS or p.role in self.ARRIVALS})
        self.wake_re = BLOW_RE + "|" + "|".join(f"(?:{a})" for a in alts)
        self.events = []
        self.remotes = None  # party.Remotes when in a party
        self.party = None  # party.LeaderRole when leading: tick() runs between fights and rests
        self.follower = False  # following a party leader: don't back off from groups alone
        self.on_events = None  # party.FollowerRole.on_events: sees every read's events
        self.melee_only = False  # never pick attack spells (casting ends autocombat)
        self.resting = False
        self.log = log
        self.kills = 0
        self.coins = collections.Counter()
        self.engaged = False  # in auto-combat (never re-send an attack then: it resets the swing timer)
        self.skip = set()  # names we can't or shouldn't fight
        self.avoid = set()  # names we had to flee from
        self.nobuy = set()  # items a purchase failed on
        self.can_goto = True  # turned off on the first refusal
        self.gear_snapshot = (None, {})
        self.pending_pile = None
        self.saving_for = 0  # price of the upgrade we're saving for (0 = nothing planned)
        self.light_item = None  # name of the light source we carry
        self.mana = self.max_mana = 0
        self.book = []  # md.Spell objects in our spellbook
        self.lit = False
        self.bynames = collections.defaultdict(list)
        for m in world.monsters.values():
            self.bynames[m.name.lower()].append(m)

    # ---- bridge -------------------------------------------------------------------------
    def send(self, text, wait=1.0, until=None, timeout=10.0):
        self.last_sent = text
        r = call("send", text=text, raw=False, wait=wait, until=until, timeout=timeout)
        return self._seen(r)

    def read(self, wait=1.0, until=None, timeout=10.0):
        return self._seen(call("read", wait=wait, until=until, timeout=timeout))

    def _seen(self, r):
        if not r["alive"]:
            raise SystemExit("disconnected")
        t = r["text"]
        if r.get("skipped") and (self.remotes or self.on_events):
            # text that arrived between commands (bridge.send skips it in the reply): a party
            # telepath or invitation can be there
            skipped = self.reg.classify(r["skipped"])
            if self.remotes:
                self.remotes.feed(skipped)
            if self.on_events:
                self.on_events(skipped)
        # stage 1 of the pattern migration: prompt and death
        # come from the registry's events
        self.events = self.reg.classify(t, sent=[getattr(self, "last_sent", "")])
        if self.remotes:
            self.remotes.feed(self.events)  # party @-messages (party.py)
        if self.on_events:
            self.on_events(self.events)  # party role sees every screen (rest, fights, walks)
        prompts = [e for e in self.events if e.name == "prompt"]
        if prompts:
            pf = prompts[-1].fields
            if pf.get("hp") is not None:
                self.hp = int(pf["hp"])
            if pf.get("mana") is not None:
                self.mana = int(pf["mana"])
            self.resting = pf.get("state") == "Resting"
        # shadow check against the old regexes until the migration is done
        old = HP_RE.findall(t)
        if old and prompts and prompts[-1].fields.get("hp") is not None \
           and int(old[-1]) != int(prompts[-1].fields["hp"]):
            self.shadow("hp", old[-1], prompts[-1].fields["hp"], t)
        # auto-combat state: the last of combat_engaged / combat_off wins
        for e in self.events:
            if e.name == "combat_engaged":
                self.engaged = True
            elif e.name == "combat_off":
                self.engaged = False
        names = {e.name for e in self.events}
        me = getattr(self, "firstname", None)
        down = "you_killed" in names or any(
            e.name == "drops_unconscious" and me and e.fields.get("who") == me for e in self.events)
        if down or (prompts and self.hp <= 0):
            raise Died(t[-300:])
        return t

    # ---- stage 2: combat events --------------------------------------------------------------
    ON_ME = {"mon_hits_you", "mon_miss_you", "mon_dodge_you", "mon_deflect_you", "attacks_you"}
    MY_ACTIONS = {"you_hit", "you_miss", "you_dodged", "you_glance", "spell_cast_you", "cast_fail",
                  "cast_resisted", "no_effect_on"}
    ARRIVALS = {"monster_arrives", "monster_arrives_generic", "monster_follows", "monster_follows_any"}
    CANT_ATTACK = {"no_effect_weapon", "wrong_weapon", "target_missing"}
    GOOD_TARGET = {"evil_warning", "too_evil", "lawful_block", "guilt"}
    MOVE_REFUSED = {"exit_level_high", "exit_level_low", "not_permitted", "exit_forbidden",
                    "exit_alignment", "need_spell", "need_item", "toll_short", "exit_timed",
                    "exit_purchase"}

    def ev(self, text):
        """Registry events for a block of text."""
        return self.reg.classify(text, sent=[getattr(self, "last_sent", "")])

    def attacked_in(self, text):
        """Is something attacking us in this text? (No false hits on '... creeps in after you!'
        or friendly heals; catches every 'X <verb>s you for N damage!'.)"""
        return any(e.name in self.ON_ME for e in self.ev(text))

    def arrivals_in(self, text):
        """Monsters that just came into our room (or followed us in) in this text."""
        return [e.fields.get("mon") or "something" for e in self.ev(text)
                if e.name in self.ARRIVALS]

    def hits_on_me(self, text):
        return [int(e.fields.get("dmg") or 0) for e in self.ev(text) if e.name == "mon_hits_you"]

    def shadow(self, what, old, new, text):
        """Log a disagreement between the old regex and the registry (migration safety net)."""
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs",
                            "shadow" + (f"-{os.environ['BRIDGE_SESSION']}"
                                        if os.environ.get("BRIDGE_SESSION") else "") + ".log")
        with open(path, "a") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {what}: old={old!r} new={new!r} "
                    f"text={text[-160:]!r}\n")

    def note(self, text):
        call("note", text=text)
        self.log(text)

    def ensure_statline(self):
        """The bot needs statline FULL (the default): the HP prompt plus the room redisplayed on
        Enter.  Players can change it with SET STATLINE."""
        self.send("set", wait=1.2)
        modes = [e.fields.get("mode", "") for e in self.events if e.name == "settings_statline"]
        if modes and modes[-1].strip().lower() != "full":
            self.note(f"Statline is '{modes[-1].strip()}': switching to FULL")
            self.send("set statline full", wait=1.2)

    def ensure_maxpages(self, n=30):
        """Accept more telepaths per 3-s medium cycle than the default 3, so party messages
        aren't refused.  Saved with the character."""
        self.send(f"set maxpages {n}", wait=1.0)

    def ensure_quiet(self):
        """QUIET (= BRIEF): short room descriptions, which only shortens what we read; the
        title and 'Obvious exits' are unchanged.  Saved with the character."""
        self.send("quiet", wait=1.2)

    def enter_realm(self, user=None, password=None):
        """From wherever the session is (BBS login, BBS menu, MajorMUD menu) into the realm."""
        t = self.send("", wait=1.5)
        for _ in range(12):
            if HP_RE.search(t.splitlines()[-1] if t.strip() else ""):
                return True
            if re.search(r"Username:\s*$", t) and user:
                self.send(user, wait=1.0)
                t = self.send(password, wait=2.0)
            elif re.search(r"Make your selection", t):
                t = self.send("M", wait=1.5)
            elif re.search(r"\[MAJORMUD\]:\s*$", t):
                t = self.send("E", wait=2.5)
            elif re.search(r"(?i)press enter|more\?|\(N\)onstop|continue", t):
                t = self.send("", wait=1.5)
            else:
                t = self.send("", wait=1.5)
        return bool(HP_RE.search(t))

    # ---- reading the screen -------------------------------------------------------------
    def room_name(self, text):
        """The last room title in the text: the known room name nearest above 'Obvious exits'."""
        lines = [l.strip() for l in text.replace("\r", "").split("\n")]
        name = None
        for i, l in enumerate(lines):
            if l.startswith("Obvious exits"):
                for j in range(i - 1, max(-1, i - 8), -1):
                    cand = re.sub(r"^\[HP=\d+\]:\s*", "", lines[j])
                    if cand in self.names:
                        name = cand
                        break
        return name

    def observe(self, text):
        """(room name, set of visible exit directions) of the last room shown, or (None, None)."""
        lines = [l.strip() for l in text.replace("\r", "").split("\n")]
        found = (None, None)
        for i, l in enumerate(lines):
            if l.startswith("Obvious exits"):
                ex = l.split(":", 1)[1] if ":" in l else ""
                j = i + 1
                while j < len(lines) and lines[j] and not lines[j].startswith("[HP"):
                    ex += " " + lines[j]; j += 1  # wrapped exit lists
                dirs = {d for d in re.findall(r"[a-z]+", ex.lower()) if d in md.DIRS}
                for k in range(i - 1, max(-1, i - 8), -1):
                    cand = re.sub(r"^\[HP=\d+\]:\s*", "", lines[k])
                    if cand in self.names:
                        found = (cand, dirs)
                        break
        return found

    def fits(self, key, name, dirs):
        r = self.w.rooms.get(key)
        if not r or r.name != name:
            return False
        if dirs is None:
            return True
        vis = {md.DIRS[d] for d, k, t in r.links() if t != "Hidden"}
        return vis == dirs

    def update(self, text, cmd=None):
        """Narrow the set of rooms we might be in, after a move (cmd) or a look."""
        name, dirs = self.observe(text)
        if not name:
            # the title can be overwritten by noise ("You hear movement..."); unless the move
            # was refused, assume it happened and follow the exit
            if cmd is not None and "Obvious exits" in text and \
               not re.search(r"(?i)no exit|can't go|cannot go|is closed", text):
                nxt = set()
                for c in self.cands:
                    r = self.w.rooms.get(c)
                    for d, k, t in (r.links() if r else []):
                        if self.w.exit_command(r, d) == cmd:
                            nxt.add(k)
                if nxt:
                    self.cands = nxt
                    if self.key not in nxt:
                        self.key = min(nxt)
            return
        if cmd is not None:
            nxt = set()
            for c in self.cands:
                r = self.w.rooms.get(c)
                if not r:
                    continue
                for d, k, t in r.links():
                    if self.w.exit_command(r, d) == cmd:
                        nxt.add(k)
            cands = {k for k in nxt if self.fits(k, name, dirs)}
        else:
            cands = {k for k in self.cands if self.fits(k, name, dirs)}
        if not cands:
            # lost: look near where we thought we were first, then anywhere
            near = set()
            for c in list(self.cands)[:20]:
                near |= set(self.w.distances(c, 12))
            cands = {k for k in near if self.fits(k, name, dirs)} or \
                    {k for k in near if self.w.rooms[k].name == name} or \
                    {r.key for r in self.w.rooms.values() if self.fits(r.key, name, dirs)} or \
                    {r.key for r in self.w.rooms.values() if r.name == name}
        self.cands = cands
        if self.key not in cands:
            self.key = min(cands)
        if len(cands) > 1:
            self.log(f"(position ambiguous: {len(cands)} rooms named '{name}')")

    @staticmethod
    def here(text):
        m = re.findall(r"Also here: (.*?)\.\s*\n", text, re.S)
        if not m:
            return []
        names = [n.strip() for n in m[-1].replace("\n", " ").split(",")]
        return [n for n in names if n and n[0].islower()]

    def known(self, name):
        """Known-monster records for a name as shown (prefixes like 'angry' or 'big' dropped)."""
        words = name.lower().split()
        for i in range(len(words)):
            ms = self.bynames.get(" ".join(words[i:]))
            if ms:
                return ms
        return []

    EVIL = {1, 2, 5, 6}  # Evil, Chaotic Evil, Neutral Evil, Lawful Evil (md.ALIGNMENTS)

    def monsters(self, text):
        """Monsters here worth fighting: evil-aligned (good ones cost alignment, neutral ones
        can't be attacked), not NPCs, not exp-less, not marked immune.  Unknown names only if
        they are attacking us."""
        out = []
        attacked = self.attacked_in(text or "")
        for n in self.here(text):
            if n in self.skip:
                continue
            ms = self.known(n)
            if not ms:
                if attacked:
                    out.append(n)
                continue
            if all(m.alignment in self.EVIL and m.montype != 2 and m.exp > 0 for m in ms):
                if self.winnable(n) or attacked:
                    out.append(n)
        return out

    @staticmethod
    def target_word(name):
        return name.split()[-1]

    # ---- status ---------------------------------------------------------------------------
    def status(self):
        t = self.send("st", wait=1.0)
        m = re.search(r"Hits:\s+(\d+)/(\d+)", t)
        if m:
            self.hp, self.max_hp = int(m.group(1)), int(m.group(2))
        m = re.search(r"Mana:\s*\*?\s*(\d+)/(\d+)", t)
        if m:
            self.mana, self.max_mana = int(m.group(1)), int(m.group(2))
        m = re.search(r"Armour Class:\s+(\d+)/(\d+)", t)
        if m:
            self.ac, self.dr = int(m.group(1)), int(m.group(2))
        m = re.search(r"Level:\s+(\d+)", t)
        if m:
            self.level = int(m.group(1))
            self.w.char_level = self.level  # Level exits (e.g. no Newhaven above 3)
            self.update_danger()
        stats = dict(re.findall(r"(Strength|Intellect|Willpower|Agility|Health|Charm):\s+(\d+)", t))
        self.stats = {k: int(v) for k, v in stats.items()}
        self.str = self.stats.get("Strength", 0)
        m = re.search(r"Class:\s+(\S+)", t)
        if m and m.group(1) in self.w.classes:
            self.class_num = self.w.classes[m.group(1)]
        m = re.search(r"Race:\s+(\S+)", t)
        if m:
            self.race = m.group(1)
        m = re.search(r"Lives/CP:\s+\d+/(\d+)", t)
        if m:
            self.cp = int(m.group(1))
        m = re.search(r"Name:\s+(\S+)", t)
        if m:
            self.firstname = m.group(1)
        t = self.send("exp", wait=1.0)
        m = re.search(r"Exp: (\d+) Level: (\d+) Exp needed for next level: (\d+) \((\d+)\)", t)
        if m:
            self.exp, self.level = int(m.group(1)), int(m.group(2))
            self.need = int(m.group(4))
        return self

    def can_level(self):
        return self.need and self.exp >= self.need

    # ---- moving -------------------------------------------------------------------------------
    def where(self):
        """Look (Enter) and make sure self.key matches the room we are in."""
        t = self.send("", wait=1.0)
        if re.search(DARK_RE, t):
            self.check_light(t)
            t = self.send("", wait=1.0)
        self.update(t)
        return t

    def sync(self, name):
        if not name or self.w.rooms[self.key].name == name:
            return
        # a neighbour with that name, else the only room with that name
        near = [k for _, k, _ in self.w.rooms[self.key].links()
                if k in self.w.rooms and self.w.rooms[k].name == name]
        cands = near or [r.key for r in self.w.rooms.values() if r.name == name]
        if cands:
            self.key = cands[0]
            if len(cands) > 1 and not near:
                self.log(f"(ambiguous room '{name}', assuming {self.key})")

    def step(self, cmd, expect):
        if self.party:
            before = self.key
            self.party.hold_for_missing()  # reuniting with a lost member comes first
            if self.key != before:
                return False  # it walked us elsewhere: the caller replans from here
        t = self.send(cmd, wait=1.0)
        refused = [e for e in self.events if e.name in self.MOVE_REFUSED]
        if refused:
            room = self.w.rooms.get(self.key)
            for d, k, typ in (room.links() if room else []):
                if self.w.exit_command(room, d) == cmd:
                    self.w.blocked.add((self.key, d))
            self.note(f"Exit refused ({refused[0].name}): routing around it")
            return False
        if any(e.name == "no_exit" for e in self.events):
            # the data has this exit, so we aren't where we think: look and re-place ourselves
            # (otherwise a patrol replans the same impossible move forever)
            self.note(f"No exit '{cmd}' from {self.w.rooms[self.key].name} {self.key}: "
                      "not where I thought, looking")
            self.where()
            return False
        if re.search(r"(?i)(door|gate) is closed", t) and not self.observe(t)[0] \
           and cmd in md.DIRCMD:
            t = self.open_way(cmd) or t
        self.update(t, cmd)
        if expect in self.cands:
            self.key = expect
        lit = self.check_light(t)
        if lit and re.search(DARK_RE, t):
            t = self.send("", wait=1.0)  # now we can see: look again to place ourselves
            self.update(t)
            if expect in self.cands:
                self.key = expect
        t += lit
        if self.attacked_in(t) or self.monsters(t):
            self.fight_all(t)
        return self.key == expect

    # SYSOP GOTO destinations (cmd_sysop); needs the SYSOP/GAMEOP key.  Only
    # these named towns are used: room-number teleports would make the game trivial (John).
    TOWNS = {"newhaven": (1, 2150), "silvermere": (1, 2189), "rhudaur": (2, 2519),
             "khazarad": (6, 1249), "lostcity": (16, 426)}

    def try_goto(self, dst, route_len):
        """Jump to a town with SYSOP GOTO when that plus the walk from there is clearly shorter."""
        if not self.can_goto:
            return False
        best = None
        for town, tk in self.TOWNS.items():
            if tk not in self.w.rooms or not self.allowed(tk):
                continue
            r = self.w.route(tk, dst)
            if r is not None and (best is None or len(r) < best[1]):
                best = (town, len(r), tk)
        if not best or best[1] + 1 >= route_len - 5:
            return False
        town, _, tk = best
        t = self.send(f"sysop goto {town}", wait=1.5)
        if re.search(r'You say "sysop', t):
            self.can_goto = False  # no key on this account
            self.note("SYSOP GOTO not available on this account")
            return False
        if re.search(r"(?i)may not|autocombat|retaliation", t):
            return False
        self.cands, self.key = {tk}, tk
        self.where()
        self.note(f"SYSOP GOTO {town} (saves {route_len - 1 - best[1]} steps)")
        return True

    def open_way(self, cmd):
        """A closed door/gate: OPEN it; if it's locked, BASH it (success when a roll of 0-100
        is below Str + exit bonus); then walk through.  Returns the move output."""
        d = md.DIRS[md.DIRCMD.index(cmd)]
        t = self.send(f"open {d}", wait=1.0)
        if re.search(r"(?i)locked", t):
            for _ in range(6):
                t = self.send(f"bash {d}", wait=1.5)
                if re.search(r"(?i)bashed the \w+ open|already open|bash the door open", t):
                    break
                if re.search(r"(?i)must wait", t):
                    time.sleep(2)
            else:
                self.note(f"Couldn't bash the way {d} open")
                return None
            self.note(f"Bashed the way {d} open")
        return self.send(cmd, wait=1.0)

    def walk_to(self, dst, why=""):
        for attempt in range(5):
            if dst in self.cands:
                self.key = dst  # indistinguishable from where we are: treat as arrived
                return True
            if attempt == 0:
                first = self.w.route(self.key, dst)
                if first is not None and len(first) > 8:
                    self.try_goto(dst, len(first))
            route = self.w.route(self.key, dst)
            if route is None:
                self.note(f"No walking route from {self.key} to {dst}")
                return False
            if not route:
                return True
            self.note(f"Walking {len(route)} steps to {self.w.rooms[dst].name}{' ' + why if why else ''}")
            ok = True
            for cmd, nxt in route:
                if not self.step(cmd, nxt):
                    ok = False
                    break
                self.rest_if_needed()
            if ok and (self.key == dst or dst in self.cands):
                self.key = dst
                return True
            self.where()
        return self.key == dst or dst in self.cands

    # ---- fighting ------------------------------------------------------------------------------
    def fight_all(self, text=""):
        """Fight until no monsters are left here (or we have to run)."""
        for _ in range(20):
            mons = self.monsters(text) if text else []
            if not mons:
                text = self.send("", wait=0.8)
                mons = self.monsters(text)
                if not mons and not self.attacked_in(text):
                    return
                if not mons:
                    text = self.read(wait=1.5)
                    continue
            if self.party and self.party.flee_now():
                self.flee(chasers=mons)  # a follower asked: the party leaves together
                return
            if not self.winnable(mons[0]) and self.hp < self.max_hp:
                self.note(f"{mons[0]} looks too costly at HP {self.hp}/{self.max_hp}: leaving")
                if self.leave_fight():
                    return
            crowd = self.too_many(mons) and not self.follower  # a party shares the hits
            if self.hp < self.max_hp * self.flee_frac() or crowd:
                if crowd:
                    self.note(f"{len(mons)} hostiles here and HP {self.hp}/{self.max_hp}: backing off")
                if self.leave_fight():
                    return
            if self.engaged:
                t = ""  # already fighting (e.g. it attacked us): don't restart the swing cycle
            else:
                t = self.send(self.attack_cmd(mons[0]), wait=1.0)
            if "need a weapon to bash" in t:  # no weapon (lost it): fists
                self.gear_snapshot = (self.key, {n: sl for n, sl in self.gear_snapshot[1].items()
                                                 if not (self.item_named(n) and self.item_named(n).maxdmg)})
                self.last_mode = "a"
                t = self.send(f"a {self.target_word(mons[0])}", wait=1.0)
            names = {e.name for e in self.ev(t)}
            if names & (self.CANT_ATTACK | self.GOOD_TARGET):
                self.skip.add(mons[0])  # immune, gone, or good-aligned: leave it alone
                text = ""
                continue
            # follow the fight while auto-combat runs; check HP at every status line
            idle = 0
            while self.engaged or (not t and idle == 0):
                if any(e.name == "no_mana" for e in self.ev(t[-300:])):
                    t = t[:-300] + re.sub(NOMANA_RE, "", t[-300:])
                    melee = [o for o in self.attack_options(mons[0]) if not o[1].startswith("spell")]
                    self.last_mode = melee[0][1]
                    if not self.engaged:
                        t += self.send(melee[0][2], wait=1.0)
                if self.party and self.party.flee_now():
                    self.flee(chasers=mons)  # a follower asked: the party leaves together
                    return
                if self.hp <= self.flee_line(mons, t):
                    if self.leave_fight(chasers=mons):
                        self.avoid.add(mons[0])  # it beat our estimate: stay away from it
                        return
                more = self.read(wait=0.3, until=r"\[HP=-?\d+|\*Combat Off\*", timeout=3.0)
                idle = idle + 1 if not more.strip() else 0
                t += more
                if idle >= 4 and not self.attacked_in(t[-400:]):
                    break  # nothing happening for ~12 s: treat the fight as over
            t += self.read(wait=1.0)
            self.record_attack(getattr(self, "last_mode", "a"), t)
            self.kills += sum(1 for e in self.ev(t) if e.name == "exp_gain")
            self.loot(t)
            text = ""

    def leave_fight(self, chasers=()):
        """Flee; or, as a party follower, ask the leader to (John: followers don't flee on
        their own, the leader takes the party out).  True if we left."""
        if self.follower and self.party:
            self.party.request_flee()
            return False
        self.flee(chasers=chasers)
        return True

    def flee_line(self, mons, fight_text=""):
        """HP at which to leave: the class flee fraction, or 2x the hardest hit seen this fight /
        the opponents' known max hit, whichever is higher (a dark monk kicked for 26: 30% of 85
        HP was too late)."""
        seen = max(self.hits_on_me(fight_text), default=0)
        known = max((self.hit_after_dr(m) for n in mons for m in self.known(n)), default=0)
        return max(self.max_hp * self.flee_frac(), 2 * max(seen, known))

    def flee_frac(self):
        """Casters (low HP, hit hard relative to it) leave a fight at 50%, melee at 30%."""
        return 0.5 if self.max_mana else 0.3

    # ---- fight evaluation ------------------------------------------------------------------------
    def our_dpr(self):
        """Rough damage we deal per round: best affordable attack spell (cast success ~0.72),
        else our weapon (BASH x3.3 for classes that have it) at ~60% hits."""
        sp = self.best_attack_spell()
        if sp and self.mana >= sp.mana:
            return sp.damage(self.level) * 0.72
        wpn = None
        for name, slot in self.gear_snapshot[1].items():
            it = self.item_named(name)
            if it and it.maxdmg:
                wpn = it
        avg = (wpn.mindmg + wpn.maxdmg) / 2 if wpn else 3
        bash = 3.3 if 31 in self.w.class_abils.get(self.class_num, {}) else 1.0
        return avg * bash * 0.6

    def monster_dpr(self, m):
        """Expected damage a monster deals us per round: its attack table (cumulative % per
        attack type, min-max damage), ~60% of swings landing, less our DR per hit."""
        exp_hit, exp_energy, last = 0.0, 0.0, 0
        for typ, pct, lo, hi, en in zip(m.atktypes, m.atkpcts, m.atkmins, m.atkmaxs,
                                        m.atkenergies):
            if not typ or pct <= last:
                continue
            share = (pct - last) / 100
            last = pct
            exp_hit += share * max(0.0, (lo + hi) / 2 - getattr(self, "dr", 0))
            exp_energy += share * max(1, en)
        # a monster swings as many times a round as its energy pays for (the dark monk landed
        # 3-4 blows in one round on Kasimir)
        swings = max(1.0, m.energy / exp_energy) if exp_energy else 1.0
        return exp_hit * 0.6 * swings

    def fight_cost(self, name, hp_now=None):
        """Expected HP we lose killing this monster (None if unknown)."""
        ms = self.known(name)
        if not ms:
            return None
        m = max(ms, key=lambda x: x.maxhp)
        best = max((o[0] for o in self.attack_options(name)), default=self.our_dpr())
        rounds = m.maxhp / max(0.5, best)
        return rounds * self.monster_dpr(m)

    def winnable(self, name, full=False):
        """Can we kill it before reaching our flee line (with a 25% margin)?"""
        if name in self.avoid:
            return False
        cost = self.fight_cost(name)
        if cost is None:
            return True
        hp = self.max_hp if full else self.hp
        # fight tougher things and rely on fleeing when it goes south (John): engage when the
        # expected cost is within RISK x the HP we have above the flee line
        return cost < (hp - self.max_hp * self.flee_frac()) * self.RISK

    def too_many(self, mons):
        """Would this group's expected hits in one round take more than half our HP?"""
        expected = sum(max((self.hit_after_dr(m) for m in self.known(n)), default=10) * 0.6
                       for n in mons)
        return len(mons) > 1 and expected > self.hp * 0.5

    # ---- spells ---------------------------------------------------------------------------------
    def read_spellbook(self):
        """Parse SPELLS: 'Level Mana Short Spell Name' rows -> self.book (md.Spell)."""
        t = self.send("spells", wait=1.2)
        mag = self.w.class_magery.get(self.class_num, (0, 0))[0]
        book = []
        for lvl, mana, short, name in re.findall(r"^\s*(\d+)\s+(\d+)\s+(\S+)\s+(.+?)\s*$", t, re.M):
            cands = [sp for sp in self.w.spells.values()
                     if sp.name.lower() == name.lower() and sp.short == short.lower()]
            cands.sort(key=lambda sp: (sp.magery != mag, sp.num))
            if cands:
                book.append(cands[0])
        self.book = book
        return book

    def best_attack_spell(self, area=False):
        usable = [sp for sp in self.book if sp.attack and sp.area == area and sp.mana <= self.mana]
        if not usable:
            return None
        # most damage per cast; ties to the cheaper one
        return max(usable, key=lambda sp: (sp.damage(self.level), -sp.mana))

    def heal_spell(self):
        hs = [sp for sp in self.book if sp.heal and sp.mana <= self.mana]
        return max(hs, key=lambda sp: sp.damage(self.level)) if hs else None

    ATTACKS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs",
                           "attack" + (f"-{os.environ['BRIDGE_SESSION']}"
                                       if os.environ.get("BRIDGE_SESSION") else "") + ".v2.json")

    def attack_stats(self):
        if not hasattr(self, "_astats"):
            try:
                with open(self.ATTACKS) as f:
                    self._astats = json.load(f)
            except (OSError, ValueError):
                self._astats = {}
        return self._astats

    def record_attack(self, mode, text):
        """Learn from a fight: damage we dealt per action of ours (hits, misses, dodged, glancing,
        spell casts and failures), per attack mode."""
        acts = [e for e in self.ev(text) if e.name in self.MY_ACTIONS]
        if not acts:
            return
        dmg = sum(int(e.fields.get("dmg") or 0) for e in acts if e.name in ("you_hit", "spell_cast_you"))
        st = self.attack_stats().setdefault(mode, {"damage": 0, "actions": 0})
        st["damage"] += dmg
        st["actions"] += len(acts)
        try:
            with open(self.ATTACKS, "w") as f:
                json.dump(self._astats, f, indent=1)
        except OSError:
            pass

    def attack_options(self, name):
        """[(expected damage per action, mode, command)] for this target.  Model first; once a
        mode has 15+ measured actions, its measured average (scaled for this monster's armour
        or magic resistance relative to typical) takes over."""
        ms = self.known(name)
        m = max(ms, key=lambda x: x.maxhp) if ms else None
        tw = self.target_word(name)
        opts = []
        # melee: weapon average, ~60% hits, less monster DR per hit; BASH x1.1 then x3 (DR
        # before the x3), -15 to-hit (~-15% hits), no crits
        wpn = None
        for n, slot in self.gear_snapshot[1].items():
            it = self.item_named(n)
            if it and it.maxdmg:
                wpn = it
        avg = (wpn.mindmg + wpn.maxdmg) / 2 if wpn else 3
        dr = m.dr if m else 0
        ac_pen = min(0.3, (m.ac if m else 0) / 200)
        opts.append((max(0.5, avg - dr) * (0.6 - ac_pen), "a", f"a {tw}"))
        if 31 in self.w.class_abils.get(self.class_num, {}) and wpn:  # BASH needs a weapon
            opts.append((max(0.5, avg * 1.1 - dr) * 3 * (0.45 - ac_pen), "bash", f"bash {tw}"))
        # spells: average x ~72% cast success x magic resistance cut
        reserve = 0
        hs = [sp for sp in self.book if sp.heal]
        if hs:
            reserve = 2 * min(sp.mana for sp in hs)  # keep mana for two heals
        mr = (m.magicres if m and hasattr(m, "magicres") else 0)
        for sp in ([] if self.melee_only else self.book):
            if sp.attack and not sp.area and sp.mana + reserve <= self.mana:
                opts.append((sp.damage(self.level) * 0.72 * (1 - min(0.75, mr / 200)),
                             f"spell:{sp.short}", f"{sp.short} {tw}"))
        st = self.attack_stats()
        out = []
        for est, mode, cmd in opts:
            seen = st.get(mode)
            if seen and seen["actions"] >= 15:
                est = seen["damage"] / seen["actions"]
            out.append((est, mode, cmd))
        return sorted(out, reverse=True)

    def attack_cmd(self, name):
        """Pick the attack with the best expected damage per action; now and then try an
        under-sampled option so the measurements can correct the model."""
        opts = self.attack_options(name)
        st = self.attack_stats()
        best = opts[0][0]
        under = [o for o in opts if st.get(o[1], {}).get("actions", 0) < 15 and o[0] >= best * 0.5]
        choice = opts[0]
        if under and random.random() < 0.10:
            choice = random.choice(under)
        self.last_mode = choice[1]
        return choice[2]

    def self_heal(self):
        """Out of combat: cast a heal on ourselves while hurt and we have the mana."""
        for _ in range(6):
            sp = self.heal_spell()
            if not sp or self.hp >= self.max_hp * 0.85:
                return
            self.send(sp.short, wait=1.2)

    def attack_verb(self, name):
        """BASH (x1.1 then x3 damage, -15 to-hit, no crits, double energy capped at a round)
        unless the monster's armour is high for our level."""
        if 31 not in self.w.class_abils.get(self.class_num, {}):
            return "a"
        ac = max((m.ac for m in self.known(name)), default=0)
        return "bash" if ac <= 30 + 2 * max(0, self.level - 1) else "a"

    def loot(self, text):
        """Pick up the coins that dropped (coins_drop events), counting what we got."""
        for c in dict.fromkeys(e.fields.get("coin") for e in self.ev(text) if e.name == "coins_drop"):
            if not c:
                continue
            got = self.send(f"get {c.split()[0]}", wait=0.8)
            for e in self.events:
                if e.name == "picked_up_coins":
                    n = e.fields.get("n") or e.fields.get("count")
                    if n:
                        self.coins[c.split()[0]] += int(n)

    def wait_for_rebirth(self):
        """Unconscious (HP <= 0) until DEATHHP, then reborn elsewhere: wait it out, relocate.
        DEATHHP is -200 on our server (WCCMMUD.MSG), so this can take minutes."""
        for _ in range(60):
            try:
                self.read(wait=5.0, timeout=10)
            except Died:
                self.answer_while_down()
                continue
            if self.hp > 0:
                break
        self.cands = set(self.w.rooms)
        try:
            self.where()
            self.status()
        except Died:
            self.wait_for_rebirth()

    def answer_while_down(self):
        """Mortally wounded players may still telepath: tell party members who
        ask after us that we're down, and where (Brynja went silent at -107 HP, 2026-10-13)."""
        r = self.remotes
        if not r:
            return

        def send(text):
            try:
                return self.send(text, wait=1.0)
            except Died as e:  # every screen says HP <= 0 now
                return str(e)
        real, r.send_fn = r.send_fn, send
        try:
            while True:
                m = r.take()
                if not m:
                    break
                if m.word in ("where", "health", "invite", "comeback", "join"):
                    r.tell(m.who, "down", f"{self.key[0]}:{self.key[1]}")
            r.pump()
        finally:
            r.send_fn = real

    def recover_death_pile(self, death_room, worn):
        """After dying: walk back to where we died, pick up what we dropped, put it back on."""
        if not death_room:
            return  # (an empty worn list means: take everything lying there)
        self.note(f"Recovering the death pile at {self.w.rooms[death_room].name}")
        if not self.walk_to(death_room, "to the death pile"):
            return
        self.fight_all()
        t = self.send("", wait=1.0)
        flat = " ".join(t.split())
        m = re.search(r"You notice (.*?) here\.", flat)
        lying = [x.strip() for x in m.group(1).split(",")] if m else []
        for name in list(worn) + [x for x in lying if x not in worn]:
            if name in lying or name in worn:
                g = self.send(f"get {name}", wait=1.0)
                if re.search(r"(?i)you (took|picked up|get)", g) and name in worn:
                    verb = "wield" if worn[name] in ("Two handed", "Weapon Hand", "One handed") else "wear"
                    self.send(f"{verb} {name}", wait=1.0)
        self.equip_carried()  # wear the best of what we picked up

    def retreat(self, maxsteps=15):
        """Walk to the nearest room where nothing spawns (and leave any fight)."""
        dist = self.w.distances(self.key, maxsteps)
        quiet = [k for k, d in sorted(dist.items(), key=lambda kv: kv[1])
                 if not self.w.spawn_pool(self.w.rooms[k]) and self.w.rooms[k].type != 2]
        if quiet:
            self.walk_to(quiet[0], "to a quiet room")

    def flee(self, chasers=()):
        """Leave the fight: prefer exits into rooms where nothing spawns, plain exits before doors;
        confirm we actually moved; if an opponent follows us in, keep running (up to 3 rooms);
        then rest somewhere quiet."""
        chase = {self.known(c)[0].name if self.known(c) else c for c in chasers}
        for hop in range(3):
            room = self.w.rooms[self.key]
            here = self.key
            opts = []
            for d, k, t in room.links():
                if t not in ("Normal", "Door", "Gate", "Block Guard", "Action") or k not in self.w.rooms \
                   or not self.w.exit_ok(room, d):
                    continue
                quiet = not self.w.spawn_pool(self.w.rooms[k])
                opts.append((not quiet, t != "Normal", d, k))
            moved = False
            for _, _, d, k in sorted(opts):
                self.note(f"HP {self.hp}/{self.max_hp}: fleeing {md.DIRS[d]}")
                t = self.send(md.DIRCMD[d], wait=1.0)
                if re.search(r"(?i)(door|gate) is (closed|locked)", t):
                    t = self.open_way(md.DIRCMD[d]) or t
                self.update(t, md.DIRCMD[d])
                if self.key != here and self.observe(t)[0]:
                    moved = True
                    break
            if not moved:
                self.note("Couldn't get out of the fight")
                return
            t = self.read(wait=1.5, timeout=3)
            followed = [e for e in self.ev(t) if e.name in self.ARRIVALS
                        and (e.fields.get("base") in chase or e.fields.get("mon") in chase)]
            if not followed and not self.attacked_in(t):
                break
            self.note("Followed: keep running")
        self.retreat()
        self.rest(0.9)

    def _raw_step(self, cmd, expect):
        t = self.send(cmd, wait=1.0)
        if self.observe(t)[0]:
            self.update(t, cmd)
            if expect in self.cands:
                self.key = expect

    def rest_if_needed(self, below=0.5):
        if self.book:
            self.self_heal()
        low_mana = self.max_mana and self.mana < self.max_mana * min(below, 0.5)
        party_low = self.party is not None and self.party.needs_rest(start=True)
        if self.hp < self.max_hp * below or low_mana or party_low:
            self.rest(0.95)

    def heal_at_healer(self):
        """Walk to the nearest healer (Hospital shop) and BUY HEALING until full or broke."""
        best = None
        for r in self.w.rooms.values():
            sh = self.w.shops.get(r.shop) if r.shop else None
            if sh and sh.type == 5:
                route = self.w.route(self.key, r.key)
                if route is not None and (best is None or len(route) < len(best[1])):
                    best = (r.key, route)
        if not best or len(best[1]) > 80:
            return False
        if not self.walk_to(best[0], "to the healer"):
            return False
        for _ in range(5):
            t = self.send("buy healing", wait=1.5)
            if self.hp >= self.max_hp * 0.95 or not re.search(r"(?i)heal|feel better|bought", t):
                break
        self.note(f"Healer: HP {self.hp}/{self.max_hp}")
        return True

    def rest(self, upto):
        if self.w.spawn_pool(self.w.rooms[self.key]) or self.w.rooms[self.key].type == 2:
            self.retreat()  # resting where monsters spawn just feeds them
        self.note(f"HP {self.hp}/{self.max_hp}" + (f", mana {self.mana}/{self.max_mana}" if self.max_mana else "") + ": resting")
        t = self.send("rest", wait=1.0)
        quiet_rounds = interrupted = 0
        while (self.hp < self.max_hp * upto or
               (self.max_mana and self.mana < self.max_mana * upto) or
               (self.party and self.party.needs_rest(start=False))) and quiet_rounds < 60:
            t = self.read(wait=4.0, until=self.wake_re, timeout=8)
            quiet_rounds += 1
            if self.party:
                self.party.tick()
            came = self.arrivals_in(t)
            if came:
                self.note(f"{came[-1]} arrived while resting")
            if came or self.attacked_in(t) or self.monsters(t):
                interrupted += 1
                self.fight_all(t)
                if interrupted >= 3 and self.hp < self.max_hp * 0.4:
                    self.note("Resting keeps getting interrupted: going to a healer")
                    if self.heal_at_healer():
                        return
                self.send("rest", wait=1.0)
                continue
            # resting continues on its own; only start it again if a prompt shows we stopped
            prompts = re.findall(r"\[HP=[^\]]*\]", t)
            if prompts and "(Resting)" not in prompts[-1]:
                self.send("rest", wait=1.0)

    # ---- light ---------------------------------------------------------------------------------
    DARK = -150

    def check_light(self, text):
        """Light our light source in a dark room (Kangs and others can't see in the dark)."""
        out = ""
        if re.search(r"(?i)burn(s|t)? out|goes out|has gone out|no more charges", text):
            self.note(f"{self.light_item} burnt out")
            self.lit = False
            self.light_item = None
        dark = self.w.rooms[self.key].light < self.DARK or re.search(DARK_RE, text)
        if dark and not self.lit:
            if self.light_item:
                out = self.send(f"light {self.light_item}", wait=1.0)
                self.lit = not re.search(r"(?i)don't have|can't|cannot|no ", out)
                if self.lit:
                    self.note(f"Dark room: lit the {self.light_item}")
            else:
                self.note("Dark room and no light source: need to buy one")
        return out

    def ensure_light(self, wealth):
        """Buy the best light source we can afford if we have none."""
        if self.light_item:
            return
        _, worn = self.inventory()
        flat = " ".join(self.last_inventory.split())
        for name in ("lantern", "torch", "brass lamp"):
            if re.search(rf"\b{name}\b", flat):
                self.light_item = name
                return
        best = None
        dist = self.w.distances(self.key, 150)
        for k, steps in dist.items():
            r = self.w.rooms[k]
            sh = self.w.shops.get(r.shop) if r.shop else None
            if not sh:
                continue
            live = md.World.live_shop(sh.num)
            for i in sh.stocked:
                it = self.w.items.get(i)
                if not it or it.type != 6 or live.get(i, 0) <= 0 or it.name in self.nobuy:
                    continue
                p = self.price(sh, it)
                if p > wealth:
                    continue
                score = (it.charges or 1) / (p + 50) / (1 + steps / 30)
                if best is None or score > best[0]:
                    best = (score, k, it, p)
        if not best:
            self.note("No affordable light source reachable")
            return
        _, k, it, p = best
        self.note(f"Buying a light: {it.name} for {p} copper at {self.w.rooms[k].name}")
        if not self.walk_to(k, "for a light"):
            return
        t = self.send(f"buy {it.name}", wait=1.5)
        if "bought" in t:
            self.light_item = it.name
            self.lit = False
        else:
            self.nobuy.add(it.name)

    # ---- gear -----------------------------------------------------------------------------------
    RATE = [1, 10, 100, 10000, 1000000]  # copper per item currency code: CUR_COPPER .. CUR_RUNIC

    @staticmethod
    def carried_items(text):
        """[(item name, slot or None)] from 'You are carrying a, b (Slot), c ...'."""
        flat = " ".join(text.split())
        m = re.search(r"You are carrying (.*?)(?:You have no keys|You have the following keys|"
                      r"You have \d+ keys?|Wealth:|$)", flat)
        if not m or m.group(1).startswith("Nothing"):
            return []
        out = []
        for part in m.group(1).split(","):
            part = part.strip().rstrip(".")
            if not part:
                continue
            mm = re.match(r"(.*?)\s*\(([A-Z][A-Za-z0-9 _/-]+)\)$", part)
            name, slot = (mm.group(1), mm.group(2)) if mm else (part, None)
            if slot:
                # normalise spellings ("Two-handed", "Two Handed" -> "Two handed")
                slot = re.sub(r"[-_]", " ", slot)
                slot = slot[0].upper() + slot[1:].lower() if slot.lower() == "two handed" else slot
                if slot.startswith("Readied"):
                    slot = "Readied"  # "(Readied/0)": a lit light source held ready
            name = re.sub(r"^(\d+|a|an|some)\s+", "", name)
            out.append((name, slot))
        return out

    def inventory(self):
        """(wealth in copper, {slot: item name}) from the 'i' command."""
        t = self.send("i", wait=1.2)
        self.last_inventory = t
        items = self.carried_items(t)
        if items or re.search(r"You are carrying Nothing", t):
            # (an empty pack clears the snapshot: after losing everything we must not think we
            # still hold a weapon and keep trying to BASH)
            self.gear_snapshot = (self.key, {n: sl for n, sl in items if sl})
        m = re.search(r"Wealth: (\d+) copper", t)
        wealth = int(m.group(1)) if m else 0
        worn = {slot: name for name, slot in self.carried_items(t) if slot}
        return wealth, worn

    def item_named(self, name):
        for it in self.w.items.values():
            if it.name == name:
                return it
        return None

    def price(self, shop, it):
        return it.cost * self.RATE[it.currency] * (100 + shop.markup) // 100 if it.currency < 5 else 10**12

    def armour_score(self, it):
        return (it.ac + 3 * it.dr) if it else 0

    def upgrades(self, wealth, worn, maxsteps=120):
        """[(gain per copper, price, shop room key, item, slot)] we can afford, best first."""
        dist = self.w.distances(self.key, maxsteps)
        current = {slot: self.item_named(n) for slot, n in worn.items()}
        slots = {"Torso": 2, "Legs": 3, "Head": 1, "Hands": 6, "Feet": 7, "Arms": 4}  # (inferred)
        out = []
        two_handed = "Two handed" in worn
        for k, steps in dist.items():
            r = self.w.rooms[k]
            sh = self.w.shops.get(r.shop) if r.shop else None
            if not sh or sh.type not in (0, 1, 2, 10):
                continue
            live = md.World.live_shop(sh.num)  # stock changes as people buy
            for i in sh.stocked:
                it = self.w.items.get(i)
                if live.get(i, 0) <= 0 or (it and it.name in self.nobuy):
                    continue
                if not it or it.strreq > getattr(self, "str", 0) or it.currency >= 5:
                    continue
                if it.name in worn or re.search(rf"\b{re.escape(it.name)}\b", getattr(self, "last_inventory", "")):
                    continue  # already have one
                if any(it.classes) and self.class_num not in it.classes:
                    continue
                if 28 in it.abilities and 51 in self.w.class_abils.get(self.class_num, {}):
                    continue  # Anti-Magic classes can't use Magical items
                if two_handed and not it.maxdmg and re.search(r"(?i)shield|buckler", it.name):
                    continue  # no shield with a two-handed weapon
                p = self.price(sh, it)
                if p > wealth or p < 0:
                    continue  # (wealth here is the planning horizon, not cash on hand)
                if it.maxdmg and re.search(r"bow|sling|dart|arrow|bolt", it.name):
                    continue  # missile weapons need ammunition
                if it.maxdmg:  # weapon: compare average damage with what we wield
                    cur = next((c for c in current.values() if c and c.maxdmg), None)
                    gain = (it.mindmg + it.maxdmg) / 2 - ((cur.mindmg + cur.maxdmg) / 2 if cur else 0)
                    slot = "Weapon"
                else:
                    slot = next((s for s, c in current.items()
                                 if c and c.worn == it.worn and not c.maxdmg), None)
                    if not slot and it.worn not in [c.worn for c in current.values() if c]:
                        slot = f"worn{it.worn}"
                    cur = current.get(slot)
                    gain = self.armour_score(it) - self.armour_score(cur)
                # free gear is always worth taking if it improves anything at all
                if (p == 0 and gain > 0) or \
                   gain >= (self.MIN_WEAPON_GAIN if it.maxdmg else self.MIN_GAIN):
                    out.append((gain, p, k, it, slot, steps))
        return out

    MIN_GAIN = 15         # armour score (AC + 3*DR) gain worth buying
    MIN_WEAPON_GAIN = 1.5  # average weapon damage gain worth buying
    SAVE_HORIZON = 2   # consider items up to this many times our spendable money

    def pick_upgrade(self, spendable, worn):
        """The upgrade to buy now, or None to keep saving for something clearly better."""
        ups = self.upgrades(spendable * self.SAVE_HORIZON, worn)
        if not ups:
            return None
        # weapons: one point of average damage ~ 10 armour points (rough exchange rate), x3 for
        # classes that BASH (x1.1 then x3 damage)
        wmult = 30 if 31 in self.w.class_abils.get(self.class_num, {}) else 10
        val = lambda u: u[0] * (wmult if u[3].maxdmg else 1)
        target = max(ups, key=lambda u: (val(u), -u[1]))
        if target[1] <= spendable:
            return target
        now = [u for u in ups if u[1] <= spendable and val(u) >= 0.7 * val(target)]
        if now:
            return max(now, key=val)
        self.note(f"Saving for {target[3].name} ({target[1]} copper, have {spendable})")
        self.saving_for = target[1]
        return None

    def equip_carried(self):
        """Wear/wield anything in the pack that beats what's equipped (death-pile items, a spare
        weapon) before shopping or dropping things."""
        _, worn = self.inventory()
        items = self.carried_items(self.last_inventory)
        cur_w = next((self.item_named(n) for n, sl in items if sl and self.item_named(n)
                      and self.item_named(n).maxdmg), None)
        worn_by_loc = {}
        for n, sl in items:
            it = self.item_named(n)
            if sl and it and not it.maxdmg:
                worn_by_loc[it.worn] = it
        for n, sl in items:
            if sl:
                continue
            it = self.item_named(n)
            if not it or it.type == 6:  # lights are lit, not wielded
                continue
            if it.maxdmg and it.type == 1:
                if not cur_w or (it.mindmg + it.maxdmg) > (cur_w.mindmg + cur_w.maxdmg):
                    if "now holding" in self.send(f"wield {n}", wait=1.0):
                        cur_w = it
            elif it.ac or it.dr:
                old = worn_by_loc.get(it.worn)
                if not old or self.armour_score(it) > self.armour_score(old):
                    if "now wearing" in self.send(f"wear {n}", wait=1.0):
                        worn_by_loc[it.worn] = it
        self.inventory()

    def learn_spells(self, maxsteps=150):
        """Buy and USE spell scrolls this class can use now and doesn't know yet, cheapest
        first (a scroll's Link to Spell ability, 42, names the spell)."""
        mag, magelvl = self.w.class_magery.get(self.class_num, (0, 0))
        if not mag:
            return
        known = {sp.num for sp in self.book}
        dist = self.w.distances(self.key, maxsteps)
        wealth = self.inventory()[0]
        offers = []
        for k, steps in dist.items():
            r = self.w.rooms[k]
            sh = self.w.shops.get(r.shop) if r.shop else None
            if not sh:
                continue
            live = None
            for i in sh.stocked:
                it = self.w.items.get(i)
                if not it or it.type != 9 or 42 not in it.abilities or it.name in self.nobuy:
                    continue
                sp = self.w.spells.get(it.abilities[42])
                if not sp or sp.num in known or sp.magery != mag or sp.magelvl > magelvl \
                   or sp.level > self.level:
                    continue
                if live is None:
                    live = md.World.live_shop(sh.num)
                p = self.price(sh, it)
                if live.get(i, 0) > 0 and p <= wealth:
                    offers.append((p, steps, k, it, sp))
        for p, steps, k, it, sp in sorted(offers, key=lambda o: (o[4].num in known, o[0], o[1])):
            if sp.num in known or p > self.inventory()[0]:
                continue
            self.note(f"Spell: {sp.name} ({sp.short}, lvl {sp.level}, {sp.mana} mana) for {p} copper")
            if not self.walk_to(k, "to buy a spell"):
                continue
            t = self.send(f"buy {it.name}", wait=1.5)
            if "bought" not in t:
                self.nobuy.add(it.name)
                continue
            t = self.send(f"use {it.name}", wait=1.5)
            if "learn the spell" in t:
                known.add(sp.num)
        self.read_spellbook()

    def shop(self, budget_frac=0.9, max_buys=8):
        """Learn new spells, buy a light source if we have none, then the best affordable
        upgrades."""
        self.equip_carried()
        self.learn_spells()
        self.ensure_light(self.inventory()[0])
        for _ in range(max_buys):
            wealth, worn = self.inventory()
            # keep the next training fee: level * 5 silver, plus up to 100% trainer markup
            reserve = self.level * 5 * 10 * 2
            pick = self.pick_upgrade(int(max(0, wealth - reserve) * budget_frac), worn)
            if not pick:
                break
            _, p, k, it, slot, _ = pick
            self.note(f"Gear: {it.name} for {p} copper (replaces {worn.get(slot, 'nothing')}), shop {self.w.rooms[k].name}")
            if not self.walk_to(k, "to shop"):
                self.nobuy.add(it.name)
                continue
            t = self.send(f"buy {it.name}", wait=1.5)
            if "bought" not in t:
                self.note("Purchase failed: " + " ".join(t.split())[:100])
                self.nobuy.add(it.name)
                continue
            verb = "wield" if it.maxdmg else "wear"
            if slot in worn:
                self.send(f"remove {worn[slot]}", wait=1.0)
            self.send(f"{verb} {it.name}", wait=1.0)
        self.cleanup()

    def carried_unused(self):
        """Names of items we carry but don't wear or wield (coins, keys and the light excluded)."""
        self.inventory()
        flat = " ".join(self.last_inventory.split())
        m = re.search(r"You are carrying (.*?)(?:You have no keys|You have the following keys|Wealth:)", flat)
        if not m:
            return []
        out = []
        for part in m.group(1).split(","):
            part = part.strip().rstrip(".")
            if not part or "(" in part or re.search(r"\b(" + "|".join(COINS) + r")\b", part):
                continue
            name = re.sub(r"^(\d+|a|an|some)\s+", "", part)
            if name == self.light_item:
                continue
            out.append(name)
        return out

    def cleanup(self, keep_value=50):
        """Sell unused gear worth keep_value copper or more where we are, drop the rest."""
        for name in self.carried_unused():
            it = self.item_named(name)
            value = it.cost * self.RATE[it.currency] if it and it.currency < 5 else 0
            if value >= keep_value:
                t = self.send(f"sell {name}", wait=1.0)
                if re.search(r"(?i)you sold|you just sold", t):
                    self.note(f"Sold unused {name}")
                    continue
                if not re.search(r"(?i)not in a shop|won't buy|doesn't buy|not interested", t):
                    continue  # keep it for a shop that buys
                continue
            self.send(f"drop {name}", wait=1.0)
            self.note(f"Dropped unused {name} (worth {value} copper)")

    # ---- training ---------------------------------------------------------------------------------
    def trainers(self):
        out = []
        for r in self.w.rooms.values():
            s = self.w.shops.get(r.shop) if r.shop else None
            if s and s.type == 8 and s.trainclass in (0, self.class_num) \
               and s.minlevel <= self.level <= s.maxlevel:
                out.append(r.key)
        return out

    def train(self, stats):
        best = None
        for k in self.trainers():
            route = self.w.route(self.key, k)
            if route is not None and (best is None or len(route) < len(best[1])):
                best = (k, route)
        if not best:
            self.note("No reachable trainer for this class and level")
            return False
        if not self.walk_to(best[0], "to train"):
            return False
        # look first: walk_to can believe it arrived when our position was wrong (Brynja sent
        # TRAIN ~150 times in the Dungeon Entrance and was killed there, 2026-10-13)
        self.where()
        if self.key != best[0]:
            self.note(f"Not at the trainer ({self.w.rooms[best[0]].name}): not training")
            return False
        t = self.send("train", wait=2.0)
        self.note("TRAIN: " + " ".join(t.split())[:150])
        if "appropriate training room" in t:
            self.note("Refused: not a training room for us; looking again")
            self.where()
            return False
        if self.attacked_in(t) or self.monsters(t):
            self.fight_all(t)
        self.status()
        # TRAIN STATS is always left to a human (John's call; the full-screen editor is also
        # unreliable under MBBSEmu)
        if getattr(self, "cp", 0):
            self.note(f"{self.cp} CP unspent: a human should TRAIN STATS")
        return True

    # ---- hunting grounds -------------------------------------------------------------------------
    def allowed(self, key):
        """Can we enter this room? Arenas refuse players above the room's maxlevel."""
        r = self.w.rooms[key]
        return not (r.type == 2 and r.maxlevel and self.level > r.maxlevel)

    def update_danger(self):
        """Rooms whose spawns could hit us harder than our hunting rule allows: routes avoid
        walking through them (Kasimir was killed crossing the Western Road to reach a far
        hunting ground).  Recomputed when level/HP change."""
        sig = (self.level, self.max_hp, getattr(self, "dr", 0), self.HIT_FRAC,
               round(self.our_dpr()))
        if getattr(self, "_danger_sig", None) == sig:
            return
        self._danger_sig = sig
        lim = self.max_hp * self.HIT_FRAC
        # dangerous = hits harder than we allow AND a fight we'd probably lose at full HP
        # (wilderness dark monks/brigands; not slum half-ogres, which Kasimir kills in 2 rounds)
        dpr = max(1.0, self.our_dpr())
        deadly = {m.num for m in self.w.monsters.values()
                  if m.alignment in self.EVIL and m.exp > 0 and self.hit_after_dr(m) > lim
                  and m.maxhp > dpr * 3}  # hits hard AND takes us more than ~3 rounds to kill
        self.w.danger = {k for k, r in self.w.rooms.items()
                         if any(m.num in deadly for m in self.w.spawn_pool(r))}

    def hit_after_dr(self, m):
        """A monster's largest hit after our armour's damage resistance (subtracted from every
        hit)."""
        return max(0, m.maxhit - getattr(self, "dr", 0))

    RISK = 1.5  # how far past the HP budget a fight may be expected to go (--risk); fleeing covers it
    HIT_FRAC = 0.25  # largest monster hit allowed, as a fraction of our max HP (--hit-frac)

    def safe(self, pool, margin=1.0):
        # no single hit too big; which monsters to actually fight is decided per fight
        # (winnable/fight_ok), so mixed rooms are fine
        return all(self.hit_after_dr(m) <= self.max_hp * self.HIT_FRAC and m.maxhp <= self.max_hp * margin
                   for m in pool)

    def huntable(self, pool):
        """A spawn pool worth hunting: safe overall, and at least one evil, non-NPC monster
        worth exp that we can beat (NPC-only rooms like healers don't count)."""
        return self.safe(pool) and any(m.alignment in self.EVIL and m.montype != 2 and m.exp > 0
                                       and self.fight_ok(m) for m in pool)

    def fight_ok(self, m):
        """At full HP, would a fight with this known monster stay above the flee line?"""
        rounds = m.maxhp / max(0.5, self.our_dpr())
        return rounds * self.monster_dpr(m) < self.max_hp * (1 - self.flee_frac()) * self.RISK

    # measured exp/hour per ground, per session (characters differ)
    GROUNDS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs",
                           "grounds" + (f"-{os.environ['BRIDGE_SESSION']}"
                                        if os.environ.get("BRIDGE_SESSION") else "") + ".json")

    OCCUPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "logs", "occupied")

    def occupy(self, anchor):
        """Record the ground this session hunts, so our other bots stay away from it."""
        os.makedirs(self.OCCUPY, exist_ok=True)
        with open(os.path.join(self.OCCUPY, os.environ.get("BRIDGE_SESSION") or "default"), "w") as f:
            f.write(f"{anchor[0]}:{anchor[1]}\n" if anchor else "")

    def occupied(self, key, radius=3):
        me = os.environ.get("BRIDGE_SESSION") or "default"
        try:
            names = os.listdir(self.OCCUPY)
        except OSError:
            return False
        near = None
        for n in names:
            if n == me:
                continue
            try:
                with open(os.path.join(self.OCCUPY, n)) as f:
                    v = f.read().strip()
            except OSError:
                continue
            if v:
                m, r = map(int, v.split(":"))
                if near is None:
                    near = set(self.w.distances(key, radius))
                if (m, r) in near:
                    return True
        return False

    def ground_stats(self):
        try:
            with open(self.GROUNDS) as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    def record_death(self, anchor):
        st = self.ground_stats()
        k = f"{anchor[0]}:{anchor[1]}"
        e = st.setdefault(k, {"name": self.w.rooms[anchor].name, "exp": 0, "seconds": 0})
        e["death_level"] = self.level
        with open(self.GROUNDS, "w") as f:
            json.dump(st, f, indent=1)

    def died_at(self, key):
        e = self.ground_stats().get(f"{key[0]}:{key[1]}", {})
        return "death_level" in e and self.level < e["death_level"] + 2

    def record_ground(self, anchor, exp, seconds):
        """Add measured experience and time for a hunting ground (by anchor room)."""
        st = self.ground_stats()
        k = f"{anchor[0]}:{anchor[1]}"
        e = st.setdefault(k, {"name": self.w.rooms[anchor].name, "exp": 0, "seconds": 0})
        e["exp"] += exp
        e["seconds"] += seconds
        with open(self.GROUNDS, "w") as f:
            json.dump(st, f, indent=1)

    # ---- progression plan (John: a fixed area per level range, then run its loop) --------------
    def load_plan(self, path=None):
        """plans/<class>.json: stages {levels:[lo,hi], area, anchor 'map:room', radius} or
        {levels, quest}.  Returns the plan or None."""
        if not path:
            name = next((n for n, num in self.w.classes.items() if num == self.class_num), "")
            path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "plans",
                                name.lower() + ".json")
        elif not os.path.isabs(path) and not os.path.exists(path):
            path = os.path.join(os.path.dirname(os.path.abspath(__file__)), path)  # e.g. plans/party-duo.json
        try:
            with open(path) as f:
                self.plan = json.load(f)
        except (OSError, ValueError):
            self.plan = None
        return self.plan

    def plan_ground(self):
        """(anchor, rooms, stage) for our level from the plan, or (None, set(), stage/None).
        A quest stage for our level is reported, and we keep to the last area stage at or below
        our level (the quest is for a human)."""
        if not getattr(self, "plan", None):
            return None, set(), None
        stages = self.plan.get("stages", [])
        # a party hunts at its lowest member's level (John, 2026-10-13)
        lvl = self.party.plan_level() if hasattr(self.party, "plan_level") else self.level
        self.planned_level = lvl  # the patrol ends when the party's level moves off this
        here = [st for st in stages if st["levels"][0] <= lvl <= st["levels"][1]]
        quest = next((st for st in here if "quest" in st), None)
        if quest and getattr(self, "_quest_noted", None) != lvl:
            self._quest_noted = lvl
            self.note(f"Plan: level {lvl} quest stage: {quest['quest']} (for a human)")
        areas = [st for st in here if "anchor" in st] or \
                [st for st in stages if "anchor" in st and st["levels"][1] <= lvl][-1:]
        if not areas:
            return None, set(), None
        st = areas[0]
        anchor = tuple(map(int, st["anchor"].split(":")))
        blocked = anchor not in self.w.rooms or self.w.route(self.key, anchor) is None or \
            not self.allowed(anchor)
        if blocked and lvl < self.level and hasattr(self.party, "send_solo"):
            # a party stage we can't reach (Newhaven refuses level 4+): the low members solo
            # until they reach our own stage, and we plan for the rest of the party
            own = [x for x in stages if "anchor" in x and x["levels"][0] <= self.level <= x["levels"][1]]
            if self.party.send_solo(own[0]["levels"][0] if own else self.level):
                return self.plan_ground()
        if anchor not in self.w.rooms or self.w.route(self.key, anchor) is None:
            self.note(f"Plan area {st['area']} ({st['anchor']}) is unreachable from here")
            return None, set(), st
        return anchor, set(self.w.distances(anchor, st.get("radius", 8))), st

    def hunting_ground(self, radius=8, maxsteps=150):
        """(anchor, set of rooms to roam): the best reachable safe area.  A ground with at
        least 10 minutes of measured play is scored by its measured exp/hour; others by an
        estimate (exp on offer per spawn room, arenas x3 since they respawn without delay)."""
        dist = self.w.distances(self.key, maxsteps)
        measured = {}
        for k, e in self.ground_stats().items():
            if e["seconds"] >= 600:
                m, r = map(int, k.split(":"))
                if (m, r) in dist and self.allowed((m, r)) and not self.occupied((m, r)) and \
                   not self.died_at((m, r)) and not self.w.danger_on_path(self.key, (m, r)) and \
                   self.safe(self.w.spawn_pool(self.w.rooms[(m, r)])):
                    measured[(m, r)] = e["exp"] / e["seconds"] * 3600
        if measured:
            best_m = max(measured, key=measured.get)
            # an unmeasured area must look clearly better on paper to be tried instead
            est_best = self._estimate_best(dist, radius)
            if not est_best or est_best[0] < measured[best_m] * 2:
                self.log(f"(measured grounds: { {self.w.rooms[k].name: round(v) for k, v in measured.items()} })")
                return best_m, set(self.w.distances(best_m, radius))
        est = self._estimate_best(dist, radius)
        return (est[1], est[2]) if est else (None, set())

    def _estimate_best(self, dist, radius):
        best = None
        for k, dsteps in dist.items():
            r = self.w.rooms[k]
            pool = self.w.spawn_pool(r)
            if not pool or not self.huntable(pool) or not self.allowed(k):
                continue
            if self.occupied(k) or self.died_at(k):
                continue  # another of our bots hunts here, or we died here recently
            if k in self.w.danger or self.w.danger_on_path(self.key, k):
                continue  # the only way there goes through rooms that could kill us (more players = more spawns at us)
            ball = self.w.distances(k, radius)
            # score (and later patrol) only the safe spawn rooms; unsafe ones are skipped
            spawn = [b for b in ball if self.huntable(self.w.spawn_pool(self.w.rooms[b]))
                     and self.allowed(b)]
            if not spawn:
                continue
            exp = sum(sum(m.exp for m in self.w.spawn_pool(self.w.rooms[b])) /
                      max(1, len(self.w.spawn_pool(self.w.rooms[b]))) *
                      min(self.w.rooms[b].maxmonsters, 3) *
                      (3 if self.w.rooms[b].type == 2 else 1) for b in spawn)
            # far grounds pay for the walk (and every trip is exposure): halve the value every ~30
            # steps, so a near, proven area beats a slightly richer one 100 steps away
            score = exp / (1 + dsteps / 20) / (2 ** (dsteps / 30))
            if best is None or score > best[0]:
                best = (score, k, set(ball))
        return best

    REGENMINS = 5  # module default regeneration delay (minutes) for rooms without their own

    def patrol(self, rooms, until, deadline):
        """Hunt the area's spawn rooms, always walking to the nearest one whose respawn timer
        (the room's regeneration delay after the last kill) has run out."""
        stops = [k for k in rooms if self.huntable(self.w.spawn_pool(self.w.rooms[k]))
                 and self.allowed(k)]
        if not stops:
            return False
        cleared = {k: 0.0 for k in stops}  # when we last left it empty
        # arenas respawn with no delay after a kill (generate_monster)
        regen = {k: 0 if self.w.rooms[k].type == 2 else
                 60 * (self.w.rooms[k].regendelay or self.REGENMINS) for k in stops}
        self.note(f"Patrol: {len(stops)} spawn rooms; visiting whichever has respawned, nearest first")
        while time.time() < deadline:
            now = time.time()
            dist = self.w.distances(self.key, 40)
            ready = [k for k in stops if now - cleared[k] >= regen[k] and k in dist]
            if ready:
                target = min(ready, key=lambda k: (dist[k], -regen[k]))
            else:
                target = min((k for k in stops if k in dist), key=lambda k: cleared[k] + regen[k],
                             default=None)
                if target is None:
                    return False
                wait = cleared[target] + regen[target] - now
                if wait > 5:
                    self.note(f"Nothing respawned yet; next in {wait:.0f}s at {self.w.rooms[target].name}")
            if target != self.key:
                route = self.w.route(self.key, target, within=rooms) or self.w.route(self.key, target)
                if route is None:
                    cleared[target] = now  # can't get there from here right now
                    continue
                for cmd, nxt in route:
                    self.step(cmd, nxt)
                    if self.key in cleared:
                        cleared[self.key] = time.time()  # passed through: it's empty now
                    self.rest_if_needed(0.8)
                    if until() or time.time() >= deadline:
                        return until()
                    if self.key != nxt:
                        break  # off course: replan from wherever we are
            else:
                # waiting for a respawn here: rest (or just look) until something shows up
                if self.hp < self.max_hp:
                    self.rest(0.95)
                else:
                    t = self.read(wait=5.0, until=self.wake_re, timeout=20)
                    came = self.arrivals_in(t)
                    if came:
                        self.note(f"{came[-1]} arrived: attacking")
                    self.fight_all(t)  # refreshes the room (Enter) and attacks what is here
            self.fight_all()
            if self.key in cleared:
                cleared[self.key] = time.time()
            if until():
                return True
        return False

    def roam(self, rooms, until, deadline):
        visits = collections.Counter()
        while time.time() < deadline:
            self.fight_all()
            self.rest_if_needed(0.8)  # never walk into the next fight hurt
            if until():
                return True
            here = self.w.rooms[self.key]
            opts = [(visits[k], random.random(), d, k) for d, k, t in here.links()
                    if k in rooms and t in ("Normal", "Door", "Block Guard", "Gate")]
            if not opts:
                return False
            _, _, d, k = min(opts)
            visits[k] += 1
            self.step(md.DIRCMD[d], k)
        return False


def single_instance():
    """Refuse to run if another bot already drives the session (two bots fight over it)."""
    import fcntl
    sess = os.environ.get("BRIDGE_SESSION", "")
    f = open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                          f"progress{'-' + sess if sess else ''}.lock"), "w")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        sys.exit("another progress.py is running")
    return f



def progression(bot, w, a, deadline, stop=lambda: False):
    """The solo progression loop: train, shop, pick the plan's area, patrol it.  Ends at the
    deadline or when stop() says so (a follower soloing until it catches up, John 2026-10-13)."""
    current_anchor = None
    startup = True  # starter-kit / light check, done once inside the loop (it may die)
    train_retry_at = 0.0  # after a failed TRAIN, hunt on for a while instead of retrying at once
    bot.note(f"Progress bot: level {bot.level}, exp {bot.exp}/{bot.need}, HP {bot.max_hp}, at {bot.w.rooms[bot.key].name}")
    while time.time() < deadline and not stop():
        # the death pile first: once we've out-levelled what killed us (or when told with --pile)
        if bot.pending_pile and bot.level > bot.pending_pile[2]:
            if bot.hp < bot.max_hp * 0.9:
                bot.rest(0.95)
            room, worn, _ = bot.pending_pile
            bot.pending_pile = None
            try:
                bot.recover_death_pile(room, worn)
            except Died:
                bot.wait_for_rebirth()
        try:
            if bot.can_level() and time.time() >= train_retry_at:
                # shop only right after levelling up (new spells, a bigger budget), not every
                # time a patrol ends
                lvl = bot.level
                bot.train([])
                if bot.level > lvl:
                    bot.shop()
                else:
                    train_retry_at = time.time() + 300
                    bot.note("Training failed: hunting on, trying again in 5 minutes")
                continue
            if startup:
                # once: a brand-new character with no weapon gets its free starter kit (shop()
                # also learns spells and buys a light); otherwise just make sure we have a light
                startup = False
                wealth, worn = bot.inventory()
                if any(bot.item_named(n) and bot.item_named(n).maxdmg for n in worn.values()):
                    bot.ensure_light(wealth)
                else:
                    bot.note("No weapon equipped: shopping for a starter kit")
                    bot.shop()
            anchor, rooms, stage = bot.plan_ground()
            if anchor:
                if getattr(bot, "_stage_area", None) != stage["area"]:
                    bot._stage_area = stage["area"]
                    bot.note(f"Plan: levels {stage['levels'][0]}-{stage['levels'][1]}: {stage['area']}")
            else:
                if bot.plan:
                    bot.note("No usable plan stage for this level: choosing a ground dynamically")
                anchor, rooms = bot.hunting_ground(a.radius)
            current_anchor = anchor
            if not anchor:
                bot.note("No safe hunting ground found: retreating to a quiet room to rest")
                bot.retreat()
                bot.rest(0.95)
                time.sleep(30)
                continue
            bot.occupy(anchor)
            bot.note(f"Hunting ground: {w.rooms[anchor].name} ({anchor[0]}:{anchor[1]}), {len(rooms)} rooms")
            bot.walk_to(anchor, "to hunt")
            bot.status()
            t0, e0 = time.time(), bot.exp
            check = [time.time()]

            def levelled():
                if bot.party:
                    bot.party.tick()
                    if getattr(bot, "planned_level", None) not in (None, bot.party.plan_level()):
                        bot.note(f"Party level now {bot.party.plan_level()}: choosing the area again")
                        return True
                if time.time() - check[0] > 90:
                    check[0] = time.time()
                    bot.status()
                    wealth = bot.inventory()[0]
                    bot.note(f"level {bot.level} exp {bot.exp}/{bot.need} HP {bot.max_hp} "
                             f"kills {bot.kills} wealth {wealth}c")
                return (bot.can_level() and time.time() >= train_retry_at) or stop()
            try:
                bot.patrol(rooms, levelled, deadline)
            finally:
                bot.status()
                bot.record_ground(anchor, max(0, bot.exp - e0), time.time() - t0)
        except Died as e:
            death_room, worn, dlevel = bot.key, dict(bot.gear_snapshot[1]), bot.level
            bot.note("Died: " + " ".join(str(e).split())[-120:])
            if current_anchor:
                bot.record_death(current_anchor)
            bot.wait_for_rebirth()
            if worn:
                bot.pending_pile = (death_room, worn, dlevel)


def main():
    conf = None
    if "--party" in sys.argv:  # parties.json gives the session, password and role
        def opt(name, default=None):
            return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default
        if "--as" not in sys.argv:
            sys.exit("--party needs --as CHARACTER")
        conf = party.load_party(opt("--party"), opt("--as"), opt("--parties"))
        if "--session" not in sys.argv:
            sys.argv += ["--session", conf["member"].get("session", "")]
    if "--session" in sys.argv:  # must be set before the lock and the bridge are used
        i = sys.argv.index("--session")
        import bridge
        bridge.set_session(sys.argv[i + 1])
    _lock = single_instance()  # noqa: F841 (held for the life of the process)
    ap = argparse.ArgumentParser()
    ap.add_argument("--session", default=None, help="bridge session (see bridge.py --session)")
    ap.add_argument("--plan", default=None,
                    help="progression plan JSON (default plans/<class>.json if present)")
    ap.add_argument("--pile", default=None,
                    help="MAP:ROOM of a death pile to recover first (if the bot lost track of it)")
    ap.add_argument("--risk", type=float, default=1.5,
                    help="engage fights expected to cost up to RISK x the HP above the flee line")
    ap.add_argument("--hit-frac", type=float, default=0.25,
                    help="largest monster hit allowed as a fraction of max HP (casters with "
                         "heals or ranged openers can take ~0.33)")
    ap.add_argument("--class-num", type=int, default=0, help="(read from the ST screen)")
    ap.add_argument("--start", default=None, help="MAP:ROOM we are in (else from the screen)")
    ap.add_argument("--minutes", type=float, default=60)
    ap.add_argument("--radius", type=int, default=8)
    ap.add_argument("--user"); ap.add_argument("--password")
    ap.add_argument("--verbose", action="store_true",
                    help="keep full room descriptions (default: switch to QUIET)")
    ap.add_argument("--melee", action="store_true",
                    help="melee only: never pick attack spells (e.g. a cleric who bashes)")
    ap.add_argument("--party", default=None, help="party name in parties.json (group scripting)")
    ap.add_argument("--as", dest="as_char", default=None, help="which party member this bot plays")
    ap.add_argument("--parties", default=None, help="party config file (default parties.json)")
    a = ap.parse_args()
    if conf:
        a.user = a.user or conf["me"]
        a.password = a.password or conf["member"].get("password")
        if "hit_frac" in conf["member"] and "--hit-frac" not in sys.argv:
            a.hit_frac = conf["member"]["hit_frac"]
        a.melee = a.melee or bool(conf["member"].get("melee"))

    w = md.World()
    start = tuple(map(int, a.start.split(":"))) if a.start else (1, 2150)
    bot = Bot(w, start, a.class_num)
    if conf:  # party messages from the start: a downed bot must still answer (answer_while_down)
        bot.remotes = party.remotes_for(bot, conf)
    Bot.HIT_FRAC = a.hit_frac
    bot.melee_only = a.melee
    Bot.RISK = a.risk
    try:
        entered = bot.enter_realm(a.user, a.password)
        if entered:
            bot.ensure_statline()
            if not a.verbose:
                bot.ensure_quiet()
            bot.ensure_maxpages()
    except Died:
        bot.note("Down at start-up: waiting to be reborn")
        bot.wait_for_rebirth()
        entered = True
    if not entered:
        sys.exit("could not get into the realm")
    try:
        bot.where()
        bot.status()
    except Died:
        bot.note("Down at start-up: waiting to be reborn")
        bot.wait_for_rebirth()
    bot.read_spellbook()
    bot.inventory()
    if bot.load_plan(a.plan or (conf and conf["role"] == "leader" and conf["settings"]["plan"])
                     or None):
        bot.note(f"Following plan {bot.plan.get('class', '?')}: "
                 + ", ".join(f"{st['levels'][0]}-{st['levels'][1]} {st.get('area') or 'quest'}"
                             for st in bot.plan.get("stages", [])))
    if a.pile:
        pm, pr = map(int, a.pile.split(":"))
        bot.pending_pile = ((pm, pr), {}, -1)  # unknown gear list: pick up everything lying there
    deadline = time.time() + a.minutes * 60
    if conf and conf["role"] == "follower":
        role = party.FollowerRole(bot, conf)
        while role.run(deadline, died=Died) == "solo":
            # the leader can't come where we need to hunt: play solo until we catch up
            role.leave_for_solo()
            target = role.solo_level

            def caught_up():
                role.solo_inbox()
                return bot.level >= (role.solo_level or target)
            progression(bot, w, a, deadline, stop=caught_up)
            role.solo_level = None
            bot.note(f"Party: level {bot.level}: rejoining {role.leader}")
            bot.remotes.tell(role.leader, "level", bot.level)
        bot.note(f"Follower done: level {bot.level} exp {bot.exp}/{bot.need}")
        return
    if conf:
        bot.party = party.LeaderRole(bot, conf)
        bot.party.form()
    progression(bot, w, a, deadline)
    bot.status()
    bot.note(f"Progress bot done: level {bot.level} exp {bot.exp}/{bot.need} kills {bot.kills} coins {dict(bot.coins)}")


if __name__ == "__main__":
    main()
