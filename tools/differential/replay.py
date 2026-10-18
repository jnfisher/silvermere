"""Replay a T-005 script against a telnet server and record what each step received.

The discipline is the recorder's (`capture.Runner`, vendor/play/capture.py): send a
line, wait `until` the regex or `timeout`, keep the raw bytes, and cut the stream into steps
at the byte offset of each send, so text that arrives between commands is attributed to the
step it arrived in. `Player` subclasses `Runner`, so the two cannot drift.
"""
import copy
import re
import os
import re
import sys
import tempfile
import time


VENDOR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vendor")


def play_dir():
    root = os.environ.get("SILVERMERE_RESEARCH_DIR")
    d = os.path.join(root, "test", "play") if root else os.path.join(VENDOR, "play")
    if not os.path.isfile(os.path.join(d, "bridge.py")):
        sys.exit(f"{d}/bridge.py not found")
    return d


def load_play():
    """(bridge, capture, patterns) modules of the MBBSEmu test harness."""
    d = play_dir()
    if d not in sys.path:
        sys.path.insert(0, d)
    import bridge
    import capture
    import patterns
    return bridge, capture, patterns



# The text of a prompt redraw in the plain stream: the slow tick (HP and mana changed) redraws the
# prompt in the middle of a typed line, so a command's echo can
# arrive as `use scr[HP=300/MA=2]:oll of vine strike` (T-084)
PROMPT_TEXT = r"\[HP=[^\]\n]*\]:"


def split_tolerant(text):
    """A regex for the echo of `text`, which a prompt redraw may split between any two
    characters (`normalise.rejoin_split_lines` rejoins the bytes the same way)."""
    return (r"(?:" + PROMPT_TEXT + r")?").join(re.escape(c) for c in text)


def anchor_on_echo(steps):
    """`echo_anchored` scripts (T-057): each in-realm step waits for its own command's echo
    and then its `until`. A prompt the original redraws late, or a second prompt after a
    command, otherwise satisfies the next step's wait before its command is even echoed, and
    every later step's output on that side shifts one step back. Steps before the realm entry
    (the BBS login, a password is not echoed) and bare Enters keep their `until`."""
    entered = False
    for st in steps:
        until = st.get("until") or ""
        in_realm = "HP=" in until
        if entered and in_realm and st.get("op", "send") == "send" and st.get("send") \
                and not st.get("raw") and not st.get("no_echo"):
            st["until"] = "(?i:" + split_tolerant(st["send"]) + r")(?s:.*?)(?:" + until + ")"
            st["_until"] = until
        entered = entered or in_realm

class StepCapture:
    """What one script step received: `raw` is the bytes between this step's send and the next."""

    def __init__(self, index, op, send, offset, raw, matched=True, until=None, waited=0.0,
                 error=None, retries=0, burst_retries=0, burst_unclean=False, client="A"):
        self.index, self.op, self.send, self.offset, self.raw = index, op, send, offset, raw
        self.matched, self.until, self.waited, self.error = matched, until, waited, error
        self.may_time_out = False   # a fight-loop wait that may run out (m2)
        self.client = client   # the connection that ran the step (`"as"` in the script)
        self.retries = retries   # bare Enters resent after the original's known drop
        self.burst_retries = burst_retries   # bursts redone after a tick landed mid-burst
        self.burst_unclean = burst_unclean   # every attempt drained mid-burst: a problem
        self.walk_out_failed = False         # a drained burst's `reset` did not match: a problem
        self.echo_missing = False

    @property
    def timed_out(self):
        # a step the script marks `may_time_out` (a fight cycle's waits: no monster came, the
        # fight outlasted the wait) neither stops the run nor counts as a problem
        return self.until is not None and not self.matched and not self.may_time_out


def _until(st):
    return st.get("until") or st.get("until_cursor")


def from_capture(script, raw):
    """StepCaptures of a recorded capture (script.json with results, raw.bin)."""
    rows = []
    for i, st in enumerate(script["steps"]):
        res = st.get("result") or {}
        if "offset" in res:
            rows.append((i, st, res))
    out = []
    for n, (i, st, res) in enumerate(rows):
        end = rows[n + 1][2]["offset"] if n + 1 < len(rows) else len(raw)
        op = st.get("op", "send")
        send = res.get("sent", st.get("send")) if op == "send" else None
        out.append(StepCapture(i, op, send, res["offset"], raw[res["offset"]:end],
                               res.get("matched", True), _until(st), res.get("waited", 0.0)))
    return out


