# Copied from the project's MBBSEmu test harness; edit here (see vendor/SOURCE.md).
"""
Party messaging for group scripting (stage G0).

MegaMud-style remote commands carried by telepaths: a line "/Name @word args" to send, "<Name>
telepaths: @word args" to receive.  Plus our request layer, so a follower can ask and the leader
can answer:

    @req <id> <verb> [args]    ask (resent until answered, at most 3 tries)
    @ack <id> | @nak <id> <reason> | @done <id>

Game facts this relies on: the sender sees "--- Telepath Sent to
Name ---" or "--- Telepath Not Sent ---"; the target's page allowance (default 3, raise it with
"set maxpages N") resets every 3-s medium cycle; anyone online can telepath anyone, so only
messages from trusted names are acted on.

Remotes knows nothing about the bridge: it is given a send(text) -> response function and a
clock, and fed registry events, so it can be tested offline.
"""
import collections
import json
import os
import re
import time

ALLOWANCE_WINDOW = 3.0   # the game's medium cycle (3 s)
PER_WINDOW = 3           # our own cap per target per window (the game lets 4 through by default)
RETRY_AFTER = 3.0        # a refused telepath: try again after one medium cycle
MAX_TRIES = 3            # then the target counts as unreachable
REQ_TIMEOUT = 10.0       # a request with no @ack/@nak: resend
REQ_TRIES = 3
REPLIES = ("ack", "nak", "done")

Msg = collections.namedtuple("Msg", "who word args raw")


class Remotes:
    def __init__(self, me, trusted, send, clock=time.time, log=print):
        self.me = me
        self.trusted = {t.lower() for t in trusted}
        self.send_fn = send          # send(text) -> the game's response text
        self.clock = clock
        self.log = log
        self.sent_at = collections.defaultdict(collections.deque)  # target -> send times
        self.queue = collections.deque()      # [to, text, tries, not_before]
        self.unreachable = set()
        self.next_id = 1
        self.pending = {}            # our request id -> {to, verb, args, tries, at}
        self.answers = {}            # our request id -> ("ack" | "nak" | "done", args)
        self.seen_reqs = set()       # (sender, id) we already handled
        self.inbox = collections.deque()  # Msgs for the role to act on
        self.untrusted = []          # (who, msg) logged and ignored

    # ---- receiving -------------------------------------------------------------------------
    def feed(self, events):
        """Take registry events from any screen read; queue trusted @-messages, settle replies."""
        for e in events:
            if e.name != "telepath":
                continue
            who = (e.fields.get("who") or "").strip()
            msg = (e.fields.get("msg") or "").strip()
            if not msg.startswith("@"):
                continue
            if who.lower() not in self.trusted:
                self.untrusted.append((who, msg))
                self.log(f"(ignored telepath from {who}: {msg})")
                continue
            parts = msg[1:].split()
            if not parts:
                continue
            m = Msg(who, parts[0].lower(), parts[1:], msg)
            if m.word in REPLIES and m.args and m.args[0].isdigit():
                rid = int(m.args[0])
                if rid in self.pending:
                    self.answers[rid] = (m.word, m.args[1:])
                    if m.word in ("nak", "done"):
                        del self.pending[rid]
                    else:
                        self.pending[rid]["acked"] = True
                continue
            if m.word == "req" and m.args and m.args[0].isdigit():
                key = (who.lower(), int(m.args[0]))
                if key in self.seen_reqs:
                    continue  # a resend of something we already have
                self.seen_reqs.add(key)
            self.inbox.append(m)

    def take(self):
        """The next trusted message for the role to handle, or None."""
        return self.inbox.popleft() if self.inbox else None

    # ---- sending ---------------------------------------------------------------------------
    def tell(self, to, word, *args):
        """Queue '@word args' for `to`.  Sent now if the rate limit allows (see pump)."""
        text = " ".join(["@" + word] + [str(a) for a in args])
        self.queue.append([to, text, 0, 0.0])
        self.pump()

    def request(self, to, verb, *args):
        """Ask `to` for something; returns the id.  Watch answers[id]."""
        rid = self.next_id
        self.next_id += 1
        self.pending[rid] = {"to": to, "verb": verb, "args": list(args), "tries": 1,
                             "at": self.clock(), "acked": False}
        self.tell(to, "req", rid, verb, *args)
        return rid

    def reply(self, msg, word, *args):
        """Answer a @req Msg with ack, nak or done."""
        self.tell(msg.who, word, msg.args[0], *args)

    def _room(self, to):
        q = self.sent_at[to.lower()]
        now = self.clock()
        while q and now - q[0] >= ALLOWANCE_WINDOW:
            q.popleft()
        return len(q) < PER_WINDOW

    def pump(self):
        """Send what the rate limit allows, retry refused telepaths, resend unanswered requests.
        Call it often (every bot loop)."""
        now = self.clock()
        for rid, p in list(self.pending.items()):
            if not p["acked"] and now - p["at"] >= REQ_TIMEOUT:
                if p["tries"] >= REQ_TRIES:
                    del self.pending[rid]
                    self.answers[rid] = ("timeout", [])
                    self.log(f"(request {rid} {p['verb']} to {p['to']}: no answer)")
                    continue
                p["tries"] += 1
                p["at"] = now
                self.queue.append([p["to"], " ".join(["@req", str(rid), p["verb"]] + p["args"]),
                                   0, 0.0])
        later = collections.deque()
        while self.queue:
            item = self.queue.popleft()
            to, text, tries, not_before = item
            if to.lower() in self.unreachable:
                continue  # dropped: see reachable()
            if now < not_before or not self._room(to):
                later.append(item)
                continue
            self.sent_at[to.lower()].append(now)
            out = self.send_fn(f"/{to} {text}") or ""
            if "Telepath Sent to" in out:
                continue
            tries += 1
            if tries >= MAX_TRIES or "Cannot find user" in out:
                self.unreachable.add(to.lower())
                self.log(f"({to} unreachable: '{text}' not delivered)")
                continue
            later.append([to, text, tries, now + RETRY_AFTER])
        self.queue = later

    def reachable(self, name):
        """Forget that `name` was unreachable (e.g. they came back online)."""
        self.unreachable.discard(name.lower())


