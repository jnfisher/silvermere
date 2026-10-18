"""Event diff (silvermere-testkit's `diff_events` per pair, after aligning the two lists) and
the byte comparison of deterministic steps."""
import difflib
import re

MASKED = "*"


def _key(e, value_masks=None):
    """What two events must share to be equal. An `unknown` event keeps its text in `line`, not
    in `fields`, so its value-masked text and its step are part of the key: a changed line, or
    one that moved to another step, is a difference."""
    fields = tuple(sorted((k, str(v)) for k, v in e["fields"].items()))
    if e["name"] == "unknown":
        return (e["name"], fields, _unmasked_line(e.get("line") or "", value_masks), e["step"])
    return (e["name"], fields)


def _pair(i, l, r, value_masks=None):
    """The differences of one aligned pair (same position in a changed stretch)."""
    if l["name"] != r["name"]:
        return [{"index": i, "kind": "name", "orig": l["name"], "new": r["name"],
                 "step": l["step"]}]
    out = [{"index": i, "kind": "field", "event": l["name"], "field": k, "orig": l["fields"].get(k),
            "new": r["fields"].get(k), "step": l["step"]}
           for k in sorted(set(l["fields"]) | set(r["fields"]))
           if l["fields"].get(k) != r["fields"].get(k)]
    if l["name"] == "unknown":
        ol, nl = l.get("line"), r.get("line")
        if _unmasked_line(ol or "", value_masks) != _unmasked_line(nl or "", value_masks):
            out.append({"index": i, "kind": "field", "event": "unknown", "field": "line",
                        "orig": ol, "new": nl, "step": l["step"]})
        if l["step"] != r["step"]:
            out.append({"index": i, "kind": "field", "event": "unknown", "field": "step",
                        "orig": l["step"], "new": r["step"], "step": l["step"]})
    return out


def diff_events(a, b, value_masks=None):
    """Differences between two normalised event lists: names equal and every field equal
    (a field masked on both sides compares equal). Empty list = equal.

    The lists are aligned first (difflib's longest matching blocks on name and fields, no
    junk heuristic), so one event more on one side is one `missing` difference, not a shift
    of every later pair (a live run's single extra prompt was 129 differences). Inside a
    stretch that differs on both sides the events are compared pairwise, as before, and the
    longer side's rest is `missing`. `index` is the position on the original's side (the
    new side's for an event only it has). `unknown` events compare by their text (the values
    of lines matching `value_masks` hidden, their labels kept) and their step."""
    out = []
    sm = difflib.SequenceMatcher(None, [_key(e, value_masks) for e in a],
                                 [_key(e, value_masks) for e in b],
                                 autojunk=False)
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        if op == "equal":
            continue
        n = min(i2 - i1, j2 - j1)
        for k in range(n):
            out += _pair(i1 + k, a[i1 + k], b[j1 + k], value_masks)
        for i in range(i1 + n, i2):
            out.append({"index": i, "kind": "missing", "extra_in": "orig", "event": a[i]["name"],
                        "step": a[i]["step"]})
        for j in range(j1 + n, j2):
            out.append({"index": j, "kind": "missing", "extra_in": "new", "event": b[j]["name"],
                        "step": b[j]["step"]})
    return out


def render_event(e):
    f = " ".join(f"{k}={v!r}" for k, v in sorted(e["fields"].items()))
    line = f" line={e['line']!r}" if e["name"] == "unknown" and e.get("line") is not None else ""
    return f"[{e['step']}] {e['name']}" + (f" {f}" if f else "") + line


def unified(a, b, orig="orig", new="new"):
    """Unified diff of the rendered event lists."""
    return "\n".join(difflib.unified_diff([render_event(e) for e in a], [render_event(e) for e in b],
                                          orig, new, lineterm="", n=2))


def _unmasked_line(line, value_masks):
    """`line` with the value of each matching mask replaced by `*`, its label kept: a mask is a
    regex whose first group is the label and whose match after it is the value; text after the
    match is kept and compared."""
    for rx in value_masks or []:
        m = re.match(rx, line)
        if m:
            return m.group(1) + MASKED + line[m.end():]
    return line


def unknown_extra(orig_events, new_events, value_masks=None, as_set=False):
    """`unknown` lines of the new side that the original does not also have (multiset). With
    `value_masks`, the values of the lines they match are not compared, only their labels.

    `as_set` (the sample-count scenarios, `m2.stop`, or `unknown_as_set`; T-040): a text the
    original never shows, with every digit masked. The two sides of a random run play different numbers of cycles, so the same
    unclassified echo comes more often on the side that played more (run 38998045779: `a rat`
    30 more times on new), and a statistics line such as SYSOP BUFFERS differs in its numbers
    (and their column padding) only; neither is a line the original lacks."""
    if as_set:
        seen = {_digits(_unmasked_line(e["line"], value_masks)) for e in orig_events
                if e["name"] == "unknown"}
        out, shown = [], set()
        for e in new_events:
            if e["name"] != "unknown":
                continue
            k = _digits(_unmasked_line(e["line"], value_masks))
            if k not in seen and k not in shown:
                shown.add(k)
                out.append(e["line"])
        return out
    have = {}
    for e in orig_events:
        if e["name"] == "unknown":
            k = _unmasked_line(e["line"], value_masks)
            have[k] = have.get(k, 0) + 1
    extra = []
    for e in new_events:
        if e["name"] == "unknown":
            k = _unmasked_line(e["line"], value_masks)
            if have.get(k, 0) > 0:
                have[k] -= 1
            else:
                extra.append(e["line"])
    return extra