def stream_bytes(text):
    return text.encode("cp437", errors="replace")


# A bare Enter in the realm: the original (MBBSEmu) drops one sent too soon after the
# realm-entry prompt (4 of 6 tries got only the CRLF echo), and on a loaded host "too soon"
# stretches past any fixed pause. So the Enter waits until the server has been quiet for
# SETTLE seconds, and when all that came back after DROP_AFTER seconds is the echo, the
# Enter is sent again (at most MAX_RETRIES times) and the echo of the lost one is cut from
# the stream, so the step reads exactly as an Enter the server took the first time.
SETTLE = 2.0
SETTLE_CAP = 20.0
DROP_AFTER = 12.0
MAX_RETRIES = 2
DROPPED = "\r\n"


def is_bare_enter(st):
    return (st.get("op", "send") == "send" and st.get("send") == "" and not st.get("raw")
            and bool(st.get("until")) and not st.get("skip_if") and not st.get("if_unset"))


def is_burst_send(st):
    """A send typed without waiting that can go out in one write with its neighbours."""
    return (st.get("op", "send") == "send" and st.get("nowait") and not st.get("delay_before")
            and not st.get("skip_if") and not st.get("if_unset"))


def make_runner(capture):
    class FastRunner(capture.Runner):
        """`Runner` whose connect step starts counting before the connection exists, so a
        banner a fast server sends at once (silvermere) is not lost the way a slow one's
        (MBBSEmu) never is; with the settle-and-retry bare Enter and the one-write burst."""

        def __init__(self, *a, **kw):
            super().__init__(*a, **kw)
            self.cuts = []   # [start, end) stream ranges to leave out of the capture

        mark = 0      # where a wait for a message from another connection starts looking
        endmark = 0   # where this connection's last connect or send step ended

        def step(self, st):
            op = st.get("op", "send")
            # `if_set` (T-040): the step runs only when the variable is set (`if_unset`'s twin)
            if st.get("if_set") and st["if_set"] not in self.vars:
                return {"skipped": True}
            # `if_unset` may name several variables: the step is skipped when any is set
            if isinstance(st.get("if_unset"), list):
                if any(v in self.vars for v in st["if_unset"]):
                    return {"skipped": True}
                st = {k: v for k, v in st.items() if k != "if_unset"}
            if op == "sleep" and st.get("since") == "last" and st.get("until") and self.sess is not None:
                # `"since": "last"`: wait until X shows, X being a message another connection's
                # command caused. It may have arrived while the script was busy with that
                # connection, so the search starts at this connection's last send, not at the
                # instant of this step (use a text no earlier message of the run also matches).
                # The step's own offset is where the previous step of this connection ended, so
                # what arrives after that belongs to this step whether it came before or after
                # the instant the script got here (otherwise which step a message lands in
                # depends on how fast the server is).
                ok, w = self.wait(self.mark, until=self.fmt(st["until"]), quiet=st["seconds"],
                                  timeout=st["seconds"])
                return {"offset": self.endmark, "waited": round(w, 2), "matched": ok}
            if op == "send":
                self.mark = self.pos()
            res = self._step(st)
            if op in ("send", "connect"):
                self.endmark = self.pos()
            return res

        def _step(self, st):
            if st.get("op") == "connect" and not st.get("skip_if") and not st.get("if_unset"):
                start = self.pos()
                self.connect()
                ok, w = self.wait(start, until=st.get("until"), quiet=st.get("wait", 1.0),
                                  timeout=st.get("timeout", 10))
                return {"offset": start, "waited": round(w, 2), "matched": ok}
            if is_bare_enter(st) and self.sess is not None:
                return self.bare_enter(st)
            return super().step(st)

        def bare_enter(self, st):
            settle = st.get("settle", SETTLE)
            if settle:
                self.wait(self.pos(), quiet=settle, timeout=max(settle, SETTLE_CAP))
            until = self.fmt(st["until"])
            timeout = st.get("timeout", 10)
            drop_after = min(st.get("drop_after", DROP_AFTER), timeout)
            t0 = time.time()
            off = self.pos()
            self.send_raw(b"\r", "")
            retries = 0
            while True:
                left = timeout - (time.time() - t0)
                limit = drop_after if retries < MAX_RETRIES else left
                ok, _ = self.wait(off, until=until, quiet=0, timeout=max(0.0, min(limit, left)))
                if ok or time.time() - t0 >= timeout or retries >= MAX_RETRIES:
                    break
                with self.sess.lock:
                    end = self.pos_locked()
                    dropped = self.stream_locked(off, end) == DROPPED
                    if dropped:
                        self.sess.sock.sendall(b"\r")
                if not dropped:
                    # something other than the bare echo came: a slow answer, not a drop
                    ok, _ = self.wait(off, until=until, quiet=0, timeout=max(0.0, left - limit))
                    break
                self.cuts.append((off, end))
                self.sess.log_event("retry", text="", dropped=DROPPED)
                retries += 1
                off = end
            res = {"offset": off, "waited": round(time.time() - t0, 2), "matched": ok}
            if retries:
                res["retries"] = retries
            self.set_vars(st, off, res)
            return res

        def pos_locked(self):
            return self.base + len(self.sess.text)

        def stream_locked(self, start, end):
            return self.sess.text[start - self.base:end - self.base]

        def send_raw(self, payload, text):
            s = self.sess
            s.sock.sendall(payload)
            s.transcript.write(f"\n>>> {text!r}\n")
            s.log_event("send", text=text, raw=False)

        def burst(self, steps):
            """Sends typed without waiting, as one write: the server's flood control counts
            the lines already queued, so how many ticks pass while the burst arrives decides
            the outcome, and fifteen separate writes from a starved client let ticks in."""
            texts = [self.fmt(capture.unescape(st["send"])) for st in steps]
            payload = b"".join(t.encode("cp437") + (b"" if st.get("raw") else b"\r")
                               for t, st in zip(texts, steps))
            off = self.pos()
            self.sess.sock.sendall(payload)
            out = []
            for st, text in zip(steps, texts):
                self.sess.transcript.write(f"\n>>> {text!r}\n")
                self.sess.log_event("send", text=text, raw=st.get("raw", False))
                res = {"offset": off}
                if text != capture.unescape(st["send"]):
                    res["sent"] = text
                out.append(res)
            return out

    return FastRunner


