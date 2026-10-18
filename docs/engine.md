# silvermere-engine: the world thread

The engine runs the game world on one thread. This page records what the crate does, where
the original's behaviour is kept exactly, and which later card fills which file.

## Pieces

| Module | What |
|---|---|
| `world` | `World`: every live table in memory, loaded once from a `WorldSnapshot`; `reload_content` for the admin API; the engine-private tables (the occupied-room table of 512, `monqref` 2000, the exit revert queue, the restock list, evil timers, gang invites); `blank`: the `clear_*` zeroed records. |
| `scheduler` | `Scheduler`: the original's private tick queue (`Scheduler::arm`, `on_fast_tick`) keyed by the fast tick; `Routine` and `Periods`; `on_fast_tick` runs one fast tick. Re-exports `silvermere-rules`' `Clock` and `Rng`. |
| `event_loop` | `Engine`, `EngineInput`, `EngineOutput`, `AdminRequest`, `GameEvent`, `PersistRequest`; `catch_unwind` around each input; the flush. |
| `ids` | `SessionIds`: dense `u16` ids from a free list, lowest first, capped at `server.max_sessions`. |
| `player_session` | `PlayerSession`: the typed VDA (`UserState`, `UserFlags`, `editor_cp`, the party fields, the auto-combat ring, the output batch); `clear_player`. |
| `output` | `OutputBatch`: what one session receives at the end of the event, rendered by `silvermere-text`. |
| `tell` | `tell_user`, `tell_room`, `tell_game`, `tell_gang`, `tell_group` with the original's listener filters; the listener's prompt through T-020's renderer. |
| `state`, `session` | The `SessionState` machine shell and the host's lifecycle callbacks. |
| `commands` | The handler module list, registered once. |

The engine depends on `silvermere-types`, `-rules` and `-text` only (`tests/deps.rs` pins it).
`bin/silvermere` adapts it to `silvermere-net` and `silvermere-db` (`bin/silvermere/src/bridge.rs`).

## The event loop

One thread, one world, one input channel. Everything the plan's `select!` names arrives as
an `EngineInput`:

| Input | Source | Handler |
|---|---|---|
| `Connected { conn, caps }` | a session task | `session::on_connect`: a `SessionId` from the free list (or `Refused`), `on_login` |
| `LoggedIn { id, login }` | `bin`'s login gate, once `silvermere-auth`'s dialogue has logged the user in | `state::login::logged_in`: the account and character bound, `USRLOGIN`, the BBS menu shim or the menu (T-021) |
| `Line { id, text }` | a session task | `session::on_line`: the state dispatch |
| `Key { id, byte }` | a session task in character mode | `session::on_key` (T-024) |
| `Naws`, `Disconnect` | a session task | caps update; `session::on_disconnect`: `huprou`'s hang-up rules (T-021), then the logoff |
| `Tick` | a 1 s tokio interval in `bin` (`bridge::ticker`); tests inject it | `scheduler::on_fast_tick` |
| `Admin(..)` | the admin API (T-018) | `who`, `broadcast`, `reload-world` |
| `DailyCleanup` | the admin side, on demand (T-056) | `session::on_daily_cleanup`: the same run `Routine::DailyCleanup` starts at `game.cleanup.at`; the store answers `EngineOutput::CleanupScan` and `cleanup::on_scan` runs the four passes |
| `Custom { session, run }` | tooling and tests | the closure, on the world thread |
| `Shutdown` | `Running::stop` | `session::on_shutdown`, then the loop returns the world |

