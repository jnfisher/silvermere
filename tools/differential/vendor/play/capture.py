#!/usr/bin/env python3
# Copied from the project's MBBSEmu test harness, then edited here (connect offset taken before connecting); see vendor/SOURCE.md. Stdlib only.
"""
Scripted session recorder: replays a capture script against an MBBSEmu instance through
bridge.py's telnet handling and writes a byte-exact capture of what the server sent.

  capture.py tests/captures/fight-arena            run the scenario's script.json, write the capture
  capture.py DIR --port 2324                       against another instance
  capture.py DIR --readme                          only regenerate README.md from the files present

Writes into DIR (format: tests/captures/README.md):
  raw.bin      every byte the server sent during the scenario, telnet negotiation removed,
               nothing else changed (CP437 + ANSI, CR/LF as sent)
  plain.txt    patterns.raw_to_lines(raw.bin), with a ">>> 'cmd'" line inserted before the line
               in which each send happened
  script.json  the script, each step given a "result" (byte offset, seconds waited, matched)
  README.md    what the scenario shows, character, date, DLL hash, behaviours exercised, mask hints and the
               lines the pattern registry does not classify

The session's own log (logs/<ts>-capture-<scenario>/ next to this script, not committed) holds the same
bytes; the capture is cut out of it from the scenario's first connect.
"""
import argparse
import datetime
import hashlib
import json
import os
import re
import socket
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import bridge  # noqa: E402
import patterns  # noqa: E402

NAWS = 31
CURSOR_RE = re.compile(r"\x1b\[(\d+);(\d+)[fH]")
DLL = os.path.join(HERE, "..", "run", "orig", "WCCMMUD", "WCCMMUD.DLL")
PROMPTS = r"\[HP=-?\d+|\[MAJORMUD\]:|Make your selection|Username:|Password:"


class CaptureSession(bridge.Session):
    """bridge.Session that can answer DO NAWS with a window size (the only option the
    line-mode editor scenario needs); every other option is refused as bridge.py does."""

    def __init__(self, port, logdir, naws=None):
        self.naws = naws
        super().__init__(port, logdir)

    def strip_telnet(self, data):
        if not self.naws:
            return super().strip_telnet(data)
        out = bytearray()
        for b in data:
            if self.iac_state == bridge.DO and b == NAWS:
                w, h = self.naws
                self.sock.sendall(bytes([bridge.IAC, bridge.WILL, NAWS, bridge.IAC, bridge.SB, NAWS,
                                         w >> 8, w & 255, h >> 8, h & 255, bridge.IAC, bridge.SE]))
                self.iac_state = 0
            else:
                out += super().strip_telnet(bytes([b]))
        return bytes(out)


unescape = bridge.unescape   # \e \r \n \t escapes in "send"