def burst_drained(text, check, plain):
    """Whether a tick drained a queued line while the burst was still arriving, read from the
    side's own output since the burst was sent: fewer warning lines ("slow down", "too
    quickly") came before the first queued line's echo than flood control prints for a burst
    that queues whole (`expect_warnings`).

    The count is the original's: `add_delayed_command` walks the queue and warns
    from PENDWARN (8 links, nine lines waiting) and drops from PENDMAX (12 links, thirteen
    waiting) with both texts. For m1's burst (`search` runs at once, then fourteen `exp`
    queue) that is lines 10-14 warning and line 14 dropped: 5 + 1 = 6 lines, all printed
    before the next fast tick runs the first queued line. A tick in
    the middle runs a line early, so later lines see one fewer waiting and fewer warnings
    come before that first echo."""
    warn, queued = re.compile(check["warnings"]), re.compile(check["queued"])
    n = 0
    for line in plain(text).split("\n"):
        if queued.search(line):
            break
        if warn.search(line):
            n += 1
    return n < check["expect_warnings"]


def cut_ranges(raw, offsets, cuts):
    """`raw` without the [start, end) ranges in `cuts`, and `offsets` moved to match."""
    if not cuts:
        return raw, list(offsets)
    cuts = sorted(cuts)
    out, prev = bytearray(), 0
    for a, b in cuts:
        out += raw[prev:a]
        prev = max(prev, b)
    out += raw[prev:]

    def move(o):
        return o - sum(min(b, max(o, a)) - a for a, b in cuts if o > a)
    return bytes(out), [move(o) for o in offsets]


# MBBSEmu's nightly cleanup warning (Cleanup.Time; five broadcasts, then every session is
# dropped and the module reloaded, HostProcess/MbbsHost.cs): m4 run 38997202711 met it at 03:00
# UTC. A run that sees it stops (`stop_reached`) and a scenario names it (differential.py).
CLEANUP_RX = re.compile(r'server will be shutting down in \d+ minutes? for the nightly '
                        r'"auto-cleanup"')