Each input is handled inside `catch_unwind`. A panic leaves the world as the handler left
it, logs the session and the input, replaces that session's queued output with
`Command interrupted.` (the original's wording) and continues; other sessions' queued
output still goes out (`tests/panic.rs`).

**Where `Tick` originates (the card's open question).** The engine never owns a timer: the
fast tick is an input like any other. In production `bin` runs a tokio interval of
`game.sec_fast` seconds and sends `Tick`; in tests `TestClock::advance_ticks` sends one
`Tick` per tick. This keeps the engine thread free of a runtime and makes every timing test
deterministic; the plan's "the engine `select!`s over session events, ticks and admin
requests" is realised where the channel is fed.

### Output

A handler appends `Msg` values to a session's `OutputBatch`; `tell_*` append to other
sessions' batches. At the end of the input, each touched batch (in first-touch order) is
rendered by `silvermere-text` in the listener's palette and sent as one `Write`, so the
messages one handler emits for a session arrive in emission order as one write, and other
sessions' writes follow (`tests/output_order.rs`). `Opened`/`Refused` for the
event's new connections go out first, so the transport has the session's mapping before
its first write; control outputs (`SetEcho`, `SetCharMode`, `Close`, `Event`, `Persist`)
follow the writes, so a `Close` never overtakes the text before it; a session that logged
off during the event (`World::logoff`, `Outcome::Logoff`) is removed and its id freed only
after that, so its goodbye text is its last write.

## Tick order

`on_fast_tick` runs one fast tick:

1. the tick count goes up by one;
2. the due timers run: while the head of the queue is due (its tick is at or before the
   current one) and fewer than `MAX_KICK_RUN` (1000) have run, run the head's routine, then
   unlink it. The node stays at the head while its routine runs, so the re-arm the routine
   does at the end of its body (`Scheduler::arm(period, routine)`) is linked in behind it;
3. `fast_update_characters`;
4. `fast_update_monsters`.

`Scheduler::arm` inserts a node after the last node due earlier: ahead of
nodes due at the same tick, unless the head itself is due then, in which case right after
the head. The scheduler keeps that rule, because it decides the order of two cycles that
fall due together: with the start order of `Scheduler::start` (monster
creation, save, slow, medium, energy, inactivity) the energy and monster-creation cycles,
which share a 5 s period, alternate (`Energy, Spawn` at tick 5, `Spawn, Energy` at tick 10,
...). `tests/scheduler.rs` pins the first minute. The card's "FIFO among equal" reading is
what a plain min-heap would do; the original does not.

A routine cannot double-fire by construction: the re-arm is relative to `scheduler.tick`,
which only `on_fast_tick` advances, and the fast tick is an input, so no routine body can
move the clock under itself.

Periods (`Periods::from_config`): energy `sec_energy` 5, medium `sec_med` 3, slow
`sec_slow` 30 (restock on every 21st slow run, `RESTOCK_SLOW` 20), monster creation
`sec_create` 5, save `sec_oth` rounded down to whole minutes with a floor of 60
(the stock `WCCMMUD.MSG` value of 160 gives 120), inactivity
`inactive_secs` 3600. Timed exits arm a one-shot `ExitRevert`.

## Sessions

`SessionId(u16)` replaces the channel number: `SessionIds` hands out the lowest free id,
reuses freed ids before fresh ones, and refuses with `ServerFull` at `server.max_sessions`
(`tests/session_ids.rs`). The transport's connection number (`ConnId`) is mapped to it in
`bin` once the engine answers `Opened`.

`PlayerSession` holds the account reference (`None` until T-021 binds one), the
`Character` (`None` until loaded), the
`SessionState`, `UserFlags`, `UserState` (with the 40-slot pending
queue and the forget list), `editor_cp`, the party fields (leader, members, dragging: session
only, never persisted), the 550-slot auto-combat ring, the output batch, the terminal caps,
`ingame` and `idle_since`. `is_in_game`: a
character loaded and a play state (`Playing` or later; `ingame` at `InGame::Realm` is kept
as a field only).

## Tell filters

All five walk the sessions in id order, consider only
sessions in the game, and append to each listener's batch the colour prefix
(the `sound` row in the listener's palette, nothing for `TELL_NOCOLOR` 0xFF or any row above 14),
the message, then the prompt when asked:

| Function | Listeners | Prompt | Counts |
|---|---|---|---|
| `tell_user` | one session | no | n/a |
| `tell_room` | same map and room, except up to two; `TellBlind::Sight` skips `PlayerStatus::BLIND` listeners; noise: `Noise::Always` reaches every listener, `Noise::Medium` all but those at `MessageLevel::Low`, `Noise::High` only those at `MessageLevel::High` | if asked | listeners told who are not hidden; hidden listeners receive but are not counted |
| `tell_game` | everyone but one, whose `player.flags` lack `mask`; for `PlayerFlags::NOGOSSIP`/`NOAUCTION` delivered while `gosscnt <= gossmax` / `auctcnt <= auctmax` (sic: `gossmax + 1` lines a period), counting up | if asked | listeners told |
| `tell_gang` | same `player.gang`, case-insensitive, the speaker included | always | listeners told |
| `tell_group` | same `player.bchannel`, except one | always | listeners told |

`tests/tell.rs` covers each row.

## State machine

`SessionState` (T-002) holds the session states; `session::on_line` dispatches on it,
one arm per state calling the
owning module in `state/`. Every arm returns `Outcome::Unimplemented(state)` until its card
fills it: nothing is printed, a line is logged. `Outcome::Logoff` is the host's `sttrou` returning 0:
the session is removed, its id freed, and `Close` sent.

The host's lifecycle callbacks are `session::on_connect`
(+ `on_login`, the `lonrou` reset), `on_line`, `on_key`,
`on_cycle`, `on_logoff` (`lofrou`), `on_disconnect` (`huprou`), `on_daily_cleanup`
(`mcurou`) and `on_shutdown` (`finrou`). The host's global-command pass and `injrou`
are not carried.

## Module ownership

Later engine cards fill bodies; none edits `lib.rs`, `state.rs` or `commands/mod.rs`.

| File | Owner |
|---|---|
| `state/login.rs` (`Start`, `Enter`, the BBS menu shim), `state/login/enter.rs` (`P`/`E`), `state/login/leave.rs` (`x`, the exit wait, `X` at the menu, `huprou`, the inactivity hang-up) | T-021 |
| `state/menu.rs` (`MainMenu`, `Help`) | T-021 |
| `state/newchar.rs` (`NewChar(..)`, `RaceChange(..)`) | T-022, T-047 |
| `state/play.rs` (`Playing`: `execute_input`, the pending queue, flood control) | T-024 |
| `state/listing.rs` (`Listing`, `UserReport`, `Disband`) | T-027, T-055, T-044 |
| `state/suicide.rs` | T-028 |
| `state/sysop_menu.rs` | T-055 |
| `commands/movement.rs` | T-026 |
| `commands/combat.rs` | T-033, T-034 |
| `commands/inventory.rs` | T-030, T-053 |
| `commands/shop.rs` | T-041 |
| `commands/bank.rs` | T-042 |
| `commands/training.rs` | T-039, T-047 |
| `commands/group.rs` | T-043 |
| `commands/gang.rs` | T-044 |
| `commands/comms.rs` | T-045 |
| `commands/info.rs` | T-025, T-027 |
| `commands/settings.rs` | T-028 |
| `commands/social.rs` | T-046 |
| `commands/sysop.rs` | T-055 |
| `commands/misc.rs` | T-024 and later |
| `World::energy_cycle`, `fast_update_monsters`, `spawn_cycle` | T-033/T-036, T-035 |
| `World::medium_cycle`, `slow_cycle`, `slow_cycle_end`, `fast_update_characters`, `shuffle_order`, `cycles/`, `commands/rest.rs` | T-037 |
| `World::save_cycle`, `PersistRequest` variants | T-023 |
| `World::inactivity_check`, the hang-up punishment (`state/login/leave.rs`) | T-021 |
| `cleanup/`, `on_daily_cleanup`, `session/hangup.rs` (`on_logoff`, `on_disconnect`, `cleanup_when_user_leaves`, `on_account_deleted`) | T-056 |
| `World::revert_exit` | T-026 |
| `World::restock_items` | T-041 |
| `AdminRequest`, `GameEvent` variants | T-018, T-060 |
| `session::on_shutdown` | T-059 |

## The login flow (T-021)

`bin`'s login gate (`bin/silvermere/src/gate.rs`) opens every connection in the engine
at once but sends its lines to `silvermere-auth`'s dialogue until the user is logged in;
then it reads the account's character and sends `EngineInput::LoggedIn`; a record the
engine queued for the account and the store may not have written yet wins over that read
(`World::queued_characters`, T-075). From there:

| State | Input | What happens |
|---|---|---|
| (login) | | reset of the channel's game state, `USRLOGIN`, tournament notice |
| `Start` (`compat.bbs_menu`) | `M` | MBBSEmu's clear screen, then the `Enter` arm: menu between two `ANSI_NORMAL`s |
| `Start` | `X`, then `Y`/`N`/`R` | MBBSEmu's `Are you sure (Y/N, or R to re-logon)?`; goodbye and close, the BBS menu, or `EngineOutput::Relogon` |
| `MainMenu` | `P`/`E` | game full; `player::load_player` on the held row, every entry (its notices, the hang-up pair among them); `USRFOLED`; room (T-025); prompt; `just entered the Realm.` |
| `MainMenu` | `X` | the BBS menu, or the logoff without the shim |
| `MainMenu` | empty / unknown | the menu / `Invalid Option.` and `\r[MAJORMUD]: ` |
| `Playing` | `x` | `cmd_quit`: refused at `hp <= 0`, else the meditation, `PlayerStatus::EXITING` and 6 cycles of delay |
| `Playing`, exiting | any line | `You may not perform any commands while waiting to exit!`; `BREAK` interrupts |
| (fast cycle) | | `...` per cycle; at 0, hang-up room, save, `USREXIT`, `\r[MAJORMUD]: `, `just left the Realm.` |

The hang-up (`state::login::leave::hangup`) and the inactivity hang-up follow `huprou`
and the original's inactivity rule; the penalty maths is `silvermere_rules::hangup`. The
session keeps the character it was given at login after leaving the realm, so `E`
enters again without a database read. Every `P`/`E` runs `player::load_player` on that
held row (T-023), as the original re-read the record on each entry; the
held row is the one saved at the last return to the menu, so the two
agree. `session::on_line` takes the engine's `Rng` and the day (`event_loop::day_start`)
for it.

## The command pipeline (T-024)

A line typed in the realm (`Playing`, `state/play.rs`) goes through `commands::input::execute_input`,
the eleven stages of the original's realm input, in the original's order: 1 the exit wait (T-021), 2
the queue when a delay runs or lines wait, 3 the empty line (the room again only under
`statline full`, modes 2 and 4, then the prompt), 4 `x`, 5 confusion, 6 the parser and
`handle_commands`, 7 text and remote-action exits matched against the whole line, 8 the
room's command text, 9 a spellbook name (living and unparalysed only), 10 mute and social
actions, 11 speech (`.` always says, `"` yells to the room and its neighbours, `talk fast`
says every plain line; otherwise `Your command had no effect.`). Stages 5, 7 (remote
actions), 8, 9 and 10 (actions) are trait hooks in `input.rs` (`HOOKS`) that default to
"no match" until their cards point them at a body.

`commands::parse::parse_command` accepts the same command words and abbreviations as the
original, from the table in `commands/table.rs` (one `(word, min, code)` run per accepted
abbreviation range, `open` entries for `VERB*`, `VERS*` and the punctuation prefixes, and
BORROW's ninth-letter quirk by hand); `tests/fixtures/command_words.json` is the oracle.
`commands::dispatch::dispatch` is `handle_commands`: the delay
safeguard, the mortally-wounded / paralysed gate with its 24 allowed codes, `ANSI_NORMAL`,
one arm per code into the family modules. `commands::pending` is the typed-ahead queue
(the walk warns at the 8th link, nine lines waiting, and drops at the
12th, thirteen waiting), `add_delay`/`delay_done` (with the echo off
through `EngineOutput::SetEcho`) and the delay part of `fast_update_character`:
one queued line runs per fast tick once the delay is 0, echoed as
it runs, and the exit wait is handed to T-021's `exit_countdown`. `input::on_key` is the
character-mode entry; ending a paged listing with `Command interrupted.` is a line rule
(a one-word line beginning with `X`) that T-027 carries in
`state::listing::on_line`.

## The background cycles (T-037)

`cycles/` holds the bodies the scheduler's routines run, in the original's order:

| Pass | Body | Order |
|---|---|---|
| slow (30 s) | `cycles::slow` | `shuffle_mappings` (the session order, `World::shuffled_sessions`); per player in the realm: `moves`, hunger, thirst, poison (`You feel ill.`, knock-out, death check), bleed or aid, HP regen, mana regen, `sesstime`, the prompt when HP or mana changed; per monster: poison, `hpregen`; then the scheduler's restock count, then the three direction-name letters and the random monster text |
| medium (3 s) | `cycles::medium` | `altpass` flips; per player: room spell every other pass, `attackers`/`pagecnt`/`auctcnt`/`gosscnt`, `moves` unless over 66%, `PlayerStatus::JUSTARRIVED`, the readied light's charge and the light timer, spell rounds; then the monsters |
| fast (1 s) | `cycles::fast` | per player: follow the leader (`direction_player_travelling`), rest (every 21st tick, the slow HP formula x3), meditate (every 15th, the slow mana formula without `manarate`; the wake line when full), the stun (`pvptimer`), then the command delay and queue |

`rules::regen` has the formulas. Work other cards own is reached through
`CycleHooks` (`World::cycles.hooks`): `move_user` (T-026), `check_kill_user` and
`knock_out_user` (T-033; the defaults are a `deathhp` check and the original's
knock-out), `delay_step` (by default T-024's `fast_update_delay`, which runs T-021's
exit countdown), `room_spell` (T-051), `spell_rounds` (T-050) and
`medium_update_monsters` (T-035). Tests install their own. `resttick`, `medtick` and
`sesstime` live in `PlayerSession::ticks`, never persisted.

## Not carried

Switching the current user, the per-player tile, the eleven record caches and
their preloading, the polling start-up, the cycle flags, emulation, the editor's CP outside the session,
`WCCRECOV.FLG`, the split-screen and client/server paths of `tell_user`, the host's
global-command pass in `sttrou`.
