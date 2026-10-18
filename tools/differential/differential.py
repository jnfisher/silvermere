#!/usr/bin/env python3
"""A/B differential harness: replay a T-005 scenario against the original (MBBSEmu) and the
silvermere server, normalise both outputs into event sequences, diff them, write a report.

  differential.py run --scenario NAME [--orig SRC] [--new SRC] [--stage] [--report DIR]
  differential.py run --all ...

SRC is `host:port`, `port`, or `capture:DIR` (a T-005 capture directory played back from disk:
no server needed). See README.md.
"""
import argparse
import difflib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import diff  # noqa: E402
import known_gaps  # noqa: E402
import normalise  # noqa: E402
import replay  # noqa: E402
import stats  # noqa: E402
import report  # noqa: E402
import stage  # noqa: E402

ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
CAPTURES = os.path.join(ROOT, "tests", "captures")
SCENARIOS = os.path.join(HERE, "scenarios")
# ORIG_PORT / NEW_PORT as .github/scripts/start-servers.sh (live.yml runs them on 2423/2425)
DEFAULT_ORIG = f"127.0.0.1:{os.environ.get('ORIG_PORT') or 2323}"
DEFAULT_NEW = f"127.0.0.1:{os.environ.get('NEW_PORT') or 2325}"
TIMEOUT_CAP = None   # --max-step-timeout
# --world-json DIR (or SILVERMERE_WORLD_JSON): `silvermere import --json` output, for the
# registry's game-data patterns (room titles, monster, weapon and spell texts; world_json.py)
WORLD_JSON = os.environ.get("SILVERMERE_WORLD_JSON") or None
REPEAT_TIMES = None  # --repeat-times: cap a scenario's `repeat` (a short statistical run)
_REGISTRIES = {}


def registry_for(scenario, pt):
    """(registry, [problems]): the game-data registry when an export is given; a scenario
    marked `world_patterns` (the m4 group: spell lines) fails without one rather than count
    every spell line as unknown."""
    if WORLD_JSON:
        if WORLD_JSON not in _REGISTRIES:
            import world_json
            _REGISTRIES[WORLD_JSON] = world_json.registry(pt, WORLD_JSON)
        return _REGISTRIES[WORLD_JSON], []
    if None not in _REGISTRIES:
        _REGISTRIES[None] = pt.Registry.load(data=False)
    problems = (["no game-data patterns: run with --world-json DIR (silvermere import --json DIR)"]
                if scenario.get("world_patterns") else [])
    return _REGISTRIES[None], problems
# Screens that carry no randomness: compared as bytes (ANSI stripped, then with colour).
DETERMINISTIC_SENDS = ["st", "exp", "i", "set", "spells", "par", "who"]


def load_scenario(name):
    path = os.path.join(SCENARIOS, f"{name}.json")
    overlay = {}
    if os.path.isfile(path):
        with open(path, encoding="utf-8") as f:
            overlay = json.load(f)
    scenario = {"name": name, "script": os.path.join(CAPTURES, name, "script.json"), **overlay}
    scenario["script"] = os.path.abspath(os.path.join(SCENARIOS, scenario["script"]))
    if scenario.get("rerun_restore"):
        scenario["rerun_restore"] = os.path.abspath(os.path.join(SCENARIOS,
                                                                 scenario["rerun_restore"]))
    return scenario


def all_scenarios():
    names = sorted(os.path.basename(os.path.dirname(p)) for p in
                   (os.path.join(CAPTURES, d, "script.json") for d in os.listdir(CAPTURES))
                   if os.path.isfile(p))
    return names


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def load_capture(path):
    script = read_json(os.path.join(path, "script.json"))
    with open(os.path.join(path, "raw.bin"), "rb") as f:
        raw = f.read()
    return script, replay.from_capture(script, raw)


_SETUP_DONE = set()   # (side, source) pairs already run against this instance


# --as USER:PASSWORD (T-040): the m2 statistical scenarios play one script on several
# characters at once; `@USER@`, `@PASSWORD@` and a scenario's own `subst` keys are replaced in
# its script and setup scripts
AS_USER = None
SUBST = {}         # --set KEY=VALUE: more substitutions (a session's own room, `@ROOM@`)
STOP_POOL = None   # --stop-pool DIR: the parallel sessions' sample counts (replay.stop_reached)


def substitute(doc, scenario):
    """`doc` (a script) with the scenario's `subst` and the --as user filled in."""
    subst = dict(scenario.get("subst", {}), **SUBST)
    if AS_USER:
        user, _, password = AS_USER.partition(":")
        subst.update({"@USER@": user, "@PASSWORD@": password or user.lower() + "1",
                      # backspaces over the given name the original's editor offers (the
                      # account's name)
                      "@USERBS@": "\b" * len(user), "@USERLC@": user.lower()})
    if not subst:
        return doc
    text = json.dumps(doc)
    for k, v in subst.items():
        text = text.replace(k, json.dumps(str(v))[1:-1])
    return json.loads(text)


