#!/usr/bin/env python3
# Copied from the project's MBBSEmu test harness; edit here; see vendor/SOURCE.md. Stdlib only.
"""
Play bridge: one persistent telnet session to an MBBSEmu instance that an agent drives one
command at a time while a human watches live.

  bridge.py start [--port 2324] [--name NAME] [--viewer TMUXNAME]
                                                start the session daemon + the tmux viewer "play"
  bridge.py send TEXT [--wait S] [--until RE] [--timeout S] [--raw]
                                                send a line (or raw keys with --raw; \\e \\t \\r
                                                escapes allowed) and print the plain text that
                                                came back
  bridge.py read [--wait S]                     print output that arrived since the last read
  bridge.py note TEXT                           show a highlighted narration line in the viewer
  bridge.py status | stop

Watch:  tmux attach -r -t play        (read-only; detach with Ctrl-b d)

Logs go to logs/<timestamp>-<name>/ next to this script:
  raw.bin         bytes from the server, telnet negotiation removed (CP437 + ANSI)
  view.ans        what the viewer shows: UTF-8 + ANSI, with narration lines (cat it to replay)
  transcript.txt  plain text: commands, notes, responses
  events.jsonl    one JSON object per command / note / response
"""
import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
LOGROOT = os.path.join(HERE, "logs")


def set_session(name):
    """Select a session: "" is the original single session (bridge.sock, tmux "play",
    logs/live.ans); a name gives bridge-<name>.sock, tmux "play-<name>", logs/live-<name>.ans.
    The name travels to the daemon and to bots through $BRIDGE_SESSION."""
    global SESSION, SOCK, TMUX, LIVE
    SESSION = name or ""
    os.environ["BRIDGE_SESSION"] = SESSION
    sfx = f"-{SESSION}" if SESSION else ""
    SOCK = os.path.join(HERE, f"bridge{sfx}.sock")
    TMUX = f"play{sfx}"
    LIVE = os.path.join(LOGROOT, f"live{sfx}.ans")


set_session(os.environ.get("BRIDGE_SESSION", ""))

IAC, DONT, DO, WONT, WILL, SB, SE = 255, 254, 253, 252, 251, 250, 240
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\x1b.")
NOTE_STYLE = "\x1b[0m\x1b[1;30;43m"  # black on yellow: stands out from the game's colours


def plain(text):
    """ANSI and the game's spinner/backspace noise removed, line ends normalised."""
    text = ANSI_RE.sub("", text)
    prev = None
    while prev != text:  # apply backspaces (the name-check spinner uses them)
        prev, text = text, re.sub("[^\n\x08]\x08", "", text)
    text = text.replace("\x08", "")
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return re.sub(r"\n{3,}", "\n\n", text)


