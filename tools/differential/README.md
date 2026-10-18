# Differential harness

Replays a T-005 scenario against the original `WCCMMUD.DLL` under MBBSEmu and against
silvermere, normalises both outputs into event sequences with the pattern registry, diffs them
and writes a report. Python, stdlib only, dev only; never shipped. It is layer 4 of the
test plan.

It reuses the recorder of the project's MBBSEmu test harness (`capture.Runner`, `bridge.Session`,
`patterns.Registry`), so the send / `until` / `timeout` discipline is the one that made the
captures. Those modules, the staging scripts and the distribution zip are vendored in
[`vendor/`](vendor/SOURCE.md) (set `SILVERMERE_RESEARCH_DIR=/path/to/harness` to use another copy
of the harness tree instead). Always run with `python3 -I`.

## Running it

```
# no servers: a recorded capture plays both sides
python3 -I tools/differential/differential.py run --scenario entry-existing \
    --orig capture:tests/captures/entry-existing --new capture:tests/captures/entry-existing
# PASS; with --new capture:tests/captures/entry-new-account it FAILs with a unified diff

# running servers (defaults: orig 127.0.0.1:2323, new 127.0.0.1:2325)
python3 -I tools/differential/differential.py run --scenario entry-existing --report report
python3 -I tools/differential/differential.py run --all --report report

# stage both from the same dist zip, start them, run, stop them
python3 -I tools/differential/differential.py run --stage --scenario entry-existing --report report
```

Options: `--orig/--new host:port|capture:DIR`, `--report DIR`, `--no-setup` (skip the scenario's
setup scripts, for an instance that already has the character), `--max-step-timeout S`,
`--include-statistical`. Exit status is 0 when every scenario passes or is skipped, 1 on a
failure, 2 when staging is impossible.

### `--stage` and the manual run

`stage.py` calls the vendored `vendor/test/stage.sh` and `vendor/test/run.sh orig` (needs
`vendor/tools/mbbsemu/MBBSEmu`, git-ignored: download the release `HANDOFF.md` names, `v1.0-alpha-090826`,
asset `mbbsemu-linux-x64-*.zip`, unzip into `vendor/tools/mbbsemu/`, `chmod +x`; needs `tmux`). For the
new server it runs `silvermere --data-dir vendor/test/run/new import --from <dist>`
and `serve --telnet-bind 127.0.0.1:2325 --seed 1` (`cargo build -p silvermere`
first, or set `SILVERMERE_BIN`). `silvermere import` is a stub until T-008 lands: stage prints
that and carries on. Until T-021 the new side fails at its first prompt and the report says
`new: timeout waiting for 'Username:'` (exit 1). To run by hand: start MBBSEmu as `test/run.sh
orig`, start silvermere yourself, then run without `--stage`.

CI: `.github/workflows/differential.yml` (nightly, and a PR labelled `differential`) builds
`docker/mbbsemu/Dockerfile` from the staged original, starts silvermere, runs `--all` and
uploads `report/`. The job is `continue-on-error: true` until T-029 flips it. It needs
no secrets.

## Files

| File | Role |
|---|---|
| `differential.py` | CLI (`run`), scenario overlays, pass/fail |
| `replay.py` | `Player.run_script(script) -> [StepCapture]` over `capture.Runner`; `from_capture` reads a recorded capture the same way |
| `normalise.py` | `raw_to_lines` + `Registry.iter_events` + masks from `tests/fixtures/masks.json` (the single mask file; the Rust tests load it too) |
| `diff.py` | event diff (silvermere-testkit's `diff_events` per pair, after aligning the two lists so one extra event is one difference), unknown-line check, byte comparison |
| `report.py` | `report/<scenario>/{orig.raw,new.raw,orig.events.jsonl,new.events.jsonl,diff.txt,summary.json}` and `report/summary.md` |
| `stage.py` | `--stage` |
| `vendor/` | `play/{bridge,capture,patterns}.py`, `test/{stage.sh,run.sh,keygen.py,config/}`, `dist/` zip; `SOURCE.md` lists the edits made on copying |
| `scenarios/NAME.json` | overlay on `tests/captures/NAME/script.json` |
| `tests/` | `cross_check.py` (Python vs Rust normalisation), `test_replay.py`, `test_differential.py`, `test_normalise.py`, `test_vendor.py` (vendored registry export equals `tests/fixtures/patterns.json`) |

`events.jsonl`: one object per event, `{step, name, cat, fields}`, masked fields `"*"`;
`unknown` events also carry `line`. `step` is the script step whose send most recently preceded
the event. Per-step bytes are cut at each step's send offset, so text that arrives between
commands belongs to the step it arrived in.

## Adding a scenario

1. Record it with the harness's `capture.py` (`vendor/play/capture.py`), copy it to `tests/captures/NAME/`.
2. Add `tools/differential/scenarios/NAME.json`:

```json
{"scenario": "NAME", "script": "../../../tests/captures/NAME/script.json",
 "deterministic": ["st", "exp", "i"],
 "statistical": false,
 "setup": [{"side": "both", "scenario": "creation-fsd"}]}
```

- `deterministic`: the `send` texts whose steps are compared as bytes, or `"all"`; default
  `st exp i set spells par who`. Script `mask` regexes (build stamp, clocks, direction slots)
  are blanked first; compared once with ANSI stripped, once with colour.
- `rerun_restore: SCRIPT`: played on each side before a rerun after interference, so the second
  attempt starts from the first one's state (m4/quest: Aldric rested to full, `st` showing 35/35, and walked back to
  his start); a problem in it fails the scenario.
