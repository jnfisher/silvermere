#!/usr/bin/env python3
"""The M4 statistical comparisons (T-057; layer 4 in docs/testing.md): the cast runs,
the monster casts and the room spell of `scenarios/m4/`, read from a harness report
(`report/m4/<scenario>/{orig.raw,new.raw}`), one side against the other.

  cast-*        fail rate (two proportions, p > 0.01) and the damage histogram of the landed
                casts (chi-square homogeneity, p > 0.01); each side needs `casts` casts (5,000);
                no resisted line for an area spell (fireball)
  monster-cast  the monster's casts at the player per round: landed, resisted and failed
                (chi-square homogeneity of the three counts)
  room          the room spell's damage histogram (chi-square homogeneity) and its lines per
                minute of the run (two Poisson rates)

The tests are T-040's (`stats.py`: chi_square_homogeneity, two_proportion_z, poisson_rates),
so the same pooling and p-values as the M2 comparisons. Stdlib only; run with `python3 -I`.

  m4_stats.py report --report DIR       prints the checks, writes DIR/m4/stats.json; exit 1 on
                                        a failed check or a missing side
  m4_stats.py selftest
"""
import argparse
import collections
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

ALPHA = 0.01
FAIL = re.compile(r"^You attempt to cast (?P<spell>.+?)(?: at .+)?, but fail\.$")
HIT = re.compile(r"^(?:You cast (?P<spell>.+?) (?:at|on) .+ for "
                 r"|The (?P<area>.+?) explodes, causing )(?P<dmg>\d+) damage!$")
RESISTED = re.compile(r"resist", re.I)
MON_CAST = re.compile(r" casts? .+ (?:on|upon|at) you")
MON_RESISTED = re.compile(r"^You resisted .+'s ")
MON_FAILED = re.compile(r" attempted to cast .+ (?:at|on) you, but failed\.$")
ROOM_DMG = re.compile(r"(\d+) damage")
PROMPT = re.compile(r"\[HP=-?\d+[^\]\n]*\]:")


def lines(raw):
    """The text lines of a raw capture, prompts taken off the front."""
    import replay
    bridge, _, _ = replay.load_play()
    text = bridge.plain(raw.decode("cp437", "replace"))
    for line in text.splitlines():
        line = PROMPT.sub("", line).strip()
        if line:
            yield line


def cast_sample(raw, spell):
    out = {"fails": 0, "damage": collections.Counter(), "resisted": 0}
    for line in lines(raw):
        m = FAIL.match(line)
        if m and m.group("spell") == spell:
            out["fails"] += 1
            continue
        m = HIT.match(line)
        if m and (m.group("spell") == spell or m.group("area") == spell):
            out["damage"][int(m.group("dmg"))] += 1
        elif RESISTED.search(line) and spell in line:
            out["resisted"] += 1
    out["casts"] = out["fails"] + sum(out["damage"].values())
    return out


def monster_sample(raw):
    c = collections.Counter()
    for line in lines(raw):
        if MON_RESISTED.search(line):
            c["resisted"] += 1
        elif MON_FAILED.search(line):
            c["failed"] += 1
        elif MON_CAST.search(line):
            c["landed"] += 1
    return c


def room_sample(raw):
    return collections.Counter(int(m.group(1)) for line in lines(raw)
                               for m in [ROOM_DMG.search(line)] if m)


def _vec(counter, keys):
    return [counter.get(k, 0) for k in keys]


def check(what, t, **kw):
    return dict({"what": what, "pass": t["p_value"] > ALPHA}, **t, **kw)


