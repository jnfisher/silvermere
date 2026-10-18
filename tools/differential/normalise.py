"""Raw bytes -> event sequences, with the same semantics as silvermere-testkit's `normalise`.

`raw_to_lines` + the pattern registry's `iter_events` (the Rust `PatternSet` does the same),
then the fields named in `tests/fixtures/masks.json` become "*". The mask file is the one the
Rust tests load; this module never carries a mask list of its own.

Event: {"step", "name", "cat", "fields"} plus "line" on `unknown` events. `step` is the script
step whose send most recently preceded the event (the first step before any send).
"""
import copy
import json
import os
import re

from replay import load_play

HERE = os.path.dirname(os.path.abspath(__file__))
MASKS = os.path.join(HERE, "..", "..", "tests", "fixtures", "masks.json")
MASKED = "*"


def load_masks(path=MASKS):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def with_script_events(masks):
    """Adds the `script_events` masks (a script that says Lives/CP are random)."""
    out = dict(masks)
    events = {k: list(v) for k, v in masks.get("events", {}).items()}
    for name, fields in masks.get("script_events", {}).items():
        events.setdefault(name, []).extend(fields)
    out["events"] = events
    return out


def with_statistical_events(masks):
    """Adds the `statistical_events` masks: the prompt's HP/MA stay random after the damage is
    masked (carried from the T-014 review)."""
    out = dict(masks)
    events = {k: list(v) for k, v in masks.get("events", {}).items()}
    for name, fields in masks.get("statistical_events", {}).items():
        events.setdefault(name, []).extend(fields)
    out["events"] = events
    return out


def _masked(masks, event, field):
    if field in masks.get("fields", []):
        return True
    fs = masks.get("events", {}).get(event, [])
    return "*" in fs or field in fs


def anchor_sends(raw, sends):
    """Sends with their marker offset moved onto their own echo.

    A step's offset is read right after the send, so it lands before or after the echo
    depending on arrival time, and a burst typed without waiting records one offset for all of
    its sends, which puts every marker on one line, where the registry keeps only the last few
    (`search` then classifies as `unknown`, or as `echo` when an echo happened to arrive between
    two sends). So each send is matched, in order, with the first echo of its text (`text` + CR
    after a prompt's `:` or `?`) that is not before the send and not already claimed; with no
    such echo (a dropped line) the offset stays as recorded."""
    out, floor = [], 0
    for step, off, text in sends:
        if text:
            want = text.encode("cp437", errors="replace") + b"\r"
            lo = max(floor, off - 80)
            i = raw.find(want, lo)
            while i != -1 and raw[i - 1:i] not in (b":", b"?"):
                i = raw.find(want, i + 1)
            if i != -1:
                floor = i + len(want)
                off = i
        out.append((step, off, text))
    return out


def combined_lines(raw, sends):
    """`raw_to_lines(raw)` with a sent marker before the line in which each send happened
    (`capture.plain_with_markers`), and the step each combined line belongs to.
    sends: [(step, byte offset, text)]."""
    _, _, pt = load_play()
    lines = pt.raw_to_lines(raw)
    at = {}
    for step, off, text in anchor_sends(raw, sends):
        k = len(pt.raw_to_lines(raw[:off])) - 1
        at.setdefault(k, []).append((step, text))
    out, steps = [], []
    cur = sends[0][0] if sends else 0

    def put(k):
        nonlocal cur
        for step, text in at.pop(k, []):
            cur = step
            if text is None:        # a (re)connect: later lines belong to it, nothing was typed
                continue
            out.append(pt.sent_marker(text))
            steps.append(cur)

    for i, line in enumerate(lines):
        put(i)
        out.append(line)
        steps.append(cur)
    for k in sorted(at):
        put(k)
    return out, steps


# A prompt redraw (the slow tick: clear the line, then the prompt)
# can land in the middle of a line the server is still writing. Two shapes only are rejoined
# (T-084), because anything else could hide a real difference in a message:
#  (a) inside a typed command's echo: the text before the redraw (after the line's own prompt) is a
#      case-insensitive prefix of the step's `send`, and the text after it completes the send (or
#      it is the whole send and the line ends right after the redraw);
#  (b) inside the exit meditation's dots: dots only on both sides, SGR aside;
# A redraw directly after the step's own complete echo line, the command's output glued after it
# on the same line, cannot come from the original: MBBSEmu runs the input handler in the same pass
# as the CR echo and the timers after all channels (MbbsHost.cs:426, :929, :458-459), and
# silvermere dispatches a completed line before a tick due with it (T-094). It is never rejoined.
_REDRAW = re.compile(rb"\x1b\[79D\x1b\[K(?:\x1b\[[0-9;]*m)*\[HP=[^\]\r\n]*\]:")
_SGRB = re.compile(rb"\x1b\[[0-9;]*m")
_CSI = re.compile(rb"^\x1b\[[0-9;]*[A-Za-z]")   # a line-start prompt opens with CSI 79D, CSI K
_PROMPT = re.compile(rb"^\[HP=[^\]]*\]:(?: \(Resting\))?")


def _strip_csi(t):
    while True:
        u = _CSI.sub(b"", t)
        if u == t:
            return t
        t = u