def setup_script(entry, scenario=None):
    """The setup entry's script: a capture scenario by name, or a script file (`script`,
    relative to scenarios/) for a side that cannot replay the capture's keystrokes."""
    if entry.get("script"):
        doc = read_json(os.path.abspath(os.path.join(SCENARIOS, entry["script"])))
    else:
        doc = read_json(os.path.join(CAPTURES, entry["scenario"], "script.json"))
    return substitute(doc, scenario or {})


def live(side, src, scenario, setup):
    """Replays the scenario against host:port; returns (steps, [problems])."""
    host, _, port = src.rpartition(":")
    player = replay.Player(int(port), host or "127.0.0.1", TIMEOUT_CAP)
    if setup:
        for entry in scenario.get("setup", []):
            if entry.get("side", "both") not in ("both", side):
                continue
            key = (side, src, entry.get("script") or entry.get("scenario"), AS_USER)
            # a setup creates an account or a character: once per instance, not per scenario
            if not (entry.get("scenario") or entry.get("script")) or key in _SETUP_DONE:
                continue
            _SETUP_DONE.add(key)
            player.run_script(setup_script(entry, scenario))
    rep = scenario.get("repeat")
    if rep and REPEAT_TIMES:
        rep = dict(rep, times=min(rep["times"], REPEAT_TIMES))
    script = expand_repeat(read_json(scenario["script"]), rep)
    # an m2 scenario may store one fight cycle and repeat it (`m2.repeat`, T-040)
    script = stats.expand_script(script, scenario.get("m2"))
    # `script_options`: script keys a scenario sets on a script it shares with others (a
    # recorded capture), e.g. `echo_anchored` and `settle`
    script.update(scenario.get("script_options", {}))
    script = substitute(script, scenario)
    # `m2.stop` (T-040): cycle until the sample is in or the time is up; parallel sessions of
    # one side pool their counts in --stop-pool
    stop = (scenario.get("m2") or {}).get("stop")
    if stop:
        stop = dict(stop)
        if os.environ.get("SILVERMERE_M2_MINUTES"):
            stop["minutes"] = float(os.environ["SILVERMERE_M2_MINUTES"])
        if STOP_POOL:
            os.makedirs(STOP_POOL, exist_ok=True)
            tag = scenario["name"].replace("/", "_")
            stop["pool_file"] = os.path.join(STOP_POOL, f"{tag}.{side}.{os.getpid()}")
            stop["pool_glob"] = os.path.join(STOP_POOL, f"{tag}.{side}.*")
        script["stop"] = stop
    steps = player.run_script(script)
    if steps:
        steps[0].stop_info = player.stop_info
    return steps


def expand_repeat(script, rep):
    """`"repeat": {"first", "last", "times"}` in a scenario: script steps first..last
    (inclusive) played `times` times in a row, so a long statistical run stores one cycle
    (the m4 cast runs; T-040's `m2.repeat` does the same for m2)."""
    if not rep:
        return script
    steps = script.get("steps", [])
    a, b, times = rep["first"], rep["last"], rep["times"]
    return dict(script, steps=steps[:a] + steps[a:b + 1] * times + steps[b + 1:])


def restore_side(side, src, scenario):
    """Plays the scenario's `rerun_restore` script on one side before a rerun, so the second
    attempt starts from the first one's state (hit points, room); returns its problems."""
    path = scenario.get("rerun_restore")
    if not path or src.startswith("capture:"):
        return []
    host, _, port = src.rpartition(":")
    player = replay.Player(int(port), host or "127.0.0.1", TIMEOUT_CAP)
    return [f"{p} (rerun_restore)" for p in problems_of(side, player.run_script(read_json(path)))]


def side_steps(side, src, scenario, setup):
    if src.startswith("capture:"):
        return load_capture(src[len("capture:"):])[1]
    return live(side, src, scenario, setup)


def problems_of(side, steps, script_steps=None):
    out = []
    for s in steps:
        if s.error:
            out.append(f"{side}: cannot connect ({s.error})" if s.op == "connect"
                       else f"{side}: connection error ({s.error})")
        elif s.timed_out:
            out.append(f"{side}: timeout waiting for {s.until!r}")
        if getattr(s, "walk_out_failed", False):
            out.append(f"{side}: step {s.index}: the flood burst drained mid-burst and the walk out "
                       f"to redo it did not match ({s.burst_retries} redone)")
        if getattr(s, "burst_unclean", False):
            out.append(f"{side}: step {s.index}: the flood burst drained mid-burst in every attempt "
                       f"({s.burst_retries} redone)")
        if side == "new" and getattr(s, "retries", 0):
            # the retry covers the original's (MBBSEmu's) known drop; silvermere must not need it
            out.append(f"{side}: step {s.index} dropped a bare Enter ({s.retries} resent)")
        if s.timed_out or s.error:
            break
    return out


def deterministic_steps(scenario, steps):
    want = scenario.get("deterministic", DETERMINISTIC_SENDS)
    if want == "all":
        return [s.index for s in steps if s.op == "send"]
    return [s.index for s in steps if s.op == "send" and s.send in want]


