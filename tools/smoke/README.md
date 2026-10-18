# Outside-in smoke test

`smoke.py` checks a running silvermere from another machine, over the internet, the way a
player reaches it. Python 3, standard library only.

```sh
read -rsp 'smoketest password: ' SMOKE_PASSWORD && export SMOKE_PASSWORD && echo
python3 tools/smoke/smoke.py HOST                         # port 2323
python3 tools/smoke/smoke.py HOST:2323 --state smoke.state
```

It connects, logs in as `smoketest` (`--user NAME` for another), or signs that account up
with `NEW` the first time, enters the realm (creating a Human Warrior the first time), walks
four rooms with `look` (ending away from where it started), runs `stat` and `inventory`,
quits, logs off, logs back in and checks that the character is in the room it left. With
`--state FILE` it also checks that the character enters where the previous run with the same
file left it, so a restart, a reboot or a restore can go between two runs, and a missing
account then fails instead of signing up again. That tests the save at `quit`, not the
SIGTERM save of a player still online. `--wait SECS` (60) keeps trying to connect while a
box reboots.

The password is read from `SMOKE_PASSWORD`, or asked for when that is unset; it never
prints, not even in a failure. The output is one line per step and `PASS` (exit 0), or
`FAIL` with the last screen of output (exit 1), and names no address. It never sends a game
command starting with `tra` (login fields are not commands, so a name like `travis` works).

Telnet is plaintext: use a password for `smoketest` you use nowhere else.

CI runs it against the compose stack (`compose-smoke.yml`), before and after a restart.
Unit tests: `python3 -I -m unittest discover -s tools/smoke -p 'test_*.py'` (harness.yml).