def _split_kind(raw, a, b, send):
    """'echo', 'dots' or None for the redraw at raw[a:b]."""
    start = max(raw.rfind(b"\n", 0, a), raw.rfind(b"\r", 0, a)) + 1
    ends = [i for i in (raw.find(b"\r", b), raw.find(b"\n", b)) if i != -1]
    end = min(ends) if ends else len(raw)
    # other redraws in the same line (two in one echo) are not part of the text either
    clean = lambda t: _SGRB.sub(b"", _REDRAW.sub(b"", t))
    before, after = clean(raw[start:a]), clean(raw[b:end])
    if before and after and set(before) == {46} and set(after) == {46}:
        return "dots"
    if send:
        typed = _PROMPT.sub(b"", _strip_csi(before), count=1)
        want = send.encode("cp437", errors="replace").lower()
        # a redraw after the whole command, before its CR echo: the tick ran between the last
        # character and the CR (one byte per pass on the original, timers after each; T-094)
        at_eol = raw[b:b + 1] in (b"\r", b"\n")
        if typed and (after or at_eol) and (typed + after).lower() == want:
            return "echo"
    return None


KINDS = ("echo", "dots")


def rejoin_split_lines(raw, send=None, kinds=KINDS):
    """`(raw', n)`: `raw` without the n prompt redraws that cut an echo of `send` or the exit's
    dots in two (`kinds`: which shapes)."""
    out, last, n = [], 0, 0
    for m in _REDRAW.finditer(raw):
        a, b = m.span()
        if _split_kind(raw, a, b, send) in kinds:
            out.append(raw[last:a])
            last = b
            n += 1
    out.append(raw[last:])
    return b"".join(out), n


def rejoin_steps(steps, kinds=KINDS):
    """`(steps', {step index: n})`: copies of the step captures with each split line rejoined
    (`rejoin_split_lines`, against the step's own send) and the offsets of every connection
    moved to match."""
    removed, counts, out = {}, {}, []
    for s in steps:
        c = copy.copy(s)
        client = getattr(s, "client", "A")
        raw, n = rejoin_split_lines(s.raw, s.send if s.op == "send" else None, kinds)
        c.offset = s.offset - removed.get(client, 0)
        removed[client] = removed.get(client, 0) + len(s.raw) - len(raw)
        c.raw = raw
        if n:
            counts[s.index] = n
        out.append(c)
    return out, counts


_DOTS = re.compile(r"^\.+$")
_SGR = re.compile(r"\x1b\[[0-9;]*m")


def _dot_progress(line):
    """The silent-meditation exit's progress line: dots only. The first dot ends the exit line
    (`"... silent meditation.\\r."`) and each fast tick adds `"..."` while the
    player is `ST_EXITING`: 1 + 5 x 3 = 16 dots."""
    return bool(_DOTS.match(_SGR.sub("", line or "").strip()))


def events_for(raw, sends, masks, registry=None):
    """Masked events of a whole run. `sends` as in `combined_lines`."""
    _, _, pt = load_play()
    registry = registry or pt.Registry.load(data=False)
    lines, steps = combined_lines(raw, sends)
    out = []
    for e in registry.iter_events(lines):
        fields = {k: str(v) for k, v in e.fields.items() if v is not None}
        for k in fields:
            if _masked(masks, e.name, k):
                fields[k] = MASKED
        ev = {"step": steps[e.lineno] if e.lineno < len(steps) else steps[-1] if steps else 0,
              "name": e.name, "cat": e.cat, "fields": fields}
        if e.name == "unknown":
            ev["line"] = e.line
        if e.name == "chargen_values" and _dot_progress(e.line):
            # the registry's loose `chargen_values` also matches a line of dots: the exit's
            # progress line gets its own name. One event per line, so a line break among the
            # dots (a real difference) still shows as an extra event.
            ev["name"] = "exit_progress"
        out.append(ev)
    return out


def sends_of(step_captures):
    """[(step, offset, text)] of the send steps of a StepCapture list, and of its connect steps
    (text None: the host banner of a (re)connect belongs to that step, not to the step before)."""
    return [(s.index, s.offset, s.send if s.op == "send" else None)
            for s in step_captures if s.op in ("send", "connect")]


def _events_of_one(step_captures, masks, registry=None):
    raw = b"".join(s.raw for s in step_captures)
    base = step_captures[0].offset if step_captures else 0
    sends = [(i, off - base, t) for i, off, t in sends_of(step_captures)]
    return events_for(raw, sends, masks, registry)


def events_of(step_captures, masks, registry=None):
    """Masked events of a run. Steps of one connection are one stream (a step's bytes end where
    the next step of the same connection begins, so their raws join into that connection's
    bytes and their offsets are relative to its first step). A script with several connections
    (`"as"`, replay.py) is classified per connection, then the events are merged in step order,
    each step's events staying together."""
    clients = {}
    for s in step_captures:
        clients.setdefault(getattr(s, "client", "A"), []).append(s)
    if len(clients) <= 1:
        return _events_of_one(step_captures, masks, registry)
    out = []
    for steps in clients.values():
        out.extend(_events_of_one(steps, masks, registry))
    out.sort(key=lambda e: e["step"])   # stable: the order within a step is kept
    return out


def write_jsonl(path, events):
    with open(path, "w", encoding="utf-8") as f:
        for e in events:
            f.write(json.dumps(e, sort_keys=True) + "\n")
