#!/usr/bin/env python3
"""Outside-in smoke test of a running silvermere (T-108): run it from another machine.

    SMOKE_PASSWORD=... python3 tools/smoke/smoke.py HOST[:PORT] [--user NAME]

It connects over telnet, logs in through the BBS menu (or signs up with `NEW` when the
account does not exist yet), enters the realm (creating a character the first time), walks
a few rooms with `look`, `stat` and `inventory`, quits, logs back in and checks that the
character is in the room it left. With --state FILE it also checks that it enters where
the run before left it, so a restart, a reboot or a restore can go between two runs; that
tests the save at `quit`, not the SIGTERM save of a player still online. It exits 0 with a
short report, or 1 at the first failure. The password comes from SMOKE_PASSWORD, or is
asked for when that is unset; it is never printed. Python 3 standard library only. It never
trains.
"""

from __future__ import annotations

import argparse
import getpass
import os
import re
import socket
import sys
import time

IAC, DONT, DO, WONT, WILL, SB, SE, NAWS = 255, 254, 253, 252, 251, 250, 240, 31
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
PROMPT = r"\[HP=-?\d+[^\]\n]*\]:"
EXITS = re.compile(r"Obvious exits: ([^\r\n]*)")
OPPOSITE = {
    "north": "south", "south": "north", "east": "west", "west": "east",
    "northeast": "southwest", "southwest": "northeast",
    "northwest": "southeast", "southeast": "northwest", "up": "down", "down": "up",
}
# never sent as a game command: training spends the character's points (and bots never
# train); login fields (a name like "travis", a password like "trapdoor1") are not commands
FORBIDDEN = re.compile(r"^\s*tra", re.IGNORECASE)
# an escape sequence cut off at the end of a chunk, finished by the next one
PARTIAL_ANSI = re.compile(r"\x1b(?:\[[0-9;?]*)?$")


class Failure(Exception):
    """A step that did not see what it waited for."""


class Telnet:
    """A socket that answers telnet negotiation like the testkit's client: `DO NAWS` with
    79x24 (narrower than 80 columns gets the line-mode character editor), every other
    `DO` with `WONT`; `WILL` is ignored (as the testkit does) and subnegotiations skipped."""

    def __init__(self, host: str, port: int, timeout: float, wait: float = 0.0):
        self.timeout = timeout
        deadline = time.monotonic() + wait
        while True:  # a box still booting or a server still starting refuses or times out
            try:
                left = max(deadline - time.monotonic(), 0)
                self.sock = socket.create_connection((host, port), timeout=min(timeout, left) or timeout)
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(1)
        self.text = ""
        self.mark = 0
        self._state = "data"
        self._pending = ""

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass

    def send_line(self, line: str, game: bool = True) -> None:
        """Sends `line`; a game command (`game`) that would train is refused."""
        if game and FORBIDDEN.match(line):
            raise Failure(f"refusing to send {line!r}")
        self.mark = len(self.text)
        self.sock.sendall(line.encode("cp437", "replace") + b"\r\n")

    def _feed(self, chunk: bytes) -> None:
        out = bytearray()
        replies = bytearray()
        for b in chunk:
            st = self._state
            if st == "data":
                if b == IAC:
                    self._state = "cmd"
                else:
                    out.append(b)
            elif st == "cmd":
                if b == IAC:
                    out.append(IAC)
                    self._state = "data"
                elif b in (DO, DONT, WILL, WONT):
                    self._state = b
                elif b == SB:
                    self._state = "sub"
                else:
                    self._state = "data"
            elif st == "sub":
                self._state = "subiac" if b == IAC else "sub"
            elif st == "subiac":
                self._state = "data" if b == SE else "sub"
            else:
                if st == DO:
                    if b == NAWS:
                        replies += bytes([IAC, WILL, NAWS, IAC, SB, NAWS, 0, 79, 0, 24, IAC, SE])
                    else:
                        replies += bytes([IAC, WONT, b])
                self._state = "data"
        if replies:
            self.sock.sendall(bytes(replies))
        text = self._pending + out.decode("cp437")
        cut = PARTIAL_ANSI.search(text)
        self._pending = text[cut.start():] if cut else ""
        text = ANSI.sub("", text[:cut.start()] if cut else text)
        first, *rest = text.split("\x08")
        self.text += first
        for piece in rest:  # a backspace takes back the character before it, any chunk's
            self.text = self.text[:-1] + piece
        self.mark = min(self.mark, len(self.text))

    def until(self, pattern: str, timeout: float | None = None) -> re.Match:
        """Waits for `pattern` in what arrived since the last line was sent."""
        rx = re.compile(pattern)
        deadline = time.monotonic() + (timeout or self.timeout)
        while True:
            m = rx.search(self.text, self.mark)
            if m:
                return m
            left = deadline - time.monotonic()
            if left <= 0:
                raise Failure(f"no {pattern!r} within {timeout or self.timeout:.0f} s")
            self.sock.settimeout(left)
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                raise Failure(f"connection closed while waiting for {pattern!r}")
            self._feed(chunk)

    def closed(self) -> None:
        """Waits for the server to close the connection."""
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            self.sock.settimeout(max(deadline - time.monotonic(), 0.01))
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                break
            except ConnectionError:
                return
            if not chunk:
                return
            self._feed(chunk)
        raise Failure(f"still connected {self.timeout:.0f} s after logging off")

    def since_mark(self) -> str:
        return self.text[self.mark:]


