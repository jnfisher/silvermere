#!/usr/bin/env python3
"""Adds the finished segments of a soak run together and compares silvermere with the original.

  compare.py NEW_RUN [ORIG_RUN] [--serve-log silvermere-serve.log]

Gates (server-side only; the exit status is 1 when one fails):
  - no disconnect (a bridge that is no longer alive, or a bot that never got into the realm),
  - no crash (`panicked` in the silvermere serve log, when given),
  - no unknown line on silvermere that the original's run does not also have (with only one
    run: no unknown line at all).
Advisory (printed, never fails): exp/hr, deaths, position-ambiguous notes, unfollowed lines and
the follower's credit, flagged only when the two sides differ by more than a factor of two.

A segment is complete when it has `end`; a machine restart leaves an unfinished `seg-NN`, which
is skipped. The bridge log directory of each character is read from `bridges.txt` (`logdir`),
its `raw.bin` classified with the pattern registry (with the world data when $MMUD_DATA names it).
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PLAY = os.path.join(HERE, "vendor", "play")
sys.path.insert(0, PLAY)

import patterns  # noqa: E402


def segments(run):
    for d in sorted(os.listdir(run)):
        p = os.path.join(run, d)
        if d.startswith("seg-") and os.path.isfile(os.path.join(p, "end")):
            yield p


def status_lines(seg):
    with open(os.path.join(seg, "bridges.txt"), encoding="utf-8", errors="replace") as f:
        return [line for line in f if "logdir" in line]


def registry():
    try:
        import mmud_data as md
        return patterns.Registry.load(world=md.World()), True
    except Exception as e:   # no world data: the unknown count is inflated, say so
        print(f"(no world data for the registry: {e}; unknown lines include game-data lines)")
        return patterns.Registry.load(data=False), False


# Lines that are correct but that the registry has no pattern for. The PARTY screen's rows
# (`  <name 30> (<class>) [M:nn%] [H:nn%]<poison><rest> - <rank>rank`) are one.
KNOWN_UNCLASSIFIED = [re.compile(r"\(\w+\)\s+(?:\[[MK]:\s*#+%\]\s*)?\[H:\s*-?#+%\].*- (?:Front|Mid|Back)rank")]


# A menu or creation prompt with the bot's own typing glued after it (the bot kept sending game
# commands at a prompt, e.g. after its character was deleted on the last life).
# The typing is the client's, the prompt is the server's: the prompt alone is what gets judged.
PROMPTS = [re.compile(p) for p in (
    r"^Please choose your race \[ \? for help \] :$",
    r"^You must choose a valid race\. \[ \? for help \]$",
    r"^\[MAJORMUD\]:$",
)]


def without_echo(text, typed):
    """`text` minus a trailing echo of something the bot typed, else None."""
    low = text.lower()
    for t in sorted(typed, key=len, reverse=True):
        if t and low.endswith(t) and len(text) > len(t):
            return text[:-len(t)].rstrip()
    return None


# What the bots say when a party is split (party.py): counted per side, advisory.
SEPARATED = {
    "got separated": "got separated (@comeback)",
    "is missing: going back": "going back for a missing member",
    "couldn't reunite": "could not reunite",
    "stopped waiting for": "stopped waiting",
    "is coming back for us: staying here": "waiting for the leader to come back",
    "no longer following": "bot says no longer following",
}


def shape(line):
    return re.sub(r"\d+", "#", line.strip())


def sent_texts(logdir):
    """The lines the bot typed (bridge `events.jsonl`, kind `send`): their echo is the client's
    own typing, not server output, so it must not count as an unknown line."""
    out = set()
    path = os.path.join(logdir, "events.jsonl")
    if os.path.isfile(path):
        for line in open(path, encoding="utf-8", errors="replace"):
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if ev.get("kind") == "send":
                out.add(str(ev.get("text", "")).strip().lower())
    return out


def measure(run, reg):
    tot = {"hours": 0.0, "segments": 0, "unknown": set(), "unknown_n": 0, "deaths": 0,
           "unfollowed": 0, "ambiguous": 0, "chargen": 0, "chargen_in": [], "separated": {}, "disconnects": 0, "exp": {}, "kills": {}}
    for seg in segments(run):
        secs = int(open(os.path.join(seg, "end")).read()) - int(open(os.path.join(seg, "start")).read())
        tot["hours"] += secs / 3600
        tot["segments"] += 1
        for who in ("Brynja", "Isolde"):
            out = os.path.join(seg, f"{who}.out")
            if os.path.isfile(out):
                text = open(out, errors="replace").read()
                tot["ambiguous"] += text.count("position ambiguous")
                for key in SEPARATED:
                    n = text.count(key)
                    if n:
                        tot["separated"][key] = tot["separated"].get(key, 0) + n
                tot["disconnects"] += text.count("could not get into the realm")
        lines = status_lines(seg)
        tot["disconnects"] += sum("'alive': False" in line for line in lines)
        for who, line in zip(("Brynja", "Isolde"), lines):
            m = re.search(r"'logdir': '([^']+)'", line)
            raw = os.path.join(m.group(1), "raw.bin") if m else ""
            if not os.path.isfile(raw):
                continue
            typed = sent_texts(m.group(1))
            for e in reg.iter_events(patterns.raw_to_lines(open(raw, "rb").read())):
                if e.name == "exp_gain":
                    tot["exp"][who] = tot["exp"].get(who, 0) + int(e.fields.get("exp", 0))
                    tot["kills"][who] = tot["kills"].get(who, 0) + 1
                elif e.name == "unknown":
                    text = (getattr(e, "line", "") or "").strip()
                    if text.lower() in typed:
                        continue   # the echo of a command the bot typed
                    if any(rx.search(shape(text)) for rx in KNOWN_UNCLASSIFIED):
                        continue
                    bare = without_echo(text, typed)
                    if bare is not None and any(rx.search(bare) for rx in PROMPTS):
                        if "choose your race" in bare:
                            tot["chargen"] += 1   # advisory: the character was gone at login
                            where = f"{os.path.basename(seg)}/{who}"
                            if where not in tot["chargen_in"]:
                                tot["chargen_in"].append(where)
                        continue
                    tot["unknown_n"] += 1
                    tot["unknown"].add(shape(text))
                elif e.name == "you_killed":
                    tot["deaths"] += 1
                elif e.name == "unfollowed":
                    tot["unfollowed"] += 1
    return tot


def report(name, t):
    h = t["hours"] or float("nan")
    k = t["kills"]
    lead, fol = k.get("Brynja", 0), k.get("Isolde", 0)
    print(f"{name}: {t['segments']} segments, {t['hours']:.2f} h")
    for who in ("Brynja", "Isolde"):
        print(f"  {who}: exp/hr {t['exp'].get(who, 0) / h:.0f} ({k.get(who, 0)} kills credited)")
    # exp_gain lines per character: the two logs cannot be lined up kill by kill, so the figure is
    # the smaller count over the larger (1.00 = both credited on every kill either saw)
    both = max(lead, fol)
    print(f"  credit overlap {(min(lead, fol) / both) if both else float('nan'):.0%} "
          f"(smaller kill count over the larger)")
    if t["separated"]:
        print("  party split notes from the bots: " + ", ".join(
            f"{SEPARATED[k]} x{n}" for k, n in sorted(t["separated"].items())))
    print(f"  deaths {t['deaths']}, unknown lines {t['unknown_n']}, position-ambiguous notes "
          f"{t['ambiguous']}, unfollowed lines {t['unfollowed']}, disconnects {t['disconnects']}")
    if t["chargen"]:
        print(f"  race prompts met with game commands typed at them: {t['chargen']} "
              "(a character was deleted on its last life: bot problem, advisory) in "
              + ", ".join(t["chargen_in"]))


def differs(a, b):
    """True when two non-negative figures differ by more than a factor of two."""
    if a == b:
        return False
    lo, hi = sorted((a, b))
    return lo == 0 or hi / lo > 2


def main():
    args = sys.argv[1:]
    serve_log = None
    if "--serve-log" in args:
        i = args.index("--serve-log")
        serve_log = args[i + 1]
        del args[i:i + 2]
    if not args:
        sys.exit(__doc__)
    reg, _ = registry()
    new = measure(args[0], reg)
    report(args[0], new)
    orig = None
    if len(args) > 1:
        orig = measure(args[1], reg)
        report(args[1], orig)
    failed = []
    if new["disconnects"]:
        failed.append(f"{new['disconnects']} disconnect(s) on silvermere")
    if serve_log and os.path.isfile(serve_log):
        crashes = open(serve_log, errors="replace").read().count("panicked")
        if crashes:
            failed.append(f"{crashes} panic(s) in {serve_log}")
    extra = new["unknown"] - (orig["unknown"] if orig else set())
    if extra:
        failed.append(f"{len(extra)} unknown line shape(s) only on silvermere:")
        failed.extend(f"    {x!r}" for x in sorted(extra)[:40])
    if orig:
        ea, eb = sum(new["exp"].values()) / (new["hours"] or 1), sum(orig["exp"].values()) / (orig["hours"] or 1)
        for what, a, b in (("exp/hr", ea, eb), ("deaths", new["deaths"], orig["deaths"]),
                           ("position-ambiguous notes", new["ambiguous"], orig["ambiguous"]),
                           ("unfollowed lines", new["unfollowed"], orig["unfollowed"])):
            if differs(a, b):
                print(f"ADVISORY: {what} differ a lot: silvermere {a:.0f}, original {b:.0f}")
    for name, t in (("silvermere", new), ("original", orig)):
        if t and sum(t["separated"].values()) >= 3 and min(t["kills"].get("Brynja", 0), t["kills"].get("Isolde", 0)) < 0.5 * max(t["kills"].get("Brynja", 0), t["kills"].get("Isolde", 0), 1):
            print(f"ADVISORY: the party was split often on {name} "
                  f"({sum(t['separated'].values())} notes) and one member got far fewer kills: a bot problem, not a server one")
    print("GATES: " + ("FAIL" if failed else "PASS"))
    for f in failed:
        print(f"  {f}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