def merge_steps(steps, groups):
    """The steps with each [first, last] group collapsed into its first step (the bytes of all
    of them, the send of the first)."""
    out = []
    for st in steps:
        grp = next((g for g in groups if g[0] <= st.index <= g[1]), None)
        if grp is None:
            out.append(st)
        elif st.index == grp[0]:
            out.append(replay.StepCapture(st.index, st.op, st.send, st.offset, st.raw, st.matched,
                                          st.until, st.waited, st.error, st.retries,
                                          st.burst_retries, st.burst_unclean))
            out[-1].walk_out_failed = st.walk_out_failed
        else:
            first = next((o for o in out if o.index == grp[0]), None)
            if first is not None:
                first.raw += st.raw
                first.retries += st.retries
                first.burst_retries += st.burst_retries
                first.burst_unclean = first.burst_unclean or st.burst_unclean
                first.walk_out_failed = first.walk_out_failed or st.walk_out_failed
    return out


def collapse_repeats(events):
    """The m2 sequence with each run of identical `drops_unconscious` events as one. The original
    itself prints some lines once or twice by chance: a monster's blow that leaves the player
    up and its hit spell that takes him down knock him out twice, `drops to the ground!` from
    the spell and again from the blow's own check (which looked before the blow); a blow that
    takes him down alone, once."""
    out = []
    for e in events:
        # only the knock-out, the case explained above; any other repeat stays compared (a
        # doubled `you_killed` or `saved` would be a difference)
        if (e["name"] in COLLAPSIBLE and out and out[-1]["name"] == e["name"]
                and out[-1].get("fields") == e.get("fields")):
            continue
        out.append(e)
    return out


COLLAPSIBLE = {"drops_unconscious"}


SCREEN_RX = re.compile(r"Name:.*?MagicRes:[^\r\n]*|Exp:[^\r\n]*?Exp needed for next level:[^\r\n]*",
                       re.DOTALL)


def screens(steps, regexes, plain):
    """[(plain, colour)] of every st and exp screen in a side's run, in order, masked."""
    text = b"".join(st.raw for st in steps).decode("cp437")
    out = []
    for m in SCREEN_RX.finditer(text):
        colour = diff.script_mask(m.group(0), regexes)
        out.append((plain(colour), colour))
    return out


def screen_diffs(orig_steps, new_steps, regexes, plain):
    """byte_diffs-shaped differences between the two sides' screens (statistical scenarios)."""
    o, n = screens(orig_steps, regexes, plain), screens(new_steps, regexes, plain)
    out = []
    if len(o) != len(n):
        out.append({"step": "screens", "kind": "count", "send": f"{len(o)} vs {len(n)} screens"})
    for i, ((op, oc), (np_, nc)) in enumerate(zip(o, n)):
        if op != np_:
            out.append({"step": f"screen {i + 1}", "kind": "plain", "send": "st/exp",
                        "diff": "\n".join(difflib.unified_diff(op.splitlines(), np_.splitlines(),
                                                               "orig", "new", lineterm="", n=1))})
        elif oc != nc:
            out.append({"step": f"screen {i + 1}", "kind": "colour", "send": "st/exp"})
    return out