class Runner:
    def __init__(self, script, port, logdir):
        self.script = script
        self.port = port
        self.logdir = logdir
        self.chunks = []        # text of finished connections (cp437-decoded, 1 char = 1 byte)
        self.sess = None
        self.base = 0           # length of earlier connections' text
        self.vars = {}

    # ---- the stream ------------------------------------------------------------------------
    def connect(self):
        if self.sess:
            self.close()
        naws = self.script.get("naws")
        self.sess = CaptureSession(self.port, self.logdir, naws=tuple(naws) if naws else None)
        self.sess.log_event("start", port=self.port, logdir=self.logdir)

    def close(self):
        s = self.sess
        try:
            s.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        s.sock.close()
        t0 = time.time()
        while s.alive and time.time() - t0 < 5:
            time.sleep(0.05)
        with s.lock:
            self.chunks.append(s.text)
            self.base += len(s.text)
        s.alive = False
        self.sess = None

    def pos(self):
        if self.sess is None:
            return self.base
        with self.sess.lock:
            return self.base + len(self.sess.text)

    def stream(self):
        return "".join(self.chunks) + (self.sess.text if self.sess else "")

    # ---- waiting ---------------------------------------------------------------------------
    def wait(self, start, until=None, until_cursor=None, quiet=1.0, timeout=10.0):
        """Wait for output after stream offset `start`: until the regex matches its plain text,
        or the last cursor position is `until_cursor` (and 0.3 s pass without output), or
        `quiet` seconds pass without output.  Returns (matched, seconds waited)."""
        rx = re.compile(until) if until else None
        t0 = time.time()
        s = self.sess
        local = start - self.base
        with s.lock:
            while True:
                new = s.text[local:]
                now = time.time()
                if rx and rx.search(bridge.plain(new)):
                    return True, now - t0
                if until_cursor:
                    cur = CURSOR_RE.findall(new)
                    if cur and ";".join(cur[-1]) == until_cursor and now - s.last_rx >= 0.3:
                        return True, now - t0
                if not rx and not until_cursor and now - max(s.last_rx, t0) >= quiet:
                    return True, now - t0
                if not s.alive:   # the server hung up: a quiet wait has ended normally
                    return not rx and not until_cursor, now - t0
                if now - t0 >= timeout:
                    return False, now - t0
                s.lock.wait(0.05)

    # ---- steps -----------------------------------------------------------------------------
    def fmt(self, s):
        for k, v in self.vars.items():
            s = s.replace("{" + k + "}", v)
        return s

    def run(self):
        steps = self.script["steps"]
        for i, st in enumerate(steps):
            res = self.step(st)
            st["result"] = res
            tag = st.get("send", st.get("op", ""))
            print(f"[{i:3}] {tag!r:28} {res}", flush=True)
            if not res.get("matched", True) and st.get("required"):
                sys.exit(f"step {i} did not match: {st}")
        if self.sess:
            # anything still arriving (an exit hang-up) belongs to the capture
            self.wait(self.pos(), quiet=1.0, timeout=5)
            self.close()

    def step(self, st):
        op = st.get("op", "send")
        if st.get("if_unset") and st["if_unset"] in self.vars:
            return {"skipped": True}
        if st.get("skip_if") and re.search(st["skip_if"], bridge.plain(self.stream())):
            return {"skipped": True}
        if op == "connect":
            start = self.pos()   # before connecting: a banner sent at once must not be missed
            self.connect()
            ok, w = self.wait(start, until=st.get("until"), quiet=st.get("wait", 1.0),
                              timeout=st.get("timeout", 10))
            return {"offset": start, "waited": round(w, 2), "matched": ok}
        if op == "disconnect":
            off = self.pos()
            self.close()
            return {"offset": off}
        if op == "sleep" and self.sess is None:
            time.sleep(st["seconds"])
            return {"offset": self.pos(), "waited": st["seconds"]}
        if op == "sleep":
            start = self.pos()
            until = self.fmt(st["until"]) if st.get("until") else None
            ok, w = self.wait(start, until=until, quiet=st["seconds"], timeout=st["seconds"])
            res = {"offset": start, "waited": round(w, 2)}
            if st.get("until"):
                res["matched"] = ok
            self.set_vars(st, start, res)
            return res
        # send
        text = self.fmt(unescape(st["send"]))
        raw = st.get("raw", False)
        off = self.pos()
        s = self.sess
        s.sock.sendall(text.encode("cp437") + (b"" if raw else b"\r"))
        s.transcript.write(f"\n>>> {text!r}\n")
        s.log_event("send", text=text, raw=raw)
        res = {"offset": off}
        if text != unescape(st["send"]):
            res["sent"] = text
        if st.get("nowait"):
            return res
        until = self.fmt(st["until"]) if st.get("until") else None
        ok, w = self.wait(off, until=until, until_cursor=st.get("until_cursor"),
                          quiet=st.get("wait", 1.0), timeout=st.get("timeout", 10))
        res.update(waited=round(w, 2), matched=ok)
        self.set_vars(st, off, res)
        return res

    def set_vars(self, st, off, res):
        """"set": {name: regex}: group 1 of the last match in this step's output becomes {name}
        in later sends and "until" regexes."""
        out = bridge.plain(self.stream()[off:])
        for k, rx in (st.get("set") or {}).items():
            ms = list(re.finditer(rx, out))
            if ms:
                self.vars[k] = ms[-1].group(1)
                res.setdefault("vars", {})[k] = ms[-1].group(1)


# ---- outputs -------------------------------------------------------------------------------
def marker(text):
    return f">>> {text!r}"


def sends(script):
    """(offset, marker text) of every send step that ran."""
    out = []
    for st in script["steps"]:
        if st.get("op", "send") == "send" and "offset" in st.get("result", {}):
            text = st["result"].get("sent", unescape(st["send"]))
            out.append((st["result"]["offset"], text))
    return out


def plain_with_markers(raw, script):
    """raw_to_lines(raw) with a marker line before the line in which each send happened."""
    lines = patterns.raw_to_lines(raw)
    at = {}
    for off, text in sends(script):
        k = len(patterns.raw_to_lines(raw[:off])) - 1   # index of the (partial) line at off
        at.setdefault(k, []).append(marker(text))
    out = []
    for i, line in enumerate(lines):
        out.extend(at.pop(i, []))
        out.append(line)
    for k in sorted(at):
        out.extend(at[k])
    return out


def strip_markers(lines, script):
    """plain.txt lines minus the markers (the k-th marker line of each send, in order)."""
    want = [marker(t) for _, t in sends(script)]
    out, j = [], 0
    for line in lines:
        if j < len(want) and line == want[j]:
            j += 1
            continue
        out.append(line)
    return out, j


def classify(plain_lines, reg):
    """Unknown lines when the registry reads plain.txt (markers become sent markers)."""
    text = []
    for line in plain_lines:
        m = re.match(r">>> (['\"])(.*)\1$", line)
        text.append(patterns.sent_marker(m.group(2)) if m else line)
    return [e.line.strip() for e in reg.classify("\n".join(text), raw=False) if e.name == "unknown"]