# ---- configuration (parties.json; parties.example.json documents every field) ----------------
DEFAULTS = {"par_every_s": 20, "catch_up_s": 30, "wait_below_hp": 60, "wait_no_longer_than_s": 300,
            "ignore_wait_if_leading": False, "ignore_party_if_following": False, "plan": None}
CONFIG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "parties.json")


def load_party(name, me, path=None):
    """My view of party `name`: {role, leader, me, friends, member (my entry), members, settings}.
    Raises ValueError for an unknown party or a character that isn't in it."""
    with open(path or CONFIG) as f:
        cfg = json.load(f)
    parties = cfg.get("parties", {})
    if name not in parties:
        raise ValueError(f"no party '{name}' in {path or CONFIG} (have: {', '.join(parties)})")
    p = parties[name]
    members = p.get("members", {})
    lookup = {k.lower(): k for k in members}
    if me.lower() not in lookup:
        raise ValueError(f"{me} is not a member of party '{name}' ({', '.join(members)})")
    me = lookup[me.lower()]
    leader = p["leader"]
    if leader not in members:
        raise ValueError(f"party '{name}': leader {leader} is not listed in members")
    if len(members) > 6:
        raise ValueError(f"party '{name}': {len(members)} members; the game allows 6")
    return {"name": name, "role": "leader" if me == leader else "follower", "leader": leader,
            "me": me, "friends": [m for m in members if m != me], "member": members[me],
            "members": members, "settings": {**DEFAULTS, **p.get("settings", {})}}


def remotes_for(bot, conf, clock=time.time):
    """The party's Remotes for this bot: a leader trusts its friends, a follower its leader."""
    trusted = conf["friends"] if conf["role"] == "leader" else [conf["leader"]]
    return Remotes(conf["me"], trusted, bot.send, clock=clock, log=bot.log)


RANKS = {"front": "frontrank", "mid": "midrank", "middle": "midrank", "back": "backrank"}


def set_rank(bot, conf):
    """Take the configured battle rank (only once in a party: the leader can't go back rank)."""
    cmd = RANKS.get(str(conf["member"].get("rank", "")).lower())
    if cmd:
        bot.send(cmd, wait=1.0)


def players_here(text):
    """Player names in the last 'Also here:' line (players are capitalised, monsters aren't)."""
    found = re.findall(r"Also here: (.*?)\.\s*(?:\n|$)", text, re.S)
    if not found:
        return []
    names = [n.strip() for n in found[-1].replace("\n", " ").split(",")]
    return [n.split()[0] for n in names if n and n[0].isupper()]