class Session:
    def __init__(self, port, logdir):
        self.sock = socket.create_connection(("127.0.0.1", port))
        self.lock = threading.Condition()
        self.text = ""           # everything received, decoded (ANSI kept)
        self.read_pos = 0        # how far the agent has read
        self.last_rx = time.time()
        self.alive = True
        self.iac_state = 0
        os.makedirs(logdir, exist_ok=True)
        self.raw = open(os.path.join(logdir, "raw.bin"), "ab", buffering=0)
        self.view = open(os.path.join(logdir, "view.ans"), "a", encoding="utf-8", buffering=1)
        self.transcript = open(os.path.join(logdir, "transcript.txt"), "a", encoding="utf-8", buffering=1)
        self.events = open(os.path.join(logdir, "events.jsonl"), "a", encoding="utf-8", buffering=1)
        threading.Thread(target=self.reader, daemon=True).start()

    def strip_telnet(self, data):
        """Remove telnet commands (state kept across chunks); refuse every option offered."""
        out = bytearray()
        for b in data:
            s = self.iac_state
            if s == 0:
                if b == IAC:
                    self.iac_state = 1
                else:
                    out.append(b)
            elif s == 1:  # after IAC
                if b == IAC:
                    out.append(IAC); self.iac_state = 0
                elif b in (DO, DONT, WILL, WONT):
                    self.iac_state = b
                elif b == SB:
                    self.iac_state = SB
                else:
                    self.iac_state = 0
            elif s in (DO, DONT, WILL, WONT):  # option byte
                if s == DO:
                    self.sock.sendall(bytes([IAC, WONT, b]))
                self.iac_state = 0
            elif s == SB:
                if b == IAC:
                    self.iac_state = -1
            elif s == -1:  # IAC inside subnegotiation
                self.iac_state = 0 if b == SE else SB
        return bytes(out)

    def reader(self):
        while self.alive:
            try:
                data = self.sock.recv(65536)
            except OSError:
                data = b""
            if not data:
                self.alive = False
                self.log_event("disconnected")
                with self.lock:
                    self.lock.notify_all()
                return
            data = self.strip_telnet(data)
            if not data:
                continue
            self.raw.write(data)
            t = data.decode("cp437")
            self.view.write(t)
            with self.lock:
                self.text += t
                self.last_rx = time.time()
                self.lock.notify_all()

    def log_event(self, kind, **kw):
        self.events.write(json.dumps({"t": time.strftime("%H:%M:%S"), "kind": kind, **kw}) + "\n")

    def collect(self, wait, until, timeout):
        """New output since the last read: returns when `until` matches, or after `wait` seconds
        of quiet, or at `timeout`."""
        start = time.time()
        rx = re.compile(until) if until else None
        with self.lock:
            while True:
                new = self.text[self.read_pos:]
                now = time.time()
                if rx and rx.search(plain(new)):
                    break
                if not rx and new and now - self.last_rx >= wait:
                    break
                if not rx and not new and now - start >= wait:
                    break
                if now - start >= timeout or not self.alive:
                    break
                self.lock.wait(0.1)
            self.read_pos = len(self.text)
        out = plain(new)
        self.transcript.write(out)
        self.log_event("output", text=out)
        return out

    def send(self, text, raw, **kw):
        """(reply, skipped): the reply starts after anything unread, which comes back
        separately as `skipped` (a telepath or a late line between commands) and is logged."""
        with self.lock:
            skipped = plain(self.text[self.read_pos:])
            self.read_pos = len(self.text)
        if skipped.strip():
            self.transcript.write(skipped)
            self.log_event("output", text=skipped, skipped=True)
        payload = text.encode("cp437") + (b"" if raw else b"\r")
        self.sock.sendall(payload)
        self.transcript.write(f"\n>>> {text!r}\n")
        self.log_event("send", text=text, raw=raw)
        return self.collect(**kw), skipped

    def note(self, text):
        line = f"\r\n{NOTE_STYLE} » {text} \x1b[0m\r\n"
        self.view.write(line)
        self.transcript.write(f"\n### {text}\n")
        self.log_event("note", text=text)


def serve(port, logdir):
    sess = Session(port, logdir)
    if os.path.exists(SOCK):
        os.unlink(SOCK)
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(SOCK)
    srv.listen(8)
    sess.log_event("start", port=port, logdir=logdir)
    while True:
        conn, _ = srv.accept()
        try:
            with conn:
                req = json.loads(conn.makefile().readline())
                op = req.pop("op")
                if op == "stop":
                    conn.sendall(b'{"ok": true}\n')
                    sess.alive = False
                    sess.sock.close()
                    os.unlink(SOCK)
                    return
                if op == "send":
                    out, skipped = sess.send(**req)
                    res = {"text": out, "skipped": skipped, "alive": sess.alive}
                elif op == "read":
                    res = {"text": sess.collect(**req), "alive": sess.alive}
                elif op == "note":
                    sess.note(req["text"]); res = {"ok": True}
                else:
                    res = {"alive": sess.alive, "logdir": logdir, "port": port}
                conn.sendall((json.dumps(res) + "\n").encode())
        except Exception as e:  # noqa: BLE001
            # a client that went away mid-request, or sent a bad request, must not take the
            # session (and the character's connection) down
            sess.log_event("client_error", error=repr(e))