def stop_reached(stop, runner, plain, now=None):
    """None while a statistical run should go on cycling, else why it stops (T-040; John,
    2026-10-16: the comparisons stop on sample count, with a hard cap of time per side).

    `stop`: {"minutes": the cap, "count": a regex counted in this connection's plain text,
    "at_least": the sample, "pool_file": this process's count file, "pool_glob": the files of
    every process of this side (parallel sessions add up)}. The clock starts at the first check.
    Returns {"reason": "count"|"minutes"|"cleanup", "count", "elapsed"}: "cleanup" when the
    original has warned of its nightly cleanup (`CLEANUP_RX`), so the run logs off with the
    sample it has rather than lose the session."""
    now = time.time() if now is None else now
    stop.setdefault("_t0", now)
    elapsed = now - stop["_t0"]
    # `also`: more samples the run needs ([{count, at_least}], the monster's swings): it stops
    # on count only when every one is in
    wants = ([(stop["count"], stop.get("at_least", 0))] if stop.get("count") else [])
    wants += [(w["count"], w.get("at_least", 0)) for w in stop.get("also", [])]
    text = plain(runner.stream())
    counts = [len(re.findall(rx, text, re.M)) for rx, _ in wants]
    totals = list(counts)
    if stop.get("pool_file"):
        try:
            with open(stop["pool_file"], "w", encoding="utf-8") as f:
                f.write(" ".join(str(c) for c in counts))
        except OSError:
            pass
        import glob
        totals = [0] * len(counts)
        for path in glob.glob(stop.get("pool_glob") or stop["pool_file"]):
            try:
                with open(path, encoding="utf-8") as f:
                    vals = [int(v) for v in f.read().split()]
            except (OSError, ValueError):
                continue
            for k, v in enumerate(vals[:len(totals)]):
                totals[k] += v
    info = {"count": counts[0] if counts else 0, "pooled": totals[0] if totals else 0,
            "elapsed": round(elapsed, 1)}
    if len(counts) > 1:
        info.update(also=counts[1:], also_pooled=totals[1:])
    if CLEANUP_RX.search(text):
        return dict(info, reason="cleanup")
    if wants and all(t >= need for t, (_, need) in zip(totals, wants) if need):
        return dict(info, reason="count")
    if stop.get("minutes") and elapsed >= stop["minutes"] * 60:
        return dict(info, reason="minutes")
    return None