def compare(scenario, orig_steps, new_steps, masks, setup_problems=()):
    bridge, _, pt = replay.load_play()
    registry, reg_problems = registry_for(scenario, pt)
    setup_problems = list(setup_problems) + reg_problems
    # `mask_prompt`: a deterministic scenario whose two characters cannot start with the same
    # HP/MA (the sysop's staged ones) masks the prompt's numbers as a statistical one does
    if scenario.get("statistical") or scenario.get("mask_prompt"):
        masks = normalise.with_statistical_events(masks)
    # `event_masks`: {event: [fields]} masked in this scenario only (a lit torch's burn count)
    if scenario.get("event_masks"):
        masks = dict(masks, events={k: list(v) for k, v in masks.get("events", {}).items()})
        for name, fields in scenario["event_masks"].items():
            masks["events"].setdefault(name, []).extend(fields)
    script = read_json(scenario["script"]) if os.path.isfile(scenario["script"]) else {}
    regexes = [m["regex"] for m in script.get("mask", [])]
    # a line a prompt redraw cut in two (a typed command's echo, the exit's dots; T-084) is
    # rejoined on both sides before anything is classified or compared; the count per side goes
    # into the redraw totals (a systematic difference between the sides still fails) and notes
    orig_steps, split_o = normalise.rejoin_steps(orig_steps)
    new_steps, split_n = normalise.rejoin_steps(new_steps)
    oe = normalise.events_of(orig_steps, masks, registry) if orig_steps else []
    ne = normalise.events_of(new_steps, masks, registry) if new_steps else []
    # an exact repeat of the prompt event directly after itself in a step: the tick's redraw
    # of the same prompt (diff.collapse_prompt_redraws), counted in the notes
    redraws = {"orig": 0, "new": 0}
    # (also across a step boundary: of `[HP=..]:[HP=..]:look` the first prompt ends one step and
    # the second, with the echo, starts the next; the later one stays)
    for side, evs in (("orig", oe), ("new", ne)):
        kept = []
        for i, e in enumerate(evs):
            nxt = evs[i + 1] if i + 1 < len(evs) else None
            if (e["name"] == "prompt" and nxt is not None and nxt["name"] == "prompt"
                    and nxt["fields"] == e["fields"] and nxt["step"] in (e["step"], e["step"] + 1)):
                redraws[side] += 1
                continue
            kept.append(e)
        evs[:] = kept
    # `value_masks`: [{"event", "where": {field: value}, "fields"}] masks the digits of those
    # fields (each run of them becomes `#`) only in the events whose `where` fields match (one
    # settings line of several, say): the words around the number stay compared
    for vm in scenario.get("value_masks", []):
        for e in oe + ne:
            if e["name"] == vm["event"] and all(
                    e["fields"].get(k) == v for k, v in vm.get("where", {}).items()):
                for f in vm["fields"]:
                    if f in e["fields"]:
                        e["fields"][f] = re.sub(r"\d+", "#", str(e["fields"][f]))
    # steps that only show the BBS host (MBBSEmu's banner box and login echo), not the module
    skip = set(scenario.get("ignore_steps", []))
    if scenario.get("ignore_host_banner"):
        # the BBS host's banner and login echo come again on every (re)connect: drop whatever
        # arrives in a connect step, on either side, instead of numbering the steps
        skip |= {s.index for s in orig_steps + new_steps if s.op == "connect"}
    oe = [e for e in oe if e["step"] not in skip]
    ne = [e for e in ne if e["step"] not in skip]
    # steps whose output lands in one or the other depending on arrival time (a burst typed
    # without waiting) are compared as one block: `merge_steps: [[first, last]]`
    groups = scenario.get("merge_steps", [])
    for e in oe + ne:
        for lo, hi in groups:
            if lo <= e["step"] <= hi:
                e["step"] = lo
    # differences that belong to a card that has not landed (known_gaps.json): classified,
    # counted per owning card, and kept out of the diff
    # and the scenario's `divergences` (same shapes): lines a decided port leaves out on
    # purpose, named by the card that decided it, counted the same way
    rules = known_gaps.load() + scenario.get("divergences", [])
    oe, od = known_gaps.apply(oe, "orig", rules)
    ne, nd = known_gaps.apply(ne, "new", rules)
    gaps = {c: {"orig": od.get(c, 0), "new": nd.get(c, 0)} for c in sorted(set(od) | set(nd))}
    # events random on both servers (random_events.json: monster noise): out of the sequence and
    # the byte comparison, counted here; the m2 scenarios compare their rate (stats.py)
    random_rules = [r for r in known_gaps.load_random()
                    if scenario.get("statistical") or not r.get("statistical_only")]
    oe, orr = known_gaps.apply(oe, "orig", random_rules)
    ne, nr = known_gaps.apply(ne, "new", random_rules)
    random_counts = {c: {"orig": orr.get(c, 0), "new": nr.get(c, 0)}
                     for c in sorted(set(orr) | set(nr))}
    # `random_steps`: a fight whose rolls (cast failures, the monsters' adjectives and attacks)
    # differ on every run is not diffed as events; its unknown lines still count
    rnd = set(scenario.get("random_steps", []))
    oe_d = [e for e in oe if e["step"] not in rnd]
    ne_d = [e for e in ne if e["step"] not in rnd]
    m2 = scenario.get("m2")
    if m2 is not None:
        # T-040: a random scenario compares as a sequence only the events its `m2.sequence`
        # names (refusals, *Combat Engaged*/*Combat Off*, the death lines); the random rest
        # goes to stats.py, and an unclassified line still fails it (`extra` below)
        keep = set(m2.get("sequence", []))
        # in which step an event lands follows the random fight (how many swings, how many
        # monsters before the one that drops the player): the order is compared, not the step
        oe_d = collapse_repeats([dict(e, step=0) for e in oe_d if e["name"] in keep])
        ne_d = collapse_repeats([dict(e, step=0) for e in ne_d if e["name"] in keep])
    value_masks = scenario.get("unknown_value_masks")
    diffs = diff.diff_events(oe_d, ne_d, value_masks)
    extra = diff.unknown_extra(oe, ne, value_masks, as_set=unknown_as_set(scenario))
    # `required_events`: {step: [names]} each present on both sides in that step (a random
    # fight's structure: the cast, the end of combat), its rolls left uncompared
    required = []
    required_steps = set()
    for st, names in scenario.get("required_events", {}).items():
        for side, evs in (("orig", oe), ("new", ne)):
            have = {e["name"] for e in evs if e["step"] == int(st)}
            missing = [n for n in names if n not in have]
            if missing:
                required.append(f"{side}: step {st} lacks {', '.join(missing)}")
                required_steps.add(int(st))
    det = [i for i in deterministic_steps(scenario, orig_steps) if i not in skip | rnd]
    if groups:
        det_all = set(det)
        det = [i for i in det if not any(lo <= i <= hi for lo, hi in groups)]
        for lo, hi in groups:
            if any(lo <= i <= hi for i in det_all):
                det.append(lo)
        orig_steps, new_steps = merge_steps(orig_steps, groups), merge_steps(new_steps, groups)
    bcount = {"orig": {}, "new": {}}   # byte-pattern removals per card, counted like events
    bredraw = {"orig": {}, "new": {}}  # prompt redraws collapsed in the byte check
    if scenario.get("statistical"):
        # a random fight's lines land around the st/exp screens and can move a step boundary
        # by one, so the screens are compared in order over the whole run, not step by step
        bd = screen_diffs(orig_steps, new_steps, regexes, bridge.plain)
    else:
        bd = diff.byte_diffs(orig_steps, new_steps, det, regexes, bridge.plain,
                             lambda t: known_gaps.mask_bytes(t, "orig", rules + random_rules,
                                                             bcount["orig"]),
                             lambda t: known_gaps.mask_bytes(t, "new", rules + random_rules,
                                                             bcount["new"]),
                             echo_on=bool(scenario.get("bytes_from_echo")), redraws=bredraw)
    # a known-gap `shape` rule (T-083): a colour difference that is exactly silvermere's
    # ANSI_NORMAL after the echo is counted under the rule's card, not reported; any other
    # colour difference in the step still fails
    shapes = [r for r in rules if r.get("shape") == "ansi_normal_after_echo"]
    kept = []
    for d in bd:
        views = d.pop("_views", None)
        if shapes and views and d["kind"] == "colour" and diff.ansi_normal_after_echo(*views):
            card = shapes[0]["card"]
            bcount["new"][card] = bcount["new"].get(card, 0) + 1
            continue
        kept.append(d)
    bd = kept
    for side in ("orig", "new"):
        for card, n in bcount[side].items():
            g = gaps.setdefault(card, {"orig": 0, "new": 0})
            g[f"{side}_bytes"] = g.get(f"{side}_bytes", 0) + n
    rejoined = {"orig": sum(split_o.values()), "new": sum(split_n.values())}
    collapsed = {side: redraws[side] + bredraw[side].get("prompt_redraws", 0) + rejoined[side]
                 for side in ("orig", "new")}
    notes = []
    lo, hi = sorted(collapsed.values())
    # a systematic difference fails the scenario: one side collapsing more than twice the
    # other's (and at least 3, so 3 against 0 too) is a problem, not a note
    redraw_problems = []
    if hi >= 3 and hi > 2 * lo:
        msg = (f"prompt redraws differ: orig {collapsed['orig']}, new {collapsed['new']} "
               "(more than twice)")
        # a random scenario (`m2`): a repeat follows each tick that leaves the prompt as it was,
        # so the count goes with how long the random fight lasts (run 39035493410's
        # death-rebirth: orig 11 in 466 s, new 4 in 148 s), a note, not a problem
        if m2 is not None:
            notes.append(msg + "; random scenario, a note")
        else:
            redraw_problems.append(msg)
    notes += [f"{side}: step {i} cut by a prompt redraw, rejoined ({n})"
              for side, counts in (("orig", split_o), ("new", split_n)) for i, n in counts.items()]
    notes += [f"{side}: step {s.index} echo split, matched on the prompt"
              for side, steps in (("orig", orig_steps), ("new", new_steps))
              for s in steps if getattr(s, "echo_missing", False)]
    problems = (list(setup_problems) + problems_of("orig", orig_steps) + problems_of("new", new_steps)
                + required + redraw_problems)
    checks, samples = [], None
    if m2 is not None and orig_steps and new_steps:
        st = stats.scenario_stats(scenario, stats.expand_script(script, m2), orig_steps,
                                  new_steps, stats.load_ab(), registry=registry)
        checks, samples = st["checks"], st["samples"]
        problems += [f"statistics: {c['what']} out of bounds (p = {c['p_value']:.4f}"
                     + (f", Holm threshold {c['holm_alpha']:.5f}" if "holm_alpha" in c else "")
                     + ")" for c in checks if not c["pass"]]
    status = "pass" if not (diffs or extra or bd or problems) else "fail"
    text = diff.unified(oe_d, ne_d)
    if extra:
        shown = extra[:DIAG_LINES]
        text = ("unknown lines only on new:\n" + "\n".join(f"  {ln}" for ln in shown)
                + (f"\n  (and {len(extra) - len(shown)} more)" if len(extra) > len(shown) else "")
                + "\n\n" + text)
    for d in bd:
        text += f"\n\nstep {d['step']} ({d.get('send')!r}) differs as {d['kind']}"
        if d.get("diff"):
            text += "\n" + d["diff"]
    if status == "fail":
        failing = ({d["step"] for d in diffs} | {d["step"] for d in bd if isinstance(d["step"], int)}
                   | required_steps)
        failing |= {st.index for st in orig_steps + new_steps if st.timed_out}
        diag = step_text(sorted(failing)[:DIAG_STEPS], orig_steps, new_steps, bridge.plain)
        if diag:
            text = diag + "\n\n" + text
    for p in problems:
        text = f"{p}\n" + text
    result = {"scenario": scenario["name"], "status": status, "problems": problems,
              "event_diffs": len(diffs), "differences": diffs[:200],
              "unknown_extra": extra, "unknown": {"orig": sum(e["name"] == "unknown" for e in oe),
                                                  "new": sum(e["name"] == "unknown" for e in ne)},
              "byte": {"checked": len(det), "equal": len(det) - len(bd), "differences": bd},
              "events": {"orig": len(oe), "new": len(ne)}, "known_gaps": gaps, "notes": notes,
              "random": random_counts, "stats": checks, "samples": samples,
              "redraws": {side: {"events": redraws[side],
                                 "bytes": bredraw[side].get("prompt_redraws", 0),
                                 "rejoined": rejoined[side]}
                          for side in ("orig", "new")},
              "burst_check": bool(script.get("burst_check")),
              "enter_retries": {"orig": sum(getattr(s, "retries", 0) for s in orig_steps),
                                "new": sum(getattr(s, "retries", 0) for s in new_steps)},
              "burst_retries": {"orig": sum(getattr(s, "burst_retries", 0) for s in orig_steps),
                                "new": sum(getattr(s, "burst_retries", 0) for s in new_steps)}}
    return result, oe, ne, text.strip("\n")