def compare(name, scenario, orig, new, durations=(0.0, 0.0)):
    import stats
    m4 = scenario.get("m4", {})
    out = []
    if "spell" in m4:
        o, n = cast_sample(orig, m4["spell"]), cast_sample(new, m4["spell"])
        want = m4.get("casts", 5000)
        for side, s in (("orig", o), ("new", n)):
            out.append({"what": f"{side} casts", "pass": s["casts"] >= want,
                        "casts": s["casts"], "need": want})
        if o["casts"] and n["casts"]:
            out.append(check("fail rate", stats.two_proportion_z(o["fails"], o["casts"],
                                                                 n["fails"], n["casts"]),
                             orig=o["fails"] / o["casts"], new=n["fails"] / n["casts"]))
        keys = sorted(set(o["damage"]) | set(n["damage"]))
        out.append(check("damage histogram",
                         stats.chi_square_homogeneity(_vec(o["damage"], keys),
                                                      _vec(n["damage"], keys)),
                         bins=len(keys)))
        if m4["spell"] == "fireball":
            out.append({"what": "never resisted", "pass": o["resisted"] == n["resisted"] == 0,
                        "orig": o["resisted"], "new": n["resisted"]})
    elif "rates" in m4:
        o, n = monster_sample(orig), monster_sample(new)
        keys = ["landed", "resisted", "failed"]
        out.append(check("monster casts: landed / resisted / failed",
                         stats.chi_square_homogeneity(_vec(o, keys), _vec(n, keys)),
                         orig=dict(o), new=dict(n)))
        out.append({"what": "the monster cast at all", "pass": sum(o.values()) > 0
                    and sum(n.values()) > 0, "orig": sum(o.values()), "new": sum(n.values())})
    elif m4.get("room_spell"):
        o, n = room_sample(orig), room_sample(new)
        keys = sorted(set(o) | set(n))
        out.append(check("room spell damage", stats.chi_square_homogeneity(_vec(o, keys),
                                                                           _vec(n, keys))))
        if all(durations):
            out.append(check("room spell lines per minute",
                             stats.poisson_rates(sum(o.values()), durations[0],
                                                 sum(n.values()), durations[1])))
    return out


def report(report_dir):
    sc_dir = os.path.join(HERE, "scenarios", "m4")
    results, ok = {}, True
    for f in sorted(os.listdir(sc_dir)):
        if not f.endswith(".json"):
            continue
        with open(os.path.join(sc_dir, f), encoding="utf-8") as fh:
            sc = json.load(fh)
        if "m4" not in sc:
            continue
        name = f[:-5]
        d = os.path.join(report_dir, "m4", name)
        paths = [os.path.join(d, side + ".raw") for side in ("orig", "new")]
        if not all(os.path.isfile(p) for p in paths):
            results[name] = [{"what": "both sides captured", "pass": False}]
            ok = False
            continue
        raws = [open(p, "rb").read() for p in paths]
        durations = (0.0, 0.0)
        summary = os.path.join(d, "summary.json")
        if os.path.isfile(summary):
            with open(summary, encoding="utf-8") as fh:
                durations = tuple(json.load(fh).get("durations", (0.0, 0.0)))
        results[name] = compare(name, sc, raws[0], raws[1], durations)
        ok &= all(c["pass"] for c in results[name])
    for name, checks in results.items():
        for c in checks:
            print(f"{'PASS' if c['pass'] else 'FAIL'}  m4/{name}: {c['what']}"
                  + (f" (p={c['p_value']:.4f})" if "p_value" in c else ""))
    os.makedirs(os.path.join(report_dir, "m4"), exist_ok=True)
    with open(os.path.join(report_dir, "m4", "stats.json"), "w", encoding="utf-8") as fh:
        json.dump(results, fh, indent=1)
    return 0 if ok else 1


def selftest():
    raw = (b"[HP=30/MA=12]:You cast vine strike at giant rat for 7 damage!\r\n"
           b"[HP=30/MA=11]:You attempt to cast vine strike, but fail.\r\n"
           b"The fireball explodes, causing 84 damage!\r\n"
           b"The orc shaman casts flame bolt at you!\r\n"
           b"You resisted orc shaman's flame bolt.\r\n"
           b"The orc shaman attempted to cast flame bolt at you, but failed.\r\n")
    v = cast_sample(raw, "vine strike")
    assert (v["fails"], dict(v["damage"]), v["casts"]) == (1, {7: 1}, 2), v
    f = cast_sample(raw, "fireball")
    assert dict(f["damage"]) == {84: 1}, f
    assert monster_sample(raw) == {"landed": 1, "resisted": 1, "failed": 1}, monster_sample(raw)
    print("selftest ok")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("report")
    r.add_argument("--report", required=True)
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    return report(args.report) if args.cmd == "report" else selftest()


if __name__ == "__main__":
    sys.exit(main())