- `no_rerun: true`: the scenario is not played again after interference (a wanderer's arrival):
  its character is not where it began (m3/bank walks to a bank across the water), so the second
  attempt would fail on its first step. The failure stands, with a note saying why.
- `skip: {card, why}`: not played; reported as SKIP under that card (no scenario carries one now: T-084 took it off m4/aoe and m4/quest).
- `max_reruns: N` (default 1, T-084): how many times a scenario is played again after interference (see the m4 list below); m4/sysop 3, m4/quest and m4/aoe 2.
- `statistical: true`: skipped until T-040 (fights, deaths); `masks.json` `statistical_events`
  (the prompt's HP/MA) are masked when it is run with `--include-statistical`.
- `setup`: capture scripts replayed first (output discarded), `side` is `orig`, `new` or
  `both`. Until T-063 the new side's character creation is the same script as the original's.

## Acceptance (per scenario)

Equal event sequences; no `unknown` line on the new side that the original does not also have;
no timeout, no refused connection, no disconnect; byte equality on the deterministic steps.
RNG statistics and record-level comparison: see the M2 section below (T-040).

## The m1 group

`scenarios/m1/*.json` are the M1 ("walk") scenarios: `python3 -I tools/differential/differential.py
run --scenarios m1 --orig 127.0.0.1:2323 --new 127.0.0.1:2325 --report report` (or one,
`--scenario m1/room-walk`). Their `setup` is per side, because the two servers draw character
creation differently: the original takes `entry-new-account` + `creation-fsd` (MBBSEmu's
positioned form, `until_cursor` waits), silvermere takes `scenarios/m1/setup/new-aldric.json`
(NEW signup, then the line-mode editor, same Aldric Stonebrook). A setup runs once per side
and instance, not once per scenario. `ignore_steps` drops the events of the steps that only
show the BBS host (MBBSEmu's banner box and login echo). `flood-and-refusals` runs a trimmed
script (`scenarios/m1/scripts/`) without the monster fight until T-035/T-040.

## The m3 group

`scenarios/m3/*.json` are the M3 scenarios (T-048): `shops`, `bank`, `party`, `comms`; run with
`--scenarios m3`. `shops` and `bank` use one connection; `party` and `comms` use two (Aldric and
Brynja) and `bank` plays Isolde, who ends in Silvermere. The setups for Brynja and Isolde are the
`entry-new-account` and `creation-fsd` scripts with the names substituted (`m3/setup/orig-*`) for
the original and `m1/setup/new-aldric.json` with the names substituted (`m3/setup/new-*`) for
silvermere. None of the four has been run against the original yet: the original was not
available when they were written, so they were checked against silvermere only (no timeout, the
text each step printed), and the first run against MBBSEmu may need masks or ordering changes.

A script may drive several connections. A step with `"as": "B"` runs on the connection B (A when
there is no `as`); B opens with its own `connect` step. A step's bytes end where the next step of
the same connection starts. To wait for a message another connection's command caused, use

```json
{"as": "B", "op": "sleep", "seconds": 20, "since": "last", "until": "has invited you to follow"}
```

`since: last` searches from this connection's last send instead of from the moment of the step,
because the message usually arrived while the script was busy with the other connection. The
bytes still belong to the step that was open when they arrived, so write the wait before the
connection's next send. Use a text that nothing earlier in the run also matches.

### Known gaps and aborted runs

`known_gaps.json` lists differences that belong to a card that has not landed. A rule names its
owning card and the side it applies to (`orig`, `new` or `both`). Its events are dropped before the
event diff (so one missing event does not shift every later pair), `then_drop` also drops the
named event directly after a dropped one, `byte_patterns` are removed from the decoded step text
before the byte-equal check (so a gap is classified in both layers), and each scenario's
`summary.json` counts what was dropped per card as `known_gaps`. The card that fixes a gap removes
its rule: the T-035 rule (monster and NPC presence lines) and the T-043 rule (the `par` list) are
gone. The one rule left is T-040's: a monster's noise in the next room (`You hear movement to the
north.`, whose direction word the original scrambles with a random letter and a backspace) and the prompt it is sent with, in the same step, are dropped on both sides, mirroring
`capture_compare::masked_lines` in the Rust capture tests; T-040 compares monsters and removes it.

A bare Enter waits until the server has been quiet for 2 s (`settle`; the original, MBBSEmu,
drops one sent too soon after the realm-entry prompt, and on a loaded host no fixed pause is
long enough), then gets at least 30 s (the original answers a room redraw late now and then).
If after 12 s (`drop_after`) all that came back is the CRLF echo, the original dropped it: the
Enter is sent again (at most twice) and the lost echo is cut from the stream, so the step reads
as an Enter taken the first time. `summary.json` counts these as `enter_retries`; a retry on the
new side is a problem (silvermere must not drop input). A script that aborts in the realm logs
off with `x` / `X` / `X` / `Y` instead of dropping the socket, so the character is not marked as
a hang-up for the next scenario.

Consecutive sends typed without waiting (`nowait`, the flood burst) go out as one write. Flood
control counts the lines already queued, and the queue drains one line per fast tick, so the
outcome depends on how many ticks pass while the burst arrives; one write per line from a
client starved on a loaded host let a tick in on one side and not the other. One write is not
enough on its own: the original can still take a tick between two lines of one write. So the m1
`flood-and-refusals` script steps north into the Weapons Shop just before the burst. A move runs
on a fast tick, so its room and prompt arrive just after one, and the burst sent at once has a
whole tick (1 s) to queue before the next tick can drain a line, on both sides. The script steps
back south after the queue has drained, so the rest of it runs from the Village Entrance as
before and ends where it did.

That still bounds nothing on the original: MBBSEmu takes one input byte per pass of its host
loop and runs the module's timers and SYSCYC hook in every pass (`MbbsHost.WorkerThread`), so
on a loaded host the 63-byte burst can take longer than a tick to arrive. So the script carries
a `burst_check`: after the queue has drained, the harness reads that side's own output since
the burst was sent. Flood control prints 6 warning lines before the first queued line runs when
the burst queued whole. If fewer came before the first queued echo, a tick
landed mid-burst. Then the harness walks out (`reset`), leaves that attempt and the walk out of
the capture, and does steps 5-21 again, at most twice (`max_retries`). The attempt that is kept
is compared event by event and byte by byte like any other. A miscounting silvermere still
fails: too few warnings retry and then fail the comparison, too many never retry.
`summary.json` counts the redone bursts per side as `burst_retries`. A retry is not a problem
on either side, since a starved silvermere can take a tick mid-burst too.

### Scenario options for host and timing noise

- `ignore_host_banner`: drops the events of every `connect` step on both sides. The banner that
  MBBSEmu shows again on a reconnect is attributed to the connect step, not to the step before
  it, so `ignore_steps` no longer has to count them.
- `merge_steps: [[first, last]]`: steps typed without waiting (the flood burst) are compared as
  one block, in events and in the byte check, because which step a prompt lands in depends on
  arrival time.
- `script_options: {key: value}`: script keys set on the replayed script, for a scenario whose
  script is shared (a recorded capture under `tests/captures/`), e.g. `echo_anchored` and
  `settle`.
- `mask_prompt` (T-057): masks the prompt event's numbers (`statistical_events` in
  `tests/fixtures/masks.json`: hp, mana) in a deterministic scenario whose HP or mana is random.
- `echo_anchored` (a script key, T-057): each step after the realm entry whose `until` is the
  prompt waits for its own command's echo first (`replay.anchor_on_echo`). Without it, a prompt
  the original redraws late satisfies the next step's wait, and every later step on that side
  is shifted one back.
- `random_steps` (T-057): steps whose events are random on both servers (a fight: cast
  failures, the monsters' adjectives and attacks) are not diffed as events nor byte-compared;
  their unknown lines still count. The rates belong to the statistical scenarios.
- `required_events` (T-057): `{step: [names]}` that must show on both sides in that step, so
  a random step's structure (the cast, the end of combat) is still checked.
- `event_masks` (T-057): `{event: [fields]}` masked in this scenario only (a summoned monster's
  adjective and direction, a lit torch's burn count).
- `answer_pager` (a script key, T-057/T-082) with `pager: true` on a step: replay answers each
  "Hit any key to continue..." of a host's page mode with a key until the step's own `until`
  follows the last pause; the pause line is a known gap of T-082. MBBSEmu (the pinned tag)
  ignores `btuxnf`, so the live original does not page and this never fires there.
- `value_masks` (T-057): `[{"event", "where": {field: value}, "fields"}]` masks the digits of
  those fields (each run of them becomes `#`; the words around stay compared) only in the
  events whose `where` fields match (one settings line's play time).
- `bytes_from_echo` (T-057): a step is byte-compared from its own command's echo on; what
  arrives before it is the step before's tail (an attack, a late prompt).
- `settle` (a script key, T-057): seconds of quiet replay waits for after each in-realm
  step, so a reply the original sends in two parts around a prompt stays in its step.
- `divergences` (T-057): rules of the `known_gaps.json` shapes for a line a decided port
  leaves out on purpose, named by the deciding card and counted in the report the same way.
- A known-gap rule may have a `shape` instead of patterns: `ansi_normal_after_echo` (T-083)
  accepts a colour difference only when it is exactly one ANSI_NORMAL directly after the
  echo on the new side (or that and the CLRLINE the original clears with it) and the rest of the step
  is byte-equal; counted per step under the card, any other difference fails.
- An exact repeat of the prompt directly after itself (the slow tick's redraw of the same
  prompt) is collapsed in events and bytes on both sides; summary.md shows the collapsed count
  per side (`Prompt redraws collapsed`), and a scenario where one side collapsed more than
  twice the other's (and at least 3, so 3 against 0 too) fails with that as a problem, so a
  systematic difference is not absorbed by the collapse.
- A failed scenario in which a monster nobody summoned arrives or attacks outside a fight step
  (a wanderer), or an earlier fight's stun or knockdown wears off (WCCMSG 2296), is played
  once more, but only when its first failing step (an event or byte difference, or a timeout)
  is at or after the first interference; a failure before it is the scenario's own. The first
  attempt's report stays as `<name>/attempt1`, and the result's `rerun` and notes say what
  interfered. Summoning scenarios rerun too: SYSOP MONSTER forces its spawn past the room's
  regeneration delay. The m4 scenarios are staged in rooms with no
  generation and few wanderers (mongroup 0: montype-0 monsters, Angels and Quest monsters
  only).
- Interference also covers a Temple's ambient line (WCCMSG 2676-2678: incense, choir, bell; room
  spell 925 "temple noise", WCCTEXT 9127/9128, about 1% each per 6 s on both servers)
  and an earlier fight's effect wearing off. A line to
  one side only is never masked: the attempt is played again (`max_reruns` times).
- A line a prompt redraw cut in two (T-084: a typed command's echo `use scr[HP=300/MA=2]:oll...`,
  or the exit's dots) is rejoined on both sides (`normalise.rejoin_split_lines`, only two shapes: inside
  the step's own echo, the text before it a case-insensitive prefix of the `send` and the text after
  completing it (or the whole `send`, the line ending right after the redraw: a tick between the last byte and the CR, T-094); or dots only on both sides. A redraw inside any other message stays and fails. A redraw directly after the step's own complete echo line with the output glued after it cannot come from the original (MbbsHost.cs:426, :929, :458-459) nor, since T-094, from silvermere: it stays and fails) before events and bytes are computed, and the echo wait
  of an `echo_anchored` step accepts a redraw between any two characters of the command. The
  report keeps the original bytes; the count per side is in the notes and in the "Prompt
  redraws collapsed" column, so a side that is cut far more than the other still fails.
- Known gaps and divergences are counted per card in each scenario's `summary.json` and in
  `summary.md` (`card: orig/new events +orig/new byte removals`), so a gap is shown, not hidden.
- `known_gaps.json` rules may carry `byte_patterns`, removed from that side's decoded step text
  before the byte-equal check, so a gap is classified in both layers.

### Echo markers

A step's offset is read right after the send, so it lands before or after the echo depending on
arrival time, and a burst typed without waiting records one offset for all its sends. The
registry keeps only the last few pending echoes, so `normalise.anchor_sends` moves each send's
marker onto its own echo (the first unclaimed `text` + CR after a `:` or `?`, in order); a send
with no echo (a dropped line) keeps the recorded offset. The same bytes now classify the same
way whatever the timing.

## The m4 group

`scenarios/m4/*.json` (T-057): spells, items, the T-054 scripts, the sysop screens, hang-up and
cleanup. Most play as the BBS sysop's character Sysadm and set the stage with the developer
sysop commands (TWEAK, SUMMON, EXPRESS_TRAIN, MONSTER), which need `CODEC_OF_TWINKS=78` on the
original (opt-in: `STAGE_CODEC_OF_TWINKS=78` for `vendor/test/stage.sh`, or
`docker run -e CODEC_OF_TWINKS=78` for the image) and the DEBUG grant on silvermere; preparation and pass criteria are in
[`docs/testing.md`](../../docs/testing.md#layer-4-the-m4-differential-group).

- `--world-json DIR` (or `$SILVERMERE_WORLD_JSON`): the classifier gets the game-data patterns
  (room titles, monster and weapon texts, the generated spell families, every WCCMSG text) from
  `silvermere import --json DIR` through `world_json.py`. A scenario marked `world_patterns`
  fails without it rather than count every spell line as unknown.
- `"repeat": {"first", "last", "times"}` plays a script's cycle many times (the statistical
  runs store one cycle); `--repeat-times N` caps it.
- `differential.py replay --script m4/setup/new-sysadm.json --to 127.0.0.1:2325` plays a
  one-time preparation against one server.
- `m4_stats.py report --report DIR` compares the statistical scenarios' captures (fail rates,
  damage histograms, monster and room casts) with T-040's tests in `stats.py`.

## M2: statistics and records (T-040)

The `scenarios/m2/` overlays are random scenarios (`"statistical": true`; run them with
`--include-statistical`) and need the game-data patterns (`world_patterns`, `--world-json`): the
monsters' swing, death and arrival texts are classified against the game data, so a wrong text
stays an unclassified line and fails. Their `m2` block says what is compared:

- `sequence`: the event names compared as a sequence (refusals, `*Combat Engaged*`/`*Combat
  Off*`, the death lines), in order, without their step numbers; the other events are random
  and go to `stats.py`. An unclassified line still fails the scenario.
- `min_swings`: the swing sample each side needs (2,000), for the player's swings and for the
  monster's swings at the player; fewer fail the comparison.
- `monster`: the sheet's monster; only swings at it and by it count (others in the room are
  not the sheet).
- The original's nightly cleanup warning (MBBSEmu `Cleanup.Time`, `Sorry to interrupt here, but
  the server will be shutting down ...`): a `stop` run logs off at its next cycle with what it
  has (stop reason `cleanup`), and any scenario that fails after it says so first ("interrupted
  by the original's nightly cleanup"), not rerun. `m2_live.sh` moves the cleanup out of its runs.
- Unclassified lines in a sample-count scenario (`m2.stop`, or `unknown_as_set: true`) are
  compared as digit-masked sets (a text the original never shows): the sides play different
  numbers of cycles, and statistics lines such as SYSOP BUFFERS differ in their numbers only.
  A fixed script keeps the multiset, which catches a doubled message.
- `stop`: John's rule (2026-10-16), sample counts, not wall-clock soaks. `{minutes, count,
  at_least}`: before each cycle (`"stop_check": true`) the run stops cycling once this side's
  sessions have `at_least` matches of `count` (the player's swings) between them, and of each
  `also` count (the monster's swings at the player), or `minutes`
  have passed (the hard cap), and goes on at the `"after_stop"` step (the logoff). The parallel
  sessions of a side pool their counts in `--stop-pool DIR`. `SILVERMERE_M2_MINUTES` lowers the
  cap for a check. Why each session stopped (`count` or `minutes`, with how far it got) goes into
  its `summary.json` (`samples.stop`) and `stats.md`.
- `repeat`: `{first, last, times}` plays steps first..last that many times (one cycle is
  stored); `#CYCLE#` in a step becomes the cycle's number. Steps marked `"round": true` are
  5-second wall-clock windows, one energy round long (secenergy 5 s): the player's swings in
  one are counted as that round's blows. A window is not aligned with the server's round, so
  one can straddle two; both sides are measured the same way.
- `gate_wanders`: the walk-ins are gated (the wander watch); elsewhere they are advisory.
- `watch` (with `monster`): arrivals and noise count only in the steps marked `"watch": true`,
  and only of that monster, per minute of those steps (the wander watch).
- A gated rate row with no event on either side refuses ("no events on either side, not
  compared"): 0 against 0 checks nothing.
- `regen: false`: no regen sample (the sheet runs reset the hit points each cycle).

The sheet runs: `m2/swings-rat`, `-kobold`, `-slime` play `scripts/swings.json` as a sysop's
character set to the Kang Witchunter sheet with the developer commands (`SYSOP TWEAK`,
`SUMMON`, `MONSTER`, `EXPRESS_TRAIN`, as m4), against one monster of the sheet summoned each
cycle (the target, nothing more), in a room of its own (`@ROOM@`) cleared first of anything an
earlier run left, and set back to the character as created at the end, or on an abort
(`on_abort`). `SYSOP BUFFERS` at the start and the end puts the active-monster cache into
`stats.md`; `monster swings per round` (advisory: an MBBSEmu artifact, John, 2026-10-17) counts the monster's swings at the player in
each round window. `differential.py run --as USER:PASSWORD --set @ROOM@=N` plays one session;
`@USER@`, `@PASSWORD@`, `@USERBS@` and a scenario's `subst` keys are replaced in its scripts
(also `differential.py replay --as`, for the setup scripts `m2/setup/*-sysop.json`).
`m2/spawn-arena` is the arena watch: a strong sysop's character fights whatever is in the
Newhaven Arena for 300 passes of the spawn timer (25 minutes) a side; its walk-ins are
advisory. `m2/wander` keeps three lashworms in a room and watches the next one for them walking
in. It is not in `m2_live.sh stats` (`M2_WANDER=1` adds it): the wander pass moves at most
MAXMOVERS (3) monsters a pass in monster-cache slot order (T-086), so on
the stock world's active monsters such a watch sees none in minutes, and 0 against 0
refuses. Run 38992586034 saw none on either server in 540 s (the original does not move its
cached lashworms either): that agreement and T-086's engine tests are the wander evidence.

The script masks hide values only, never labels: digits with lookbehinds and lookaheads
(`(?<=\[HP=)-?\d+`, `(?<=for )\d+(?= damage)`, `\d+(?= copper farthings?)`), with a column's
padding where its width follows the number (`Hits:`, `Exp:` on `st`).

`stats.py` (stdlib; the statistics are `crates/silvermere-testkit/src/stats.rs`, and
`stats.py selftest` checks the same numbers) compares hit rate (two-proportion z, and for the
player's swings within 2 points), the outcome mix, the damage histogram and blows per round
(chi-square homogeneity), the same for the monster's swings at the player (no 2-point bound),
monster swings per round (advisory), regen per tick (identical sets of gains), spawns per minute (monsters
created "from nowhere", two Poisson rates) and, in a watch that gates them, walk-ins per minute.
The gated tested rows of a report are one family: Holm-Bonferroni at family alpha 0.01 (`holm`;
John, 2026-10-17, option C), and `stats.md` shows each row's raw p, Holm threshold and adjusted
p. With `SILVERMERE_M2_MINUTES` set (a check run) the 2,000-swing samples are not required.
Elsewhere walk-ins and monster noise next door are only reported (advisory, FLAG at raw
p <= 0.01). `stats.py report --report DIR [--report DIR2 ...]` writes
`stats.md`/`stats.json` from the scenarios' `summary.json`, pooling the sessions of one scenario,
with a table of the sample sizes (swings, monster swings, spawns, walk-ins, seconds, how each
session stopped). `stats.py records --orig-db DIR --new-db FILE --name NAME` compares the
original's `WCCUSERS` row (decoded by the vendored `mmud_data.py`) with the new server's
`characters` row, and `WCCACMSR` with `active_monsters` per map. It fails on a player field or
on a map with monsters on one side and none on the other; per-room counts are listed but do not
fail it, since monsters wander. Two things are not carried by design and are not compared:
`settings`' `PS_PAYING` bit (0x8000, set from the dropped SAVEKEY) and a worn item's uses (arbitrary on the original after EQUIP;
the worn item numbers are compared). In the statistical scenarios a monster arrival (spawn or walk-in)
is a random event (`random_events.json`, `statistical_only`).

Both run on demand on the maintainers' runner with
`tools/differential/m2_live.sh short|stats|both` (docs/testing.md, "M2"). There is no M2 soak
(John, 2026-10-16).