def _digits(line):
    # digits masked, and the padding of a column whose width follows its number
    return re.sub(r"\s+", " ", re.sub(r"\d+", "#", line)).strip()


def script_mask(text, regexes):
    """Blanks the script's masks. An entry is a regex (replaced by `<masked>`) or a
    `(regex, replacement)` pair: the direction scramble (`nJ\\borth`) puts its random letter at a
    random place, so it is removed (replacement empty), not marked."""
    for rx in regexes:
        rx, repl = rx if isinstance(rx, (tuple, list)) else (rx, "<masked>")
        try:
            text = re.sub(rx, repl, text)
        except re.error:
            pass
    return text


def step_views(raw, regexes, bridge_plain, prefilter=None):
    """(plain, colour) of one step's bytes: `prefilter` (known-gap lines) and the script masks
    applied to the decoded text, then with ANSI stripped (`bridge.plain`) or kept."""
    text = raw.decode("cp437")
    if prefilter:
        text = prefilter(text)
    colour = script_mask(text, regexes)
    return bridge_plain(colour), colour


def from_echo(raw, send):
    """A step's bytes from its own command's echo on (T-057 `bytes_from_echo`): what arrives
    before it is the previous step's tail (an attack, a late prompt). Unchanged when the echo
    is not found."""
    if not send:
        return raw
    i = raw.find(send.encode("cp437", "replace"))
    return raw[i:] if i >= 0 else raw


def byte_diffs(orig_steps, new_steps, deterministic, regexes, bridge_plain, orig_filter=None,
               new_filter=None, echo_on=False, redraws=None):
    """Per deterministic step index: which views differ. Steps are compared by index; a step
    missing on one side or with a different send is a difference."""
    out = []
    o = {s.index: s for s in orig_steps}
    n = {s.index: s for s in new_steps}
    for i in sorted(deterministic):
        if i not in o:
            continue
        if i not in n:
            out.append({"step": i, "kind": "missing-in-new"})
            continue
        if o[i].send != n[i].send:
            out.append({"step": i, "kind": "different-send", "orig": o[i].send, "new": n[i].send})
            continue
        oraw, nraw = o[i].raw, n[i].raw
        if echo_on:
            oraw, nraw = from_echo(oraw, o[i].send), from_echo(nraw, n[i].send)
        op, oc = step_views(oraw, regexes, bridge_plain, orig_filter)
        np_, nc = step_views(nraw, regexes, bridge_plain, new_filter)
        if redraws is not None:
            # on the masked views: a redraw whose only change is a masked number is the
            # same prompt again (the tick's regeneration)
            co, cn = redraws.setdefault("orig", {}), redraws.setdefault("new", {})
            op, oc = collapse_prompt_redraws(op, co), collapse_prompt_redraws(oc)
            np_, nc = collapse_prompt_redraws(np_, cn), collapse_prompt_redraws(nc)
        if op != np_:
            out.append({"step": i, "kind": "plain", "send": o[i].send,
                        "diff": "\n".join(difflib.unified_diff(op.splitlines(), np_.splitlines(),
                                                               "orig", "new", lineterm="", n=1))})
        elif oc != nc:
            # the escape sequences made visible, so a report shows which colour differs
            def esc(t):
                return t.replace("\x1b", "\\e").splitlines()
            out.append({"step": i, "kind": "colour", "send": o[i].send, "_views": (oc, nc),
                        "diff": "\n".join(difflib.unified_diff(esc(oc), esc(nc), "orig", "new",
                                                               lineterm="", n=1))})
    return out


ANSI_NORMAL = "\x1b[0;37;40m"
ANSI_CLRLINE = "\x1b[79D\x1b[K"


def ansi_normal_after_echo(orig_colour, new_colour):
    """The T-083 shape: the two colour views are equal but for one ANSI_NORMAL that the new
    side has directly after the step's echo line and the original has not, or ANSI_NORMAL with
    the ANSI_CLRLINE the original clears it with. Nothing else."""
    i, j = orig_colour.find("\n"), new_colour.find("\n")
    if i < 0 or j < 0 or orig_colour[:i + 1] != new_colour[:j + 1]:
        return False
    o_rest, n_rest = orig_colour[i + 1:], new_colour[j + 1:]
    if o_rest.startswith(ANSI_NORMAL):
        return False
    if n_rest.startswith(ANSI_NORMAL) and n_rest[len(ANSI_NORMAL):] == o_rest:
        return True
    both = ANSI_NORMAL + ANSI_CLRLINE
    return (not o_rest.startswith(ANSI_CLRLINE) and n_rest.startswith(both)
            and n_rest[len(both):] == o_rest)


# a prompt redrawn with nothing between: the slow tick sends the prompt again when what it
# shows may have changed (slow_update_character's `changed`), which on the screen
# puts the same prompt over itself; the capture shows it twice
_PROMPT = r"(?:\x1b\[79D\x1b\[K)?(?:\x1b\[[0-9;]*m)*\[HP=[^\]\r\n]*\]:"
_REDRAW = re.compile(r"(" + _PROMPT + r")(?:\1)+")


def collapse_prompt_redraws(text, counts=None):
    """`text` with an exact repeat of the prompt directly after itself taken out (counted)."""
    def one(m):
        if counts is not None:
            counts["prompt_redraws"] = counts.get("prompt_redraws", 0) + len(m.group(0)) // len(m.group(1)) - 1
        return m.group(1)
    return _REDRAW.sub(one, text)