# a failed scenario's diff.txt opens with what each side received in its first failing steps
# (ANSI stripped, the last DIAG_LINES lines of each), so a live run's job log, which prints the
# head of diff.txt, shows the text behind a difference without the report artifact
DIAG_STEPS, DIAG_LINES = 2, 40


def step_text(indexes, orig_steps, new_steps, plain):
    """Both sides' text of each step in `indexes`, the last DIAG_LINES lines of each."""
    out = []
    for i in indexes:
        for side, steps in (("orig", orig_steps), ("new", new_steps)):
            st = next((s for s in steps if s.index == i), None)
            if st is None:
                out.append(f"=== {side} step {i}: not played")
                continue
            lines = plain(st.raw.decode("cp437")).replace("\r\n", "\n").replace("\r", "\n")
            lines = [ln.rstrip() for ln in lines.split("\n")]
            while lines and not lines[-1]:
                lines.pop()
            cut = len(lines) - DIAG_LINES
            head = f"=== {side} step {i} ({st.send!r})" + (", timed out" if st.timed_out else "")
            out.append(head + (f", last {DIAG_LINES} of {len(lines)} lines" if cut > 0 else ""))
            out += lines[max(cut, 0):]
    return "\n".join(out)


# a timed effect wearing off on its own timer: WCCMSG 2296, "You begin to feel calmer.", the end
# of a monster's stun or knockdown (spells 1244, 1245) or of song of battle (801), which an
# earlier scenario's fight can leave running on one side
WEAR_OFF_MSGNUMS = {"2296"}