class Smoke:
    def __init__(self, host: str, port: int, user: str, password: str, timeout: float,
                 state: str | None = None, wait: float = 60.0):
        self.host, self.port = host, port
        self.state = state
        self.wait = wait
        # a state file means an earlier run made the account: it must still be there
        self.expect_account = bool(state and os.path.exists(state))
        self.user, self.password = user, password
        self.timeout = timeout
        self.report: list[str] = []
        self.t0 = time.monotonic()

    def ok(self, what: str) -> None:
        self.report.append(f"ok   {time.monotonic() - self.t0:6.1f}s  {what}")

    def connect(self) -> Telnet:
        t = time.monotonic()
        c = Telnet(self.host, self.port, self.timeout, self.wait)
        self.conn = c
        c.until("Username:")
        # no address in the report, so it can be pasted as it is
        self.ok(f"connected, Username: in {time.monotonic() - t:.1f} s")
        return c

    def login(self, c: Telnet) -> None:
        """Username and password, or `NEW` when the name is unknown; up to `[MAJORMUD]:`."""
        c.send_line(self.user, game=False)
        c.until("Password:")
        c.send_line(self.password, game=False)
        m = c.until(r"Make your selection \(X to exit\):|Invalid Credentials|already|locked")
        if m.group(0) == "Invalid Credentials" and self.expect_account:
            raise Failure(f"{self.user} did not log in, but the last run with this --state "
                          "file made it: the account is gone, or the password is wrong")
        if m.group(0) == "Invalid Credentials":
            self.signup(c)
            self.expect_account = True
        elif m.group(0) != "Make your selection (X to exit):":
            raise Failure(f"login refused: {c.since_mark().strip()[:200]!r}")
        else:
            self.ok(f"logged in as {self.user}")
        c.send_line("M")
        c.until(r"\[MAJORMUD\]:")
        self.ok("BBS menu: M reached [MAJORMUD]:")

    def signup(self, c: Telnet) -> None:
        c.until("Username:")
        c.send_line("NEW", game=False)
        steps = [
            (r"unique Username", self.user),
            (r"strong Password:", self.password),
            (r"re-enter your password", self.password),
            (r"e-Mail Address:", f"{self.user.lower()}@example.invalid"),
            (r"gender 'M' or 'F'", "M"),
        ]
        refused = (r"unavailable|better password|did not match|must be between"
                   r"|Please enter a valid (?:Username|e-Mail address\.|gender selection)")
        for prompt, answer in steps:
            m = c.until(f"(?P<p>{prompt})|{refused}")
            if m.group(0) == "unavailable":
                raise Failure(f"{self.user} exists and refused the password (wrong SMOKE_PASSWORD?)")
            if not m.group("p"):
                raise Failure(f"signup refused: {m.group(0)!r}")
            c.send_line(answer, game=False)
        c.until(r"Make your selection \(X to exit\):")
        self.ok(f"signed up {self.user} with NEW")

    def enter(self, c: Telnet) -> str:
        """`E` from `[MAJORMUD]:`; creates the character the first time. Returns the room."""
        c.send_line("E")
        m = c.until(PROMPT + "|choose your race")
        if m.group(0) == "choose your race":
            self.create(c)
        else:
            self.ok("entered the realm")
        return self.look(c)

    def create(self, c: Telnet) -> None:
        c.send_line("1")  # Human
        c.until("choose your class")
        c.send_line("1")  # Warrior
        m = c.until(r"Lawful\?|Given name")
        if m.group(0) != "Given name":
            c.send_line("N")
            c.until("Given name")
        for answer, nxt in [
            ("", "Family name"),
            ("Smoke", "Strength"),
            ("", "Intellect"),
            ("", "Willpower"),
            ("", "Agility"),
            ("", "Health"),
            ("", "Charm"),
            ("", "Hair length"),
            ("", "Hair colour"),
            ("", "Eye colour"),
            ("", "SAVE or QUIT"),
            ("", PROMPT),
        ]:
            c.send_line(answer)
            c.until(nxt)
        self.ok("created a character (Human Warrior, default stats)")

    def command(self, c: Telnet, line: str, expect: str) -> str:
        c.send_line(line)
        c.until(expect)
        c.until(PROMPT)
        return c.since_mark()

    def look(self, c: Telnet) -> str:
        out = self.command(c, "look", "Obvious exits:")
        return room_name(out)

    def walk(self, c: Telnet, start: str) -> str:
        """Four steps through ordinary exits (up to eight, to end away from `start`), each
        way once (and the way back counts) when there is another way, with a `look` in each
        room. Returns the room it ends in, which is not `start`."""
        here, back, steps, taken = start, "", 0, set()
        for _ in range(8):
            if steps >= 4 and here != start:
                break
            exits = [e for e in parse_exits(c.since_mark()) if e in OPPOSITE]
            fresh = [e for e in exits if (here, e) not in taken]
            way = next((e for e in fresh if e != back), (fresh or exits or [None])[0])
            if way is None:
                break
            out = self.command(c, way, "Obvious exits:|You can't go that way|closed")
            if "Obvious exits:" not in out:
                break
            there = room_name(out)
            taken |= {(here, way), (there, OPPOSITE[way])}  # the way back is known too
            here, back, steps = there, OPPOSITE[way], steps + 1
            self.ok(f"walked {way} to {here!r}")
            self.look(c)
        if steps == 0:
            raise Failure(f"could not walk anywhere from {start!r}")
        if here == start:
            raise Failure(f"the walk ended where it started, {start!r}: the re-login checks nothing")
        return here

    def logoff(self, c: Telnet) -> None:
        """`quit` (after the meditation), then `X` at both menus; the server hangs up."""
        c.send_line("quit")
        c.until(r"\[MAJORMUD\]:", self.timeout + 15)
        c.send_line("X")
        c.until(r"Make your selection \(X to exit\):")
        c.send_line("X")
        c.until(r"Are you sure \(Y/N")
        c.send_line("Y")
        c.closed()

    def run(self) -> None:
        c = self.connect()
        try:
            self.login(c)
            start = self.enter(c)
            self.ok(f"look: {start!r}")
            if self.state and os.path.exists(self.state):
                with open(self.state, encoding="utf-8") as f:
                    saved = f.read().strip()
                if start != saved:
                    raise Failure(f"entered in {start!r}, the last run left in {saved!r} ({self.state})")
                self.ok(f"still in {saved!r}, where the last run left (--state)")
            here = self.walk(c, start)
            stat = self.command(c, "stat", r"Name:")
            name = re.search(r"Name:\s*([^\r\n]+?)\s{2,}|Name:\s*([^\r\n]+)", stat)
            self.ok(f"stat: {(name.group(1) or name.group(2)).strip() if name else '?'}")
            self.command(c, "inventory", r"You are carrying|Wealth")
            self.ok("inventory")
            self.logoff(c)
            self.ok(f"quit in {here!r} and logged off")
        finally:
            c.close()
        c = self.connect()
        try:
            self.login(c)
            again = self.enter(c)
            if again != here:
                raise Failure(f"logged back in to {again!r}, left in {here!r}")
            self.ok(f"logged back in, still in {again!r}")
            self.logoff(c)
            self.ok("logged off")
        finally:
            c.close()
        if self.state:
            with open(self.state, "w", encoding="utf-8") as f:
                f.write(again + "\n")