class Player:
    """Runs scripts against 127.0.0.1:`port`; `run_script(script)` returns [StepCapture]."""

    def __init__(self, port, host="127.0.0.1", timeout_cap=None):
        self.port, self.host, self.timeout_cap = port, host, timeout_cap
        self.stop_info = None   # why a `stop` run stopped cycling (stop_reached)
        self.bridge, self.capture, _ = load_play()
        self.logdir = tempfile.mkdtemp(prefix="differential-")

    def log_off(self, runner):
        """After an aborted script: leave the realm the way a player does (`x`, then the BBS
        logoff) so the character is not marked as having dropped the connection, which the next
        scenario would meet as the hang-up punishment."""
        try:
            for send, until, timeout in (("x", r"\[MAJORMUD\]:", 40), ("X", "Make your selection", 15),
                                         ("X", "Are you sure", 15), ("Y", None, 5)):
                st = {"op": "send", "send": send, "timeout": timeout, "wait": 1.0}
                if until:
                    st["until"] = until
                runner.step(st)
        except OSError:
            pass

    def burst_was_drained(self, runner, check, out, attempt):
        """After the burst's last step: whether a tick drained a line mid-burst on this side
        (`burst_drained` on the output since the burst's first send)."""
        first = next((sc for sc in out[attempt[0]:] if sc.index == check["from"] + 1), None)
        if first is None or runner.sess is None:
            return False
        return burst_drained(runner.stream()[first.offset:runner.pos()], check, self.bridge.plain)

    def walk_out(self, runner, check, attempt):
        """Runs the `reset` steps and leaves the discarded attempt and the walk out (stream
        [attempt start, now)) out of the capture; False when a reset step did not match."""
        for st in check["reset"]:
            if not runner.step(copy.deepcopy(st)).get("matched", True):
                return False
        runner.cuts.append((attempt[1], runner.pos()))
        runner.sess.log_event("burst-retry", cut=[attempt[1], runner.pos()])
        return True

    def step_paged(self, runner, st, pager, answers=20):
        """One step; with a script's `answer_pager` regex, a step marked `pager` also answers
        a host's page mode (`btuxnf`: "Hit any key to continue...", the next key resumes the
        output) with a key each time it pauses, until the step's own `until` follows the last
        pause. The answers belong to the step's capture. (MBBSEmu at the pinned tag ignores
        btuxnf, so the live original never pauses.)"""
        if not (pager and st.get("pager") and st.get("until")):
            return runner.step(st)
        either = f"(?:{st['until']})|(?:{pager})"
        res = runner.step(dict(st, until=either))
        if "offset" not in res:
            return res
        cont = st.get("_until", st["until"])
        for _ in range(answers):
            text = self.bridge.plain(runner.stream()[res["offset"]:])
            pauses = list(re.finditer(pager, text))
            if not pauses:
                break
            if re.search(cont, text[pauses[-1].end():]):
                res["matched"] = True
                break
            key = runner.step({"op": "send", "send": " ", "raw": True,
                               "until": f"(?:{cont})|(?:{pager})", "timeout": st.get("timeout", 10)})
            res["matched"] = key.get("matched", False)
            res["paged"] = res.get("paged", 0) + 1
        return res

    def run_script(self, script, stop_on_timeout=True):
        script = copy.deepcopy(script)
        if self.timeout_cap:
            for st in script["steps"]:
                st["timeout"] = min(st.get("timeout", 10), self.timeout_cap)
        # a bare Enter settles first and is retried on the original's drop (`bare_enter`), and
        # the original answers a room redraw late now and then: allow 30 s
        for st in script["steps"]:
            if is_bare_enter(st):
                st["timeout"] = max(st.get("timeout", 10), 30)
        if script.get("echo_anchored"):
            anchor_on_echo(script["steps"])
        # A script may drive several connections: a step with `"as": "B"` runs on the
        # connection B (opened by B's own `connect` step); steps without `as` run on A. A
        # step's bytes end where the next step of the same connection begins.
        runners = {}

        def runner_of(name):
            if name not in runners:
                logdir = os.path.join(self.logdir, name)
                os.makedirs(logdir, exist_ok=True)
                runners[name] = make_runner(self.capture)(script, self.port, logdir)
            return runners[name]

        runner = runner_of("A")
        out = []
        aborted = False
        end_pos = {}
        steps = script["steps"]
        # `burst_check`: a burst typed without waiting (steps `from`..`to`) that a tick drained
        # while it was still arriving is done again (`burst_was_drained`, `walk_out`)
        check = script.get("burst_check")
        attempt = None         # (len(out), stream offset) where the current attempt started
        burst_retries, burst_unclean, walk_out_failed = 0, False, False
        try:
            i = 0
            while i < len(steps):
                st = steps[i]
                # `stop` (T-040, the m2 statistical runs): at each `stop_check` step (the start
                # of a cycle) the run stops cycling once it has its sample or its time is up,
                # and goes on at the first `after_stop` step (the logoff)
                if st.get("stop_check") and script.get("stop"):
                    info = stop_reached(script["stop"], runner_of(st.get("as", "A")), self.bridge.plain)
                    if info:
                        j = next((k for k in range(i, len(steps)) if steps[k].get("after_stop")),
                                 len(steps))
                        self.stop_info = info
                        i = j
                        continue
                client = st.get("as", "A")
                runner = runner_of(client)
                if check and i == check["from"]:
                    attempt = (len(out), runner.pos())
                if st.get("delay_before"):
                    time.sleep(st["delay_before"])
                before = runner.pos()
                j = i
                while j < len(steps) and is_burst_send(steps[j]):
                    j += 1
                if j - i > 1 and runner.sess is not None:
                    try:
                        results = runner.burst(steps[i:j])
                    except OSError as e:
                        out.append(StepCapture(i, "send", st.get("send"), before, b"", False,
                                               "connection", error=str(e), client=client))
                        break
                    for k, res in zip(range(i, j), results):
                        out.append(StepCapture(k, "send", res.get("sent", steps[k].get("send")),
                                               res["offset"], b"", client=client))
                    i = j
                    continue
                i += 1
                try:
                    res = self.step_paged(runner, st, script.get("answer_pager"))
                    # an echo split by another line (a monster's message in the middle of the
                    # typed command) never matches the anchored wait: fall back to the step's
                    # own `until`, and say so in the capture (`echo_missing`)
                    if st.get("_until") and not res.get("matched", True) and "offset" in res:
                        text = self.bridge.plain(runner.stream()[res["offset"]:])
                        if re.search(st["_until"], text):
                            res["matched"] = True
                            res["echo_missing"] = True
                    # `settle` (T-057): after an in-realm step's wait, until `settle` seconds
                    # pass without output, so the rest of its reply (a message the original
                    # sends just after the prompt) stays in this step and not the next
                    # not in a fight step (`random`): a fight streams without a quiet half
                    # second, so the settle would take the whole fight into its first step
                    if (script.get("settle") and st.get("_until") and not st.get("random")
                            and res.get("matched", True)):
                        runner.wait(runner.pos(), quiet=script["settle"],
                                    timeout=script["settle"] + 5)
                except OSError as e:
                    out.append(StepCapture(i - 1, st.get("op", "send"), st.get("send"), before, b"",
                                           False, _until(st) or "connection", error=str(e),
                                           client=client))
                    break
                if "offset" not in res:
                    continue
                op = st.get("op", "send")
                out.append(StepCapture(i - 1, op, res.get("sent", st.get("send")) if op == "send" else None,
                                       res["offset"], b"", res.get("matched", True), _until(st),
                                       res.get("waited", 0.0), retries=res.get("retries", 0),
                                       client=client))
                out[-1].echo_missing = bool(res.get("echo_missing"))
                out[-1].may_time_out = bool(st.get("may_time_out"))
                if out[-1].timed_out and stop_on_timeout:
                    aborted = True
                    break
                if (check and i - 1 == check["to"] and attempt is not None
                        and self.burst_was_drained(runner, check, out, attempt)):
                    # bounded: after `max_retries` the last attempt is kept, marked unclean
                    # (a problem), and compared as it is
                    if burst_retries >= check.get("max_retries", 2):
                        burst_unclean = True
                    elif self.walk_out(runner, check, attempt):
                        del out[attempt[0]:]
                        burst_retries += 1
                        i = check["from"]
                    else:
                        walk_out_failed = True
            if check:
                for sc in out:
                    if sc.index == check["from"]:
                        sc.burst_retries, sc.burst_unclean = burst_retries, burst_unclean
                        sc.walk_out_failed = walk_out_failed
            for name, rn in runners.items():
                if rn.sess:
                    rn.wait(rn.pos(), quiet=1.0, timeout=5)
                end_pos[name] = rn.pos()
            for name, rn in runners.items():
                if aborted and rn.sess and "[HP=" in rn.stream():
                    # `on_abort` (T-040): steps that undo what the script changed (a sysop's
                    # tweaks) before the logoff
                    for st in script.get("on_abort", []) if name == "A" else []:
                        try:
                            rn.step(dict(st))
                        except OSError:
                            break
                    self.log_off(rn)   # only when the script got as far as the realm
        finally:
            for rn in runners.values():
                sess = rn.sess
                if sess:
                    rn.close()
                    for f in (sess.raw, sess.view, sess.transcript, sess.events):
                        f.close()
        for name, rn in runners.items():
            raw = stream_bytes(rn.stream())
            if name in end_pos:
                raw = raw[:end_pos[name]]   # the clean logoff after an abort is not part of any step
            mine = [sc for sc in out if sc.client == name]
            # the echo of each Enter the original dropped (`bare_enter`); cp437 is one byte a
            # char, so stream offsets are byte offsets
            raw, offsets = cut_ranges(raw, [sc.offset for sc in mine], rn.cuts)
            for sc, o in zip(mine, offsets):
                sc.offset = o
            for n, sc in enumerate(mine):
                end = mine[n + 1].offset if n + 1 < len(mine) else len(raw)
                sc.raw = raw[sc.offset:end]
        return out
