"""Known gaps: differences that belong to a card that has not landed yet.

`known_gaps.json` lists rules; each names the owning card. `apply(events, side, rules)` drops
the events a rule covers from one side's event list (before the event diff, so one missing
event does not shift every later pair) and counts them per card.

Rule shapes:
  {"card", "side": "orig"|"new"|"both", "events": [names], "unknown_lines": [regexes],
   "then_drop": [names]}
      events of those names, and `unknown` events whose line matches, on that side (or both);
      with `then_drop`, an event of one of those names directly after a dropped one, in the
      same step, goes too (a monster's noise is sent with a prompt of its own); when the
      dropped event is the last of its step, the one directly before it goes instead (the noise
      came after the step's own prompt, erased it with ANSI_CLRLINE and drew it again, so
      the redrawn prompt sits on the next step's echo line)
  {"card", "side": "orig"|"new"|"both", "byte_patterns": [regexes]}
      removed from that side's decoded step text before the byte-equal check
  {"card", "side": "orig"|"new"|"both", "field_subs": {event: {field: [[regex, replacement], ...]}}}
      the field of each kept event of that name, on that side, with the substitutions applied
      (a difference of the emulator that is not a difference of the game, normalised away
      instead of dropped, so the rest of the field is still compared); each substitution that
      fires is counted under the card
  {"card", "command": "par", "orig": [names], "new": [names]}
      within the steps in which that command was typed, the named events of each side
  {"card", "side", "command": "x", "after_echo": [names]}
      in the steps in which that command was typed, exactly that run of events directly after
      the command's echo (T-093: the original's `*Combat Off*` and its prompt before the exit's
      first line); anything else in the step stays
  {"card", "side", "command": "x", "unknown_lines": [regexes]}
      `unknown` events matching, in the steps in which that command was typed only; with
      `"then_drop_one": [names]`, one event of those names directly after a dropped one, in the
      same step (T-093: the prompt the cast's mana change redraws among the dots), nothing more
"""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
PATH = os.path.join(HERE, "known_gaps.json")


RANDOM = os.path.join(HERE, "random_events.json")


def load(path=PATH):
    with open(path, encoding="utf-8") as f:
        return json.load(f)["rules"]


def load_random(path=RANDOM):
    """The rules of random_events.json (same shapes): events random on both servers by nature."""
    return load(path)


def _step_commands(events):
    """step -> the command typed in it (the first `echo` event's cmd)."""
    cmds = {}
    for e in events:
        if e["name"] == "echo" and e["step"] not in cmds:
            cmds[e["step"]] = e["fields"].get("cmd")
    return cmds


def _subs(e, side, rules, hits):
    for r in rules:
        if r.get("side") in (side, "both"):
            for field, subs in r.get("field_subs", {}).get(e["name"], {}).items():
                val = e["fields"].get(field)
                if isinstance(val, str):
                    for rx, rep in subs:
                        val, n = re.subn(rx, rep, val)
                        if n:
                            hits[r["card"]] = hits.get(r["card"], 0) + n
                    e = {**e, "fields": {**e["fields"], field: val}}
    return e


def apply(events, side, rules):
    """(kept events, {card: count}): events dropped, plus the fields a rule rewrote."""
    cmds = _step_commands(events)
    dropped = {}
    kept = []
    # `after_echo`: the run of events directly after the command's echo, found up front
    pre = {}
    for r in rules:
        if "after_echo" in r and r.get("side") in (side, "both"):
            names = r["after_echo"]
            for i, e in enumerate(events):
                run = events[i + 1:i + 1 + len(names)]
                if (e["name"] == "echo" and cmds.get(e["step"]) == r["command"]
                        and [x["name"] for x in run] == names
                        and all(x["step"] == e["step"] for x in run)):
                    for k in range(i + 1, i + 1 + len(names)):
                        pre[k] = r["card"]
    follow = None   # (rule, names, step, before): the rule whose dropped event may be followed
    #                 by these; `before`, the index in `kept` of the same kind of event directly
    #                 before the dropped one, dropped instead when none follows

    def unfollowed(nxt):
        # the dropped event was the last of its step (`nxt`, the next event, is in another step
        # or there is none): the one of those names directly before it goes instead
        if follow and follow[3] is not None and (nxt is None or nxt["step"] != follow[2]):
            kept.pop(follow[3])
            dropped[follow[0]["card"]] = dropped.get(follow[0]["card"], 0) + 1

    one = None   # (names, step, card): a single event that may follow a dropped one
    for idx, e in enumerate(events):
        card = None
        then = None
        mine, one = one, None
        if mine and e["name"] in mine[0] and e["step"] == mine[1]:
            dropped[mine[2]] = dropped.get(mine[2], 0) + 1
            continue
        if idx in pre:
            dropped[pre[idx]] = dropped.get(pre[idx], 0) + 1
            continue
        if follow and e["name"] in follow[1] and e["step"] == follow[2]:
            card = follow[0]["card"]
        else:
            unfollowed(e)
        follow = None
        for r in [] if card else rules:
            if "after_echo" in r:
                continue
            if "command" in r and "unknown_lines" in r:
                if (r.get("side") in (side, "both") and cmds.get(e["step"]) == r["command"]
                        and e["name"] == "unknown"
                        and any(re.search(rx, e.get("line", "")) for rx in r["unknown_lines"])):
                    card = r["card"]
                    if r.get("then_drop_one"):
                        one = (r["then_drop_one"], e["step"], card)
            elif "command" in r:
                if cmds.get(e["step"]) == r["command"] and e["name"] in r.get(side, []):
                    card = r["card"]
            elif r.get("side", side) in (side, "both"):
                if e["name"] in r.get("events", []):
                    card = r["card"]
                    then = r
                elif e["name"] == "unknown" and any(
                        re.search(rx, e.get("line", "")) for rx in r.get("unknown_lines", [])):
                    card = r["card"]
            if card:
                break
        if card:
            dropped[card] = dropped.get(card, 0) + 1
            if then and then.get("then_drop"):
                before = (len(kept) - 1 if kept and kept[-1]["name"] in then["then_drop"]
                          and kept[-1]["step"] == e["step"] else None)
                follow = (then, then["then_drop"], e["step"], before)
        else:
            kept.append(_subs(e, side, rules, dropped))
    unfollowed(None)
    return kept, dropped


def mask_bytes(text, side, rules, counts=None):
    """Decoded step text with the lines of known gaps removed: the `byte_patterns` of the rules
    for `side` and of the rules for `both`. With `counts`, each removal is counted there per
    card, so the report shows the gap and does not just hide it."""
    for r in rules:
        if r.get("side") in (side, "both"):
            for rx in r.get("byte_patterns", []):
                text, n = re.subn(rx, "", text, flags=re.DOTALL)
                if n and counts is not None:
                    counts[r["card"]] = counts.get(r["card"], 0) + n
    return text