# the Temple's ambient lines (T-084): WCCMSG 2676 ("The smell of incense wafts faintly in the
# air."), 2677 ("The angelic sound of a choir floats down through the air.") and 2678 (the
# temple bell). Temple rooms carry room spell 925 "temple noise"; the room spell is cast on
# every other medium pass (secmed 3 s), runs WCCTEXT 9127 `random 9128`
# (a text block: one roll 0..99, first line whose percentage is above it), and block 9128 ("97:addexp 0", "98:message 2676", "99:message 2678", "100:message 2677")
# shows incense on a roll of 97, the bell on 98 and the choir on 99, the rest nothing: about
# 1% each, every 6 s, on both servers. A line that comes to one side only is not the
# scenario's; the attempt is played again.
AMBIENT_MSGNUMS = {"2676", "2677", "2678"}


def unknown_as_set(scenario):
    """The sample-count runs (`m2.stop`: the sides play different numbers of cycles) compare
    unclassified lines as digit-masked sets, as does a scenario with `unknown_as_set`; a fixed
    script keeps the multiset, which catches a doubled message."""
    return bool(scenario.get("unknown_as_set", (scenario.get("m2") or {}).get("stop")))


def interference(scenario, orig_steps, oe, ne):
    """A monster arriving or attacking in a step that does not summon one and is not a random
    (fight) step, an earlier fight's effect wearing off, or a Temple's ambient line: interference
    from outside the script, as (step, `side step N: event`) per hit (T-057, T-084)."""
    skip = set(scenario.get("random_steps", []))
    skip |= {st.index for st in orig_steps if (st.send or "").startswith("sysop monster")}
    return [(e["step"], f"{side} step {e['step']}: {e['name']}"
             + (f" {e['fields'].get('msgnum')}" if e["name"] == "wccmsg" else ""))
            for side, evs in (("orig", oe), ("new", ne)) for e in evs
            if e["step"] not in skip
            and (e["name"].startswith("mon_") or e["name"].startswith("monster_arrives")
                 or (e["name"] == "wccmsg"
                     and e["fields"].get("msgnum") in WEAR_OFF_MSGNUMS | AMBIENT_MSGNUMS))
            ] + cleanup_hits(oe)


def cleanup_hits(oe):
    """MBBSEmu's nightly cleanup warning on the original (`replay.CLEANUP_RX`), as interference
    hits: from the first one on the original drops every session and reloads the module, so
    nothing after it compares and a rerun meets the same cleanup."""
    return [(e["step"], f"orig step {e['step']}: cleanup (MBBSEmu nightly auto-cleanup)")
            for e in oe
            if e["name"] == "unknown" and replay.CLEANUP_RX.search(e.get("line", ""))][:1]


def wanderers(scenario, orig_steps, oe, ne):
    """The texts of `interference`."""
    return [t for _, t in interference(scenario, orig_steps, oe, ne)]


def first_failing_step(result, orig_steps, new_steps):
    """The earliest step with an event difference, a byte difference or a timeout, or None."""
    steps = [d["step"] for d in result.get("differences", [])]
    # (a statistical scenario's screen differences name no step: "screens", "screen N")
    steps += [d["step"] for d in result.get("byte", {}).get("differences", [])
              if isinstance(d["step"], int)]
    steps += [s.index for s in orig_steps + new_steps if s.timed_out]
    return min(steps) if steps else None