def dll_hash():
    try:
        with open(DLL, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def readme(d, script, raw, unknown_data, unknown_static):
    c = script.get("character", {})
    pf = script.get("behaviours", [])
    lines = [f"# Capture: {script['scenario']}", "", script.get("description", "").strip(), ""]
    lines += ["| | |", "|---|---|",
              f"| Server | original `WCCMMUD.DLL` (sha256 `{script.get('dll_sha256', '?')}`) under MBBSEmu "
              f"v1.0-alpha-090826, telnet {script.get('port', 2323)} |",
              f"| Captured | {script.get('captured', '?')} |",
              f"| Terminal | " + (f"NAWS {script['naws'][0]}x{script['naws'][1]}" if script.get("naws")
                                  else "no NAWS answer (MBBSEmu default 80x24, ANSI on)") + " |",
              f"| Character | " + ", ".join(f"{k} {v}" for k, v in c.items() if k != "password") + " |",
              f"| raw.bin | {len(raw)} bytes, {raw.count(b'\x1b[79D')} `ESC[79D`, "
              f"{raw.count(b'\x01')} `\\x01` |",
              f"| Steps | {len(script['steps'])} ({len(sends(script))} sends) |", ""]
    if script.get("notes"):
        lines += ["## Notes", ""] + [f"- {n}" for n in script["notes"]] + [""]
    lines += ["## Behaviours exercised", ""]
    for why in pf:
        lines.append(f"- {why}")
    if not pf:
        lines.append("None: this scenario is MBBSEmu's account dialogue and the MajorMUD menu.")
    lines.append("")
    if script.get("mask"):
        lines += ["## Mask hints", "", "Fields the comparer masks before a byte comparison "
                  "(also in `script.json` `mask`):", ""]
        for m in script["mask"]:
            lines.append(f"- `{m['regex']}`: {m.get('why', '')}")
        lines.append("")
    lines += ["## Unclassified lines", "",
              "Lines of `plain.txt` the pattern registry reports as `unknown`. "
              "`tests/test_captures.py` allows exactly these.", "",
              "With game data (`Registry.load()` with a staged `test/run/built`):", "", "```"]
    lines += sorted(set(unknown_data)) or ["(none)"]
    lines += ["```", "", "Additionally without game data (bare checkout, static patterns only):", "", "```"]
    lines += sorted(set(unknown_static) - set(unknown_data)) or ["(none)"]
    lines += ["```", ""]
    with open(os.path.join(d, "README.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def write_outputs(d, script, raw):
    with open(os.path.join(d, "raw.bin"), "wb") as f:
        f.write(raw)
    pl = plain_with_markers(raw, script)
    with open(os.path.join(d, "plain.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(pl))
    with open(os.path.join(d, "script.json"), "w", encoding="utf-8") as f:
        json.dump(script, f, indent=1, ensure_ascii=False)
        f.write("\n")
    write_readme(d, script, raw, pl)


def write_readme(d, script, raw, pl):
    reg_static = patterns.Registry.load(data=False)
    try:
        import mmud_data as md
        reg_data = patterns.Registry.load(world=md.World())
        unknown_data = classify(pl, reg_data)
    except Exception as e:  # noqa: BLE001
        sys.exit(f"capture.py needs the staged game data for the README ({e})")
    readme(d, script, raw, unknown_data, classify(pl, reg_static))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dir", help="scenario directory holding script.json")
    ap.add_argument("--port", type=int, default=None)
    ap.add_argument("--readme", action="store_true", help="only regenerate README.md")
    a = ap.parse_args()
    d = os.path.abspath(a.dir)
    with open(os.path.join(d, "script.json"), encoding="utf-8") as f:
        script = json.load(f)
    if a.readme:
        with open(os.path.join(d, "raw.bin"), "rb") as f:
            raw = f.read()
        with open(os.path.join(d, "plain.txt"), encoding="utf-8") as f:
            pl = f.read().split("\n")
        write_readme(d, script, raw, pl)
        return
    port = a.port or script.get("port", 2323)
    script["port"] = port
    logdir = os.path.join(bridge.LOGROOT, time.strftime("%Y%m%d-%H%M%S") + "-capture-" + script["scenario"])
    r = Runner(script, port, logdir)
    for st in script["steps"]:
        st.pop("result", None)
    if script["steps"][0].get("op") != "connect":
        script["steps"].insert(0, {"op": "connect", "until": "Username:", "timeout": 20})
    r.run()
    raw = r.stream().encode("cp437")
    script["captured"] = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    script["dll_sha256"] = dll_hash()
    write_outputs(d, script, raw)
    print(f"{d}: {len(raw)} bytes; log {logdir}")


if __name__ == "__main__":
    main()