# ---- stage G1: forming and keeping the party ----------------------------------------------
class LeaderRole:
    """Invite the friends, keep the roster from 'par', answer @invite/@comeback/@where/@health.
    tick() is called between fights (from the patrol loop's until())."""

    def __init__(self, bot, conf, clock=time.time):
        self.bot, self.conf, self.clock = bot, conf, clock
        self.s = conf["settings"]
        self.friends = [f for f in conf["friends"]]
        self.bots = [f for f in self.friends if not conf["members"][f].get("human")]
        self.joined = set()          # friends the last 'par' showed as following
        self.invited_at = {}         # friend -> when we last invited them
        self.told_at = {}            # friend -> when we last told them where we are
        self.last_par = 0.0
        self.hp = {}                 # member -> (hp %, flags) from the last 'par'   (G2)
        self.waits = {}              # member -> when their @wait came in
        self.missing_since = {}      # member -> when 'par' first showed them missing
        self.rest_started = None
        self.member_at = {}          # member -> (map, room) from their last @here
        self.reuniting = False       # walking back for a member (don't recurse via step())
        self.flee_asked = None       # (who, when) of the last @flee
        self.member_level = {}       # member -> level from their @level
        self.solo = {}               # member -> level at which they rejoin (soloing until then)
        self.fleeing = False
        if bot.remotes is None:
            bot.remotes = remotes_for(bot, conf, clock)

    def where_text(self):
        k = self.bot.key
        return f"{k[0]}:{k[1]}"

    def invite(self, who):
        """Invite, tell them to follow (@join, in case they miss the invitation line: it
        happened while Isolde was resting), and give them a few seconds to do it."""
        self.invited_at[who] = self.clock()
        t = self.bot.send(f"invite {who}", wait=1.0)
        if "You have invited" not in t:
            return False
        self.bot.remotes.tell(who, "join")
        t = self.bot.read(wait=1.0, until=rf"{re.escape(who)} started to follow you", timeout=3)
        if "started to follow you" in t:
            self.joined.add(who)
            self.missing_since.pop(who, None)
        return True

    def roster(self):
        """Who follows us now, from 'par'."""
        t = self.bot.send("par", wait=1.2)
        rows = [e for e in self.bot.events if e.name == "party_row"]
        now = set()
        for e in rows:
            name = (e.fields.get("name") or e.fields.get("invited") or "").split()
            # the leader's own list includes the leader (seen live 2026-10-13)
            if name and not e.fields.get("invited") and name[0] != self.conf["me"]:
                now.add(name[0])
                self.hp[name[0]] = (int(e.fields.get("hpct") or 100), e.fields.get("flags") or "")
        lost = self.joined - now
        for who in sorted(lost):
            self.bot.note(f"Party: {who} is no longer following")
        for who in sorted(now - self.joined):
            self.bot.note(f"Party: {who} is following")
            if who not in self.member_level:
                self.bot.remotes.tell(who, "level")
        if now and not self.joined:
            set_rank(self.bot, self.conf)
        self.joined = now
        self.last_par = self.clock()
        for f in self.bots:
            if f in now or f in self.solo:
                self.missing_since.pop(f, None)
            else:
                self.missing_since.setdefault(f, self.clock())
        return t

    def form(self):
        """At start-up: invite every friend who is here; tell the others where we are."""
        self.roster()
        self.bot.note(f"Leading party '{self.conf['name']}': {', '.join(self.friends)}")
        self.recruit()

    def recruit(self):
        now = self.clock()
        here = set(players_here(self.bot.send("", wait=0.8)))
        for f in self.friends:
            if f in self.joined or f in self.solo:
                continue
            if f in here and now - self.invited_at.get(f, 0) >= 15:
                self.invite(f)
            elif f not in here and f in self.bots and now - self.told_at.get(f, 0) >= 60:
                self.told_at[f] = now
                self.bot.remotes.tell(f, "here", self.where_text())

    def tick(self):
        r = self.bot.remotes
        r.pump()
        while True:
            m = r.take()
            if not m:
                break
            if m.word in ("invite", "comeback"):
                if m.word == "comeback":
                    self.bot.note(f"Party: {m.who} got separated (@comeback)")
                if not self.invite(m.who):
                    r.tell(m.who, "here", self.where_text())
            elif m.word == "where":
                r.tell(m.who, "here", self.where_text())
            elif m.word == "health":
                r.tell(m.who, "health", f"{self.bot.hp}/{self.bot.max_hp}")
            elif m.word == "level" and m.args and m.args[0].isdigit():
                if self.member_level.get(m.who) != int(m.args[0]):
                    self.bot.note(f"Party: {m.who} is level {m.args[0]}")
                self.member_level[m.who] = int(m.args[0])
                if m.who in self.solo and int(m.args[0]) >= self.solo[m.who]:
                    del self.solo[m.who]
                    self.missing_since[m.who] = self.clock()  # a fresh reunite budget
                    self.bot.note(f"Party: {m.who} caught up (level {m.args[0]}): regrouping")
            elif m.word in ("here", "down") and m.args:
                try:
                    self.member_at[m.who] = tuple(map(int, m.args[0].split(":")))
                except ValueError:
                    pass
            elif m.word == "flee":
                self.flee_asked = (m.who, self.clock())
                self.bot.note(f"Party: {m.who} asks to flee (@flee)")
            elif m.word == "wait":
                if self.s["ignore_wait_if_leading"]:
                    continue
                self.waits.setdefault(m.who, self.clock())
                self.bot.note(f"Party: {m.who} asks to wait (@wait)")
            elif m.word == "ok":
                if self.waits.pop(m.who, None) is not None:
                    self.bot.note(f"Party: {m.who} is ready (@ok)")
            elif m.word == "req":
                r.reply(m, "nak", "not-yet")  # requests come in stage G5
        if self.flee_asked and not self.bot.engaged and not self.fleeing and self.flee_now():
            self.fleeing = True  # outside a fight (fight_all checks flee_now itself)
            try:
                self.bot.flee()
            finally:
                self.fleeing = False
        if self.clock() - self.last_par >= self.s["par_every_s"]:
            self.roster()
            if set(self.friends) - self.joined:
                self.recruit()


    def plan_level(self):
        """The level the party hunts at: its lowest member's (John, 2026-10-13).  Members we
        haven't heard from yet, and members soloing to catch up, don't count."""
        return min([self.bot.level] + [lv for who, lv in self.member_level.items()
                                       if who not in self.solo])

    def send_solo(self, regroup):
        """The lowest member's area is somewhere we can't go (Newhaven refuses level 4+): the
        members below `regroup` play solo until they reach it (John, 2026-10-13)."""
        sent = []
        for who, lv in self.member_level.items():
            if lv < regroup and who not in self.solo:
                self.solo[who] = regroup
                self.missing_since.pop(who, None)
                self.bot.remotes.tell(who, "solo", regroup)
                sent.append(who)
        if sent:
            self.bot.note(f"Party: {', '.join(sent)} can't come where we can: soloing to level {regroup}")
        return bool(sent)

    def flee_now(self):
        """A follower's @flee in the last 15 s: the leader takes the party out at once
        (John, 2026-10-13: followers ask, the leader flees, and that comes first)."""
        if self.flee_asked and self.clock() - self.flee_asked[1] < 15:
            self.bot.note(f"Party: fleeing for {self.flee_asked[0]}")
            self.flee_asked = None
            return True
        self.flee_asked = None
        return False

    # ---- stage G2: holding for the party -------------------------------------------------
    def expire_waits(self):
        limit = self.s["wait_no_longer_than_s"]
        for who, since in list(self.waits.items()):
            if self.clock() - since >= limit:
                del self.waits[who]
                self.bot.note(f"Party: stopped waiting for {who} after {limit}s")

    def needs_rest(self, start):
        """Should the leader (start to / keep) rest for the party?  Start: an @wait, or a member
        below wait_below_hp.  Keep resting: until no @wait is left and everyone is at 90%+,
        but never longer than wait_no_longer_than_s."""
        self.tick()
        if self.clock() - self.last_par >= 10:
            self.roster()
        self.expire_waits()
        hurt = {w: h for w, (h, _) in self.hp.items() if w in self.joined}
        if start:
            low = [w for w, h in hurt.items() if h < self.s["wait_below_hp"]]
            if self.waits or low:
                self.rest_started = self.clock()
                self.bot.note("Party rest: " + ", ".join(
                    [f"{w} @wait" for w in self.waits] + [f"{w} at {hurt[w]}%" for w in low]))
                return True
            return False
        if self.rest_started is None:
            self.rest_started = self.clock()
        if self.clock() - self.rest_started >= self.s["wait_no_longer_than_s"]:
            self.bot.note("Party rest: waited long enough, going on")
            self.waits.clear()
            self.rest_started = None
            return False
        more = bool(self.waits) or any(h < 90 for h in hurt.values())
        if not more:
            self.rest_started = None  # each party rest gets its own time limit
        return more

    def hold_for_missing(self):
        """Before a step: reuniting with a lost member comes first (John, 2026-10-13).  Tell
        them to stay put, ask where they are, walk there and invite them, until they follow
        again or wait_no_longer_than_s runs out.  Then go on without them."""
        if self.reuniting:
            return
        lost = [f for f in self.bots if f not in self.joined and
                self.clock() - self.missing_since.get(f, self.clock()) < self.s["wait_no_longer_than_s"]]
        if not lost:
            return
        self.reuniting = True
        try:
            for f in lost:
                self.reunite(f)
        finally:
            self.reuniting = False

    def reunite(self, f):
        bot, r = self.bot, self.bot.remotes
        bot.note(f"Party: {f} is missing: going back for them")
        r.tell(f, "stay")
        r.tell(f, "where")
        asked = self.clock()
        while f not in self.joined:
            if self.clock() - self.missing_since.get(f, self.clock()) >= self.s["wait_no_longer_than_s"]:
                bot.note(f"Party: couldn't reunite with {f}; going on without them")
                return
            self.tick()
            if f.lower() in r.unreachable:
                bot.note(f"Party: {f} is offline; going on without them")
                return
            at = self.member_at.get(f)
            if at and at != bot.key and at in bot.w.rooms:
                bot.walk_to(at, f"to fetch {f}")
            here = players_here(bot.send("", wait=0.8))
            if f in here and self.clock() - self.invited_at.get(f, 0) >= 10:
                self.invite(f)
            t = bot.read(wait=2.0, until=bot.wake_re + "|started to follow you", timeout=5)
            if bot.attacked_in(t) or bot.monsters(t):
                bot.fight_all(t)
            if "started to follow you" in t or self.clock() - self.last_par >= 5:
                self.roster()
            if f not in self.joined and self.clock() - asked >= 20:
                asked = self.clock()
                r.tell(f, "where")  # they may have moved (e.g. to rest)
        bot.note(f"Party: back together with {f}")