def run_one(name, args, masks):
    scenario = load_scenario(name)
    # `skip`: {card, why}: a scenario a later card must make reliable is skipped and counted
    # under that card in the report, not played
    if scenario.get("skip"):
        return {"scenario": name, "status": "skipped",
                "reason": f"{scenario['skip']['card']}: {scenario['skip']['why']}"}, None
    if scenario.get("statistical") and not args.include_statistical:
        return {"scenario": name, "status": "skipped",
                "reason": "statistical: run with --include-statistical"}, None
    orig_src = args.orig or DEFAULT_ORIG
    new_src = args.new or DEFAULT_NEW
    orig_steps = side_steps("orig", orig_src, scenario, not args.no_setup)
    new_steps = side_steps("new", new_src, scenario, not args.no_setup)
    result, oe, ne, text = compare(scenario, orig_steps, new_steps, masks)
    # a failed scenario with a wanderer's interference (a monster nobody summoned arriving or
    # attacking outside a fight step) is recorded and played once more; the first attempt's
    # report stays, as <name>/attempt1, and the result carries what interfered
    # Only when the scenario's first failing step is at or after the first interference: a
    # failure before it is the scenario's own. (A summoning scenario reruns too: SYSOP MONSTER
    # forces its spawn past the room's regeneration delay.)
    # `max_reruns` (default 1): wanderers can come again on the second attempt (m4/sysop walks
    # to six towns), so a scenario may be played again up to that many times; each attempt
    # that is rerun stays as <name>/attemptN
    reruns, first_info, notes_first = 0, None, []
    while True:
        found = interference(scenario, orig_steps, oe, ne) if result["status"] == "fail" else []
        first_fail = first_failing_step(result, orig_steps, new_steps)
        hits = [t for _, t in found]
        if any(t.endswith("(MBBSEmu nightly auto-cleanup)") for t in hits):
            # the stated cause, whatever failed first; never rerun into the cleanup
            cleanup = next(t for t in hits if t.endswith("(MBBSEmu nightly auto-cleanup)"))
            result["problems"] = [f"interrupted by the original's nightly cleanup ({cleanup}); "
                                  "not a difference, not rerun"] + list(result.get("problems", []))
            result["cleanup"] = True
            break
        if found and (first_fail is None or first_fail < min(st for st, _ in found)):
            result["notes"] = list(result.get("notes", [])) + [
                f"interference ({'; '.join(hits[:3])}) after the first failing step {first_fail}: "
                "not rerun"]
            hits = []
        if hits and scenario.get("no_rerun"):
            # a scenario that cannot start again (its character is not where it began): the
            # failure stands, and says why it was not played once more
            result["notes"] = list(result.get("notes", [])) + [
                f"interference ({'; '.join(hits[:3])}): not rerun (the scenario cannot start again)"]
            hits = []
        if not hits or scenario.get("statistical"):
            break
        if reruns >= scenario.get("max_reruns", 1):
            result["notes"] = list(result.get("notes", [])) + [
                f"interference again ({'; '.join(hits[:3])}): not rerun, "
                f"{reruns} rerun{'s' if reruns != 1 else ''} spent"]
            break
        reruns += 1
        if args.report:
            report.write_scenario(args.report, f"{name}/attempt{reruns}", result, orig_steps,
                                  new_steps, oe, ne, text)
        first = {"status": result["status"], "event_diffs": result["event_diffs"],
                 "problems": result["problems"], "wanderers": hits}
        # `rerun_restore`: a script that brings each side back to the first attempt's start
        restored = (restore_side("orig", orig_src, scenario)
                    + restore_side("new", new_src, scenario))
        orig_steps = side_steps("orig", orig_src, scenario, not args.no_setup)
        new_steps = side_steps("new", new_src, scenario, not args.no_setup)
        result, oe, ne, text = compare(scenario, orig_steps, new_steps, masks, restored)
        first_info = first_info or first
        notes_first.append(f"{'; '.join(hits[:3])}{'; ...' if len(hits) > 3 else ''}")
    if reruns:
        result["rerun"] = first_info
        result["reruns"] = reruns
        result["notes"] = list(result.get("notes", [])) + [
            (f"rerun once after interference ({notes_first[0]})" if reruns == 1 else
             f"rerun {reruns} times after interference ({' | '.join(notes_first)})")]
    if args.report:
        report.write_scenario(args.report, name, result, orig_steps, new_steps, oe, ne, text)
    return result, text


def cmd_run(args):
    global TIMEOUT_CAP, WORLD_JSON, REPEAT_TIMES, AS_USER, STOP_POOL, SUBST
    AS_USER, STOP_POOL = args.as_user, args.stop_pool
    SUBST = dict(kv.split("=", 1) for kv in args.set or [])
    TIMEOUT_CAP = args.max_step_timeout
    REPEAT_TIMES = args.repeat_times
    WORLD_JSON = args.world_json or WORLD_JSON
    if not args.all and not args.scenario and not args.scenarios:
        sys.exit("--scenario NAME, --scenarios GROUP or --all")
    try:
        if args.stage:
            stage.stage_both(args)
        return run_scenarios(args)
    finally:
        if args.stage:
            stage.stop_both()