def room_name(out: str) -> str:
    """The room name: the line before the indented description, or (a room shown without
    one) the first line after the command echo."""
    lines = [ln.rstrip() for ln in re.split(r"\r?\n", out.split("Obvious exits:")[0])]
    for i, ln in enumerate(lines):
        if ln.startswith("    ") and i > 0 and lines[i - 1].strip():
            return lines[i - 1].strip()
    rest = [ln.strip() for ln in lines[1:] if ln.strip() and not re.match(PROMPT, ln.strip())]
    return rest[0] if rest else "?"


def parse_exits(out: str) -> list[str]:
    m = None
    for m in EXITS.finditer(out):
        pass
    if not m:
        return []
    return [e.strip().lower() for e in re.split(r",|\band\b", m.group(1).rstrip(". ")) if e.strip()]


def parse_target(target: str) -> tuple[str, int]:
    """`HOST`, `HOST:PORT`, `[V6]:PORT` or a bare IPv6 address; port 2323 by default."""
    if target.startswith("["):
        host, _, rest = target[1:].partition("]")
        return host, int(rest[1:]) if rest.startswith(":") else 2323
    if target.count(":") == 1:
        host, _, port = target.partition(":")
        return host, int(port)
    return target, 2323


def failure_report(s: Smoke, e: BaseException) -> str:
    """The report so far, the failure and the last screen, the password masked in all."""
    lines = [*s.report, f"FAIL {time.monotonic() - s.t0:6.1f}s  {e}"]
    conn = getattr(s, "conn", None)
    if conn is not None:
        lines.append("---- last output ----\n" + conn.text[-800:].replace("\r", ""))
    text = "\n".join(lines)
    return text.replace(s.password, "********") if s.password else text


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("target", help="HOST, HOST:PORT or [IPV6]:PORT (port 2323 by default)")
    ap.add_argument("--user", default="smoketest", help="account and character name (default smoketest)")
    ap.add_argument("--timeout", type=float, default=20.0, help="seconds per step (default 20)")
    ap.add_argument("--wait", type=float, default=60.0,
                    help="seconds to keep trying to connect, for a box still rebooting (default 60)")
    ap.add_argument("--state", metavar="FILE",
                    help="check the character enters where the last run with FILE left it, "
                         "then record where this run leaves it (for a restart or reboot between runs)")
    a = ap.parse_args(argv)
    host, port = parse_target(a.target)
    password = os.environ.get("SMOKE_PASSWORD") or getpass.getpass(f"password for {a.user}: ")
    s = Smoke(host, port, a.user, password, a.timeout, a.state, a.wait)
    try:
        s.run()
    except (Failure, OSError, UnicodeError) as e:
        print(failure_report(s, e))
        return 1
    print("\n".join(s.report))
    print(f"PASS {time.monotonic() - s.t0:6.1f}s  {len(s.report)} steps")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
