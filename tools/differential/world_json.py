"""The pattern registry's game-data patterns from a JSON export (T-057).

`patterns.Registry.load_world` (vendor/play/patterns.py) reads the game's raw records
(`mmud_data._rows`, record offsets). The differential harness has no such database; it has
`silvermere import --json DIR` (T-006's shape: one `TABLE.jsonl` or `TABLE.jsonl.gz` per
file). `load(registry, DIR)` adds the same patterns from that export, field by field:

- room titles (WCCMP001 `name`);
- monster texts: arrival, departure, follow (`arrivemsg`), death (`deathmsg`), and per
  attack slot the hit, defence and miss texts (`hitmsgs`, `defmsgs`, `missmsgs`);
- weapon verbs (`hitmsg`, `missmsg`) and item destroy texts (`destroymsg`);
- spell texts (`msgnum`): `spell_cast_you`, `spell_cast_on_you`, `spell_cast_room`, the
  generated families of the pattern registry (209, 332 and 561 rows on the stock data);
- every other WCCMSG text (`wccmsg`).

Roles, categories and priorities are load_world's, so a line classifies the same way as
when the registry is built from the raw records. Stdlib only; dev only.
"""

import gzip
import json
import os
import re


def rows(directory, table):
    """The rows of one exported table as dicts."""
    for name, opener in ((table + ".jsonl.gz", gzip.open), (table + ".jsonl", open)):
        path = os.path.join(directory, name)
        if os.path.isfile(path):
            with opener(path, "rt", encoding="utf-8") as f:
                return [json.loads(line) for line in f if line.strip()]
    raise FileNotFoundError(f"{table}.jsonl(.gz) not in {directory}")


def load(reg, directory, pt):
    """Adds the game-data patterns of the export in `directory` to `reg` (a
    `patterns.Registry` before `build()`); `pt` is the patterns module."""
    reg.room_names = {r["name"] for r in rows(directory, "WCCMP001") if r.get("name")}
    msgs = {m["number"]: m["msgs"] for m in rows(directory, "WCCMSG")}
    seen = {}

    def add_t(text, role, cat, prio, num, mon=None):
        # patterns.Registry.load_world's add_t
        text = (text or "").rstrip()
        if not text or len(text) < 4:
            return
        if role == "monster_death" and mon:
            reg.death_texts[text].append(mon)
        toks = pt._template_tokens(text)
        if not any(len(t) >= 3 for t in toks):
            return
        old = seen.get(text)
        if old is not None and old.prio <= prio:
            return
        try:
            rx = re.compile(pt.template_regex(text, pt.ROLE_ARGS.get(role, ())))
        except re.error:
            return
        p = pt.Pattern(role, cat, rx, text, "C", prio, role=role, msgnum=num)
        p.extra = text
        if old is not None:
            reg.patterns.remove(old)
        seen[text] = p
        reg.add(p)

    for m in rows(directory, "WCCKNMSR"):
        name = m.get("name", "")
        reg.monster_names.add(name.lower())
        arr, dth = m.get("arrivemsg", 0), m.get("deathmsg", 0)
        if arr in msgs:
            a = msgs[arr]
            add_t(a[0], "monster_arrives", "movement", 55, arr)
            add_t(a[1], "monster_leaves", "movement", 55, arr)
            add_t(a[2], "monster_follows", "movement", 55, arr)
        if dth in msgs:
            add_t(msgs[dth][2], "monster_death", "combat", 55, dth, name)
        for t, h, df, ms in zip(m.get("atktypes", []), m.get("hitmsgs", []),
                                m.get("defmsgs", []), m.get("missmsgs", [])):
            if not t:
                continue
            if h in msgs:
                add_t(msgs[h][0], "mon_hits_you", "combat", 55, h)
                add_t(msgs[h][1], "mon_hits_other", "combat", 56, h)
            if df in msgs:
                add_t(msgs[df][0], "mon_deflect_you", "combat", 55, df)
                add_t(msgs[df][1], "mon_deflect_other", "combat", 56, df)
                add_t(msgs[df][2], "mon_dodge_you", "combat", 55, df)
            if ms in msgs:
                add_t(msgs[ms][0], "mon_dodge_other", "combat", 56, ms)
                add_t(msgs[ms][1], "mon_miss_you", "combat", 55, ms)
                add_t(msgs[ms][2], "mon_miss_other", "combat", 56, ms)
    for it in rows(directory, "WCCITEMS"):
        for num, kind in ((it.get("hitmsg", 0), "hit"), (it.get("missmsg", 0), "miss")):
            m = msgs.get(num)
            if m:
                reg.verbs[kind].update(v.strip() for v in m[0].split("|") if v.strip())
                reg.verbs[kind + "3"].update(v.strip() for v in m[2].split("|") if v.strip())
        dm = it.get("destroymsg", 0)
        if dm in msgs:
            add_t(msgs[dm][0], "item_destroyed", "equipment", 58, dm)
            add_t(msgs[dm][1], "item_destroyed_other", "equipment", 59, dm)
    reg.verbs["hit"].update(["punch", "kick", "jumpkick", "attack"])
    reg.verbs["hit3"].update(["punches", "kicks", "jumpkicks", "attacks"])
    reg.verbs["miss"].update(["swing at"])
    reg.verbs["miss3"].update(["swings at"])
    for s in rows(directory, "WCCSPELS"):
        n = s.get("msgnum", 0)
        for t in msgs.get(n) or ():
            role = ("spell_cast_you" if t.startswith("You ") else
                    "spell_cast_on_you" if re.search(r"\byou(?:r)?\b", t) else "spell_cast_room")
            add_t(t, role, "spell", 57, n)
    for num, m in msgs.items():
        for t in m:
            add_t(t, "wccmsg", "message", 70, num)


def registry(pt, directory):
    """A registry with the static patterns and the export's game-data patterns."""
    # Registry.load's order: the game data first (the weapon verbs go into the static
    # combat patterns), then the static patterns, then the index
    reg = pt.Registry(prompt=None)
    load(reg, directory, pt)
    reg.load_static()
    reg.build()
    return reg