def group_scenarios(group):
    """`--scenarios m1`: every scenario file of scenarios/m1/, as `m1/NAME`."""
    d = os.path.join(SCENARIOS, group)
    return sorted(f"{group}/{f[:-5]}" for f in os.listdir(d) if f.endswith(".json"))


def retry_note(result):
    """The harness's retries for the job log: always for a scenario with a `burst_check`, else
    only when there were any."""
    parts = []
    for key in ("burst_retries", "enter_retries"):
        n = result.get(key) or {}
        if (key == "burst_retries" and result.get("burst_check")) or any(n.values()):
            parts.append(f"{key} orig={n.get('orig', 0)} new={n.get('new', 0)}")
    return f"  [{', '.join(parts)}]" if parts else ""


def run_scenarios(args):
    names = (group_scenarios(args.scenarios) if args.scenarios
             else all_scenarios() if args.all else [args.scenario])
    masks = normalise.load_masks()
    results = []
    for name in names:
        result, text = run_one(name, args, masks)
        results.append(result)
        line = f"{result['status'].upper():5} {name}"
        if result["status"] == "fail":
            line += f"  ({result['event_diffs']} event diffs, {len(result['problems'])} problems)"
        line += retry_note(result)
        print(line)
        if result["status"] == "fail" and not (args.all or args.scenarios) and text:
            print(text[:4000])
        elif result["status"] == "fail":
            for p in result["problems"]:
                print(f"  {p}")
    if args.report:
        report.write_summary(args.report, results)
    return 1 if any(r["status"] == "fail" for r in results) else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    sub = ap.add_subparsers(dest="cmd", required=True)
    rp = sub.add_parser("replay", help="play one script against one server (a preparation)")
    rp.add_argument("--script", required=True, help="path under scenarios/")
    rp.add_argument("--to", required=True, help="host:port")
    rp.add_argument("--max-step-timeout", type=float)
    rp.add_argument("--as", dest="as_user", metavar="USER:PASSWORD",
                    help="`@USER@`, `@PASSWORD@` in the script (an m2 sysop's account)")
    rp.add_argument("--text", metavar="FILE", help="write what the server sent, as plain text")
    r = sub.add_parser("run")
    r.add_argument("--scenario")
    r.add_argument("--all", action="store_true")
    r.add_argument("--scenarios", metavar="GROUP",
                   help="every scenario in scenarios/GROUP/ (e.g. m1), in name order")
    r.add_argument("--orig", help=f"host:port or capture:DIR (default {DEFAULT_ORIG})")
    r.add_argument("--new", help=f"host:port or capture:DIR (default {DEFAULT_NEW})")
    r.add_argument("--stage", action="store_true", help="stage and start both servers first")
    r.add_argument("--report", help="write the report into this directory")
    r.add_argument("--no-setup", action="store_true", help="do not run the scenario's setup scripts")
    r.add_argument("--max-step-timeout", type=float, help="cap every step's timeout (seconds)")
    r.add_argument("--include-statistical", action="store_true")
    r.add_argument("--repeat-times", type=int, metavar="N",
                   help="play a scenario's repeated cycle at most N times (a short statistical run)")
    r.add_argument("--as", dest="as_user", metavar="USER:PASSWORD",
                   help="play the scenario as this account (`@USER@`, `@PASSWORD@` in its scripts)")
    r.add_argument("--set", action="append", metavar="KEY=VALUE",
                   help="replace KEY by VALUE in the scenario's scripts, e.g. @ROOM@=2500")
    r.add_argument("--stop-pool", metavar="DIR",
                   help="parallel sessions' sample counts for a scenario's `m2.stop`")
    r.add_argument("--world-json", metavar="DIR",
                   help="`silvermere import --json DIR` output: game-data patterns for the "
                        "classifier (default $SILVERMERE_WORLD_JSON)")
    args = ap.parse_args(argv)
    if args.cmd == "replay":
        return cmd_replay(args)
    return cmd_run(args)


def cmd_replay(args):
    """`replay --script FILE --to HOST:PORT`: plays one script (a path under scenarios/, e.g. a
    one-time preparation like m4/setup/new-sysadm.json) and reports its problems."""
    host, _, port = args.to.rpartition(":")
    player = replay.Player(int(port), host or "127.0.0.1", args.max_step_timeout)
    global AS_USER
    AS_USER = args.as_user
    script = read_json(os.path.abspath(os.path.join(SCENARIOS, args.script)))
    steps = player.run_script(substitute(script, {}))
    if args.text:
        # what the server sent, ANSI stripped (m2_live.sh reads SYSOP BUFFERS from it)
        bridge, _, _ = replay.load_play()
        with open(args.text, "w", encoding="utf-8") as f:
            f.write(bridge.plain(b"".join(st.raw for st in steps).decode("cp437", "replace")))
    problems = problems_of(args.to, steps)
    for p in problems:
        print(p)
    print(f"{args.script}: {len(steps)} steps, {'ok' if not problems else 'problems'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