class FollowerRole:
    """Accept the leader's invitation, follow, and rejoin after a drop: ask @invite when the
    leader is in the room, else ask @where and walk there.  When the leader is offline, wait
    in a quiet room (John, 2026-10-13).  Defends itself; assisting comes in stage G3."""

    WAKE = r"|has invited you to follow|You are no longer following|You are now following|telepaths:"

    def __init__(self, bot, conf, clock=time.time):
        self.bot, self.conf, self.clock = bot, conf, clock
        self.leader = conf["leader"]
        self.s = conf["settings"]
        self.following = False
        self.asked_at = 0.0
        self.leader_resting = False  # G2: saw "<leader> stops to rest" and no move since
        self.waiting = False         # we sent @wait and no @ok yet
        self.well_since = None       # when we were last seen healthy enough for @ok
        self.leader_down_until = 0.0  # leader said @down: don't pester until then
        self.stay_until = 0.0        # leader said @stay: it is coming to us
        self.pending_follow = False  # invited (or @join) by the leader: follow at the next tick
        self.pending_comeback = False  # dropped out: tell the leader at the next tick
        self.flee_asked_at = 0.0
        self.solo_level = None       # leader said @solo N: play solo until level N
        if bot.remotes is None:
            bot.remotes = remotes_for(bot, conf, clock)

    def on_events(self, events):
        """Called by the Bot for every screen it reads, wherever it is (resting, fighting,
        walking): note invitations and drops.  No sending here; tick() acts on them."""
        for e in events:
            who = (e.fields.get("who") or "").strip().rstrip(".")
            if e.name == "invited_follow" and who == self.leader:
                self.pending_follow = True
            elif e.name == "following" and who == self.leader:
                self.pending_follow = False
                if not self.following:
                    self.following = True
                    self.just_joined = True
            elif e.name == "unfollowed" and who == self.leader:
                if self.following:
                    self.pending_comeback = True
                self.following = False

    def act(self):
        """Do what on_events noted: follow, set our rank, report a drop."""
        bot = self.bot
        if self.pending_follow and not self.following:
            self.pending_follow = False
            bot.send(f"follow {self.leader}", wait=1.0)
        if getattr(self, "just_joined", False):
            self.just_joined = False
            bot.note(f"Party: following {self.leader}")
            self.stay_until = 0.0
            set_rank(bot, self.conf)
            bot.remotes.tell(self.leader, "level", bot.level)
            self.told_level = bot.level
        if self.following and getattr(self, "told_level", bot.level) != bot.level:
            self.told_level = bot.level  # we trained: the party may move to a new area
            bot.remotes.tell(self.leader, "level", bot.level)
        if self.pending_comeback:
            self.pending_comeback = False
            bot.note(f"Party: no longer following {self.leader}; rejoining")
            bot.remotes.tell(self.leader, "comeback")
            self.asked_at = self.clock()

    def handle(self, text):
        """React to one block of screen text (the events were already seen by on_events)."""
        bot = self.bot
        self.on_events(bot.ev(text))  # idempotent: the Bot may have shown us these already
        self.act()
        for e in bot.ev(text):
            who = (e.fields.get("who") or "").strip()
            if e.name == "follow_leader":
                self.leader_resting = False
                bot.update(text)  # moved with the party: place ourselves from the room shown
            elif e.name == "rest_other" and who == self.leader:
                self.leader_resting = True
                if bot.hp < bot.max_hp or (bot.max_mana and bot.mana < bot.max_mana):
                    bot.send("rest", wait=1.0)  # rest with the leader
            elif e.name == "attacks_other" and who == self.leader:
                self.leader_resting = False
        if bot.attacked_in(text) or (bot.monsters(text) and bot.engaged):
            bot.fight_all(text)

    def inbox(self):
        r = self.bot.remotes
        r.pump()
        while True:
            m = r.take()
            if not m:
                break
            if m.word == "here" and m.args and not self.following and \
               self.clock() >= self.stay_until and self.bot.hp >= self.bot.max_hp * 0.5:
                try:
                    key = tuple(map(int, m.args[0].split(":")))
                except ValueError:
                    continue
                if key in self.bot.w.rooms and key != self.bot.key:
                    self.bot.note(f"Party: {self.leader} is at {self.bot.w.rooms[key].name}: going")
                    self.bot.walk_to(key, f"to rejoin {self.leader}")
                r.tell(self.leader, "invite")
                self.asked_at = self.clock()
            elif m.word == "where":
                r.tell(m.who, "here", f"{self.bot.key[0]}:{self.bot.key[1]}")
            elif m.word == "health":
                r.tell(m.who, "health", f"{self.bot.hp}/{self.bot.max_hp}")
            elif m.word == "level":
                r.tell(m.who, "level", self.bot.level)
            elif m.word == "join" and m.who == self.leader:
                self.pending_follow = True  # MegaMud's @join: follow the sender
            elif m.word == "solo" and m.who == self.leader and m.args and m.args[0].isdigit():
                self.solo_level = int(m.args[0])
            elif m.word == "stay" and m.who == self.leader:
                self.stay_until = self.clock() + self.s["wait_no_longer_than_s"]
                self.bot.note(f"Party: {self.leader} is coming back for us: staying here")
            elif m.word == "down" and m.who == self.leader:
                # the leader is unconscious (HP <= 0, alive until DEATHHP): wait somewhere quiet
                # instead of walking into whatever downed them; ask again later
                self.following = False
                if self.clock() >= self.leader_down_until:
                    self.bot.note(f"Party: {self.leader} is down ({' '.join(m.args)}); waiting")
                    self.bot.retreat()
                self.leader_down_until = self.clock() + 60

    def rejoin(self):
        """Not following: ask to be invited (leader here) or where the leader is."""
        r = self.bot.remotes
        if self.leader.lower() in r.unreachable:
            if self.clock() - self.asked_at >= 60:
                self.asked_at = self.clock()
                self.bot.note(f"Party: {self.leader} is offline; waiting somewhere quiet")
                self.bot.retreat()
                r.reachable(self.leader)  # try again next round
            return
        if self.clock() - self.asked_at < 20 or self.clock() < self.leader_down_until:
            return
        if self.clock() < self.stay_until:
            return  # the leader is on the way; it invites us when it gets here
        self.asked_at = self.clock()
        here = players_here(self.bot.send("", wait=0.8))
        r.tell(self.leader, "invite" if self.leader in here else "where")

    # hooks the Bot calls on whatever plays its party role (see LeaderRole)
    def tick(self):
        """Handle party messages while the bot is busy (resting, fighting)."""
        self.inbox()
        self.act()

    def needs_rest(self, start):
        return False

    def flee_now(self):
        return False  # only the leader flees

    def request_flee(self):
        """In trouble: ask the leader to take the party out (at most every 5 s), keep fighting."""
        if self.clock() - self.flee_asked_at >= 5:
            self.flee_asked_at = self.clock()
            self.bot.remotes.tell(self.leader, "flee")
            self.bot.note(f"Party: HP {self.bot.hp}/{self.bot.max_hp}: asked {self.leader} to flee")

    def hold_for_missing(self):
        pass

    def low(self):
        bot = self.bot
        return bot.hp < bot.max_hp * self.s["wait_below_hp"] / 100 or \
            (bot.max_mana and bot.mana < bot.max_mana * 0.25)

    def pace(self):
        """@wait when hurt (not while the leader already rests: that looped in mudengine), rest,
        and @ok once we have stood healthy for 5 s."""
        bot, r = self.bot, self.bot.remotes
        if not self.following or bot.engaged:
            return
        if bot.hp < bot.max_hp * 0.85 and bot.book:
            bot.self_heal()  # a healer heals itself first (Isolde sat at 4/40 with 10 mana)
        if not self.waiting and self.low() and not self.leader_resting:
            self.waiting = True
            self.well_since = None
            r.tell(self.leader, "wait")
            bot.note(f"Party: HP {bot.hp}/{bot.max_hp}: asked {self.leader} to wait")
            bot.send("rest", wait=1.0)
        elif self.waiting:
            healthy = bot.hp >= bot.max_hp * 0.9 and \
                not (bot.max_mana and bot.mana < bot.max_mana * 0.5)
            if not healthy:
                self.well_since = None
                if not bot.resting:
                    bot.send("rest", wait=1.0)
            elif self.well_since is None:
                self.well_since = self.clock()
            elif self.clock() - self.well_since >= 5:
                self.waiting = False
                r.tell(self.leader, "ok")
                bot.note(f"Party: HP {bot.hp}/{bot.max_hp}: told {self.leader} @ok")

    def solo_inbox(self):
        """While soloing to catch up: still answer the leader's questions."""
        r = self.bot.remotes
        r.pump()
        while True:
            m = r.take()
            if not m:
                break
            if m.word == "where":
                r.tell(m.who, "here", f"{self.bot.key[0]}:{self.bot.key[1]}")
            elif m.word in ("level", "health"):
                r.tell(m.who, m.word, self.bot.level if m.word == "level"
                       else f"{self.bot.hp}/{self.bot.max_hp}")
            elif m.word == "solo" and m.args and m.args[0].isdigit():
                self.solo_level = int(m.args[0])

    def leave_for_solo(self):
        """Stop following (FOLLOW on its own) and drop the follower hooks: the Bot plays solo."""
        bot = self.bot
        if self.following:
            bot.send("follow", wait=1.0)
        self.following = self.waiting = self.leader_resting = False
        bot.party, bot.on_events, bot.follower = None, None, False
        bot.note(f"Party: soloing until level {self.solo_level} ({self.leader} can't go where we hunt)")

    def run(self, deadline, died=Exception):
        """The follower's main loop.  `died` is progress.Died: on death, wait to be reborn
        and rejoin (Isolde's bot crashed on her first death, 2026-10-13)."""
        bot = self.bot
        bot.follower = True
        bot.party = self  # so rest() and other busy loops keep handling our messages
        bot.on_events = self.on_events
        bot.note(f"Following {self.leader} (party '{self.conf['name']}')")
        bot.ensure_light(bot.inventory()[0])  # registers the light we carry (dark rooms)
        self.handle(bot.send("par", wait=1.2))
        while time.time() < deadline:
            try:
                if self.solo_level and bot.level < self.solo_level:
                    return "solo"
                t = bot.read(wait=2.0, until=bot.wake_re + self.WAKE, timeout=15)
                self.handle(t)
                self.inbox()
                self.act()
                if not self.following:
                    self.rejoin()
                else:
                    self.pace()
            except died as e:
                bot.note("Died: " + " ".join(str(e).split())[-120:])
                self.following = self.waiting = self.leader_resting = False
                bot.wait_for_rebirth()
                self.asked_at = 0.0  # rejoin straight away
        return "deadline"