def call(op, **kw):
    # the daemon serves one request at a time, so a busy daemon can refuse briefly: retry
    for attempt in range(40):
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            c.connect(SOCK)
            break
        except OSError:
            c.close()
            if not os.path.exists(SOCK) or attempt == 39:
                sys.exit("bridge not running (bridge.py start)")
            time.sleep(0.5)
    c.sendall((json.dumps({"op": op, **kw}) + "\n").encode())
    return json.loads(c.makefile().readline())


def unescape(s):
    return s.replace("\\e", "\x1b").replace("\\t", "\t").replace("\\r", "\r").replace("\\n", "\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--session", default=None,
                    help="session name for running several characters (default: the original one)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("start"); p.add_argument("--port", type=int, default=2324)
    p.add_argument("--viewer", default=None, help="tmux session name for the viewer (default play[-SESSION])")
    p.add_argument("--name", default="session")
    p = sub.add_parser("serve"); p.add_argument("--port", type=int); p.add_argument("--logdir")
    p = sub.add_parser("send"); p.add_argument("text"); p.add_argument("--raw", action="store_true")
    p.add_argument("--wait", type=float, default=1.0); p.add_argument("--until")
    p.add_argument("--timeout", type=float, default=10.0)
    p = sub.add_parser("read"); p.add_argument("--wait", type=float, default=1.0)
    p.add_argument("--until"); p.add_argument("--timeout", type=float, default=10.0)
    p = sub.add_parser("note"); p.add_argument("text")
    sub.add_parser("status"); sub.add_parser("stop")
    a = ap.parse_args()
    if a.session is not None:
        set_session(a.session)

    if a.cmd == "start":
        if os.path.exists(SOCK):
            try:
                call("status")
                sys.exit("already running (bridge.py stop first)")
            except SystemExit as e:
                if "already" in str(e):
                    raise
                os.unlink(SOCK)  # stale socket from a crashed daemon
        logdir = os.path.join(LOGROOT, time.strftime("%Y%m%d-%H%M%S") + "-" + a.name)
        os.makedirs(logdir, exist_ok=True)
        subprocess.Popen([sys.executable, os.path.abspath(__file__), "serve", "--port", str(a.port),
                          "--logdir", logdir], start_new_session=True,
                         stdout=open(os.path.join(logdir, "bridge.err"), "w"), stderr=subprocess.STDOUT)
        for _ in range(50):
            if os.path.exists(SOCK):
                break
            time.sleep(0.1)
        # the viewer follows logs/live.ans, which points at the current session, so a restarted
        # bridge doesn't disconnect someone watching
        live = LIVE
        tmp = live + ".new"
        if os.path.lexists(tmp):
            os.unlink(tmp)
        os.symlink(os.path.join(logdir, "view.ans"), tmp)
        os.replace(tmp, live)
        # uutils tail (newer Ubuntu's default) misses writes through the symlink; prefer GNU's
        tail = shutil.which("gnutail") or "tail"
        view = a.viewer or TMUX
        if subprocess.run(["tmux", "has-session", "-t", view], stderr=subprocess.DEVNULL).returncode:
            subprocess.run(["tmux", "new-session", "-d", "-s", view, "-x", "100", "-y", "40",
                            f"printf '\\e[2J\\e[H'; {tail} -n 0 -F '{live}'"], check=True)
        print(f"bridge on port {a.port}; logs {logdir}\nwatch: tmux attach -r -t {view}")
    elif a.cmd == "serve":
        serve(a.port, a.logdir)
    elif a.cmd in ("send", "read"):
        kw = dict(wait=a.wait, until=a.until, timeout=a.timeout)
        res = call("send", text=unescape(a.text), raw=a.raw, **kw) if a.cmd == "send" else call("read", **kw)
        print(res["text"])
        if not res["alive"]:
            print("[disconnected]")
    elif a.cmd == "note":
        call("note", text=a.text)
    elif a.cmd == "status":
        print(call("status"))
    elif a.cmd == "stop":
        call("stop")


if __name__ == "__main__":
    main()
