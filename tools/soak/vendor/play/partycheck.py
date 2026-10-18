# Copied from the project's MBBSEmu test harness; edit here (see vendor/SOURCE.md).
#!/usr/bin/env python3
"""
Live check for group scripting stage G0: two characters, each on its own
bridge session, exchange numbered @-messages through party.Remotes, and we count what arrives.
Stop the bots driving those sessions first (one driver per session).

  partycheck.py --a "":Brynja --b cleric:Isolde --count 50
"""
import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bridge  # noqa: E402
import mmud_data as md  # noqa: E402
import party  # noqa: E402
import patterns  # noqa: E402


class Side:
    def __init__(self, session, name, other, reg):
        self.session, self.name, self.reg = session, name, reg
        self.got = []
        self.remotes = party.Remotes(name, [other], self.send, log=lambda s: print(f"  [{name}] {s}"))

    def call(self, op, **kw):
        bridge.set_session(self.session)
        r = bridge.call(op, **kw)
        text = r.get("text", "")
        self.remotes.feed(self.reg.classify(r.get("skipped", "")))
        self.remotes.feed(self.reg.classify(text))
        while True:
            m = self.remotes.take()
            if not m:
                break
            self.got.append(m)
        return text

    def send(self, text):
        return self.call("send", text=text, raw=False, wait=0.6, until=None, timeout=10.0)

    def read(self):
        return self.call("read", wait=0.5, until=None, timeout=10.0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", default=":Brynja", help="SESSION:NAME")
    ap.add_argument("--b", default="cleric:Isolde", help="SESSION:NAME")
    ap.add_argument("--count", type=int, default=50, help="messages each way")
    a = ap.parse_args()
    reg = patterns.Registry.load(world=md.World())
    (sa, na), (sb, nb) = a.a.split(":", 1), a.b.split(":", 1)
    A, B = Side(sa, na, nb, reg), Side(sb, nb, na, reg)
    for s in (A, B):
        s.send("set maxpages 30")
    t0 = time.time()
    for i in range(a.count):
        A.remotes.tell(nb, "ping", i)
        B.remotes.tell(na, "ping", i)
        for s in (A, B):
            s.remotes.pump()
            s.read()
    deadline = time.time() + 30
    while (A.remotes.queue or B.remotes.queue) and time.time() < deadline:
        for s in (A, B):
            s.remotes.pump()
            s.read()
        time.sleep(0.5)
    for s in (A, B):
        s.read()
    took = time.time() - t0
    for s, other in ((A, B), (B, A)):
        nums = sorted({int(m.args[0]) for m in s.got if m.word == "ping" and m.args})
        missing = sorted(set(range(a.count)) - set(nums))
        print(f"{s.name} received {len(nums)}/{a.count} from {other.name}; missing {missing[:10]}; "
              f"unreachable {sorted(s.remotes.unreachable)}")
    print(f"{2 * a.count} messages in {took:.0f}s")


if __name__ == "__main__":
    main()
