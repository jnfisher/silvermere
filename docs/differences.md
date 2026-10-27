# How silvermere differs from the original

silvermere plays like MajorMUD 1.11p as players and MegaMud saw it. The rules follow the
original 1.11p. This page lists what is different. Anything not listed is meant to be the same.

- [Deleted](#deleted)
- [Product decisions](#product-decisions)
- [Changed on purpose](#changed-on-purpose)
- [Kept exact](#kept-exact)
- [Not the same numbers](#not-the-same-numbers)
- [Sysop commands and menu (T-055)](#sysop-commands-and-menu-t-055)

## Deleted

No replacement and no option. Each one is a validation, a host feature or a piece of the old host.

- Registration checks and the registration whitelist.
- Activation codes and add-on codes.
- Demo mode and the `DEMOLVL` option ([config-defaults.md](config-defaults.md)).
- Player-limit gating tied to a licence level.
- The `WCCDMA.SYS` IP whitelist.
- BBS credits and every "not enough credits" path.
- The Worldgroup client/server agent.
- Split-screen mode and the emulation test mode.
- The vacation stub.
- The mail hooks.
- `WCCRECOV.FLG` crash recovery. The server saves through a persistence worker instead.
- BBS pages from other modules.
- The host commands `/P`, `/#` and `/SYS`, and all other host global commands. Who is online,
  broadcast and sysop actions are `silvermere sysop` and the [admin API](admin-api.md).
- The second line of the "game full" message, which asked the sysop to upgrade a licence.

## Product decisions

Stated here so they are not read as differences from a stock install.

- **Death at -200.** `game.death_hp = -200`, the value the distribution shipped (the compiled-in
  default was -10).
- **One character per account.** `game.characters_per_account = 1`
  ([config.md](config.md), [defaults](../crates/silvermere-types/src/config.rs)).
- **Open signup** and a **3600 second inactive timeout** are under Changed on purpose below.

## Changed on purpose

- **Accounts.** Passwords are stored with argon2id, failed logins lock an account for a while, and
  passwords are 1-128 bytes with no 5-9 character limit. See [auth.md](auth.md),
  [lockout code](../crates/silvermere-auth/src/lockout.rs).
- **Empty login input.** A blank or whitespace-only line at `Username:` or `Password:` asks the same
  prompt again and is not a failed attempt, so a stray Enter from a login script is harmless
  ([login code](../crates/silvermere-auth/src/login.rs)). Whitespace-only passwords are refused at
  signup and `account set-password`.
- **Open signup.** Anyone may create an account with `NEW` (`game.open_signup = true`);
  [signup code](../crates/silvermere-auth/src/signup.rs). Set it to `false` and only a sysop can.
- **Inactive timeout 3600 seconds.** `game.inactive_secs = 3600`. The login dialogue has its own
  timeout, `net.login_timeout_secs = 300`.
- **`go nightblack portal` moves the player.** In the Negative Power Plane (8/984) the long phrase
  `go nightblack portal` now teleports to 1/2683 like `go portal`, `enter portal` and
  `enter nightblack portal`. In the original that line lacked its `teleport`, so it showed the
  portal messages and did nothing. The fix is applied as content loads, only to the exact stock
  line ([code](../crates/silvermere-engine/src/stock_faults.rs)).
- **Add-ons are on by default.** `game.addons = ["all"]` replaces the add-on codes and DLL hooks.
- **The BBS menu is a shim.** `compat.bbs_menu = true` shows the `Make your selection (X to exit):`
  menu that legacy login scripts expect; `M` enters MajorMUD, `X` logs off
  ([menu code](../crates/silvermere-engine/src/state/menu.rs)). Set it to `false` to go straight to the `[MAJORMUD]:` menu.
- **The daily cleanup runs with everyone on line.** The original's BBS took the system down at its
  cleanup hour. silvermere runs the same four passes (gang-house tax, the floor sweep, evil
  forgiveness and the session minutes, empty gangs) in the process at `game.cleanup.at` (default
  `"03:00"` local): nobody is logged off, refused or told. A player standing in a house that closes
  sees the notice and the new room at once instead of at the next login
  ([cleanup code](../crates/silvermere-engine/src/cleanup/mod.rs)). Because the original came back
  up after its cleanup, its start-up reset of the rooms also ran every night; silvermere's
  cleanup ends with that same reset, so the server never needs a restart for it: each room's
  recent-kill and found-hidden-item markers are cleared, its placed items that are gone are put
  back (the alder chest of the Hidden Keep Treasure Room, for example), Doors, Gates and Key exits
  lock, disarmed traps re-arm, found hidden exits hide again and Timed exits restart their timer.
  Players standing in a room are not told; a door that locks under them is simply locked. Shop
  slots with no restock interval also get start-up's one roll (topped up, or cut back to their
  ceiling); timed slots keep their own queued restocks. The start-up's Shop-room fill and top list
  build follow: each Shop room gets its keeper back and its pool's monsters up to its cap, and
  players there see them arrive. A Shop room whose monsters were killed shortly before the cleanup
  is refilled too, as it was after the original's restart. The top list is rebuilt, so a character
  taken off it or a sysop's experience edit shows the next day. The report and the `cleanup.complete` audit line count the
  placed items put back, the exits reset, the shop slots topped up, the monsters placed in Shop
  rooms and the top list's entries.
- **Groups, following and dragging are not saved.** They live in the session only.
- **Name validation finishes at once** instead of over several polling passes, with the same message.
- **`set utf8 on`.** A player can switch a session to UTF-8 output; the default stays CP437
  ([session code](../crates/silvermere-engine/src/player_session.rs)).
- **Configuration** is TOML and `SILVERMERE_*` environment variables instead of `WCCMMUD.INI` and the
  `WCCMMUD.MSG` options. Out-of-range values are rejected, not clamped ([config.md](config.md)).
- **Logs** are `tracing` output, text or JSON, in place of the old log files.
- **Shutdown** saves every character on SIGTERM ([metrics.md](metrics.md#shutdown-and-backups),
  [test](../bin/silvermere/tests/shutdown.rs)).
- **The character editor** is the original's line-mode path. The full-screen ANSI editor is not built yet.
- **FEEDBACK is an audit event, not mail.** The original mailed the comments to WCC support through
  the BBS's mail. silvermere has no BBS mail: the same text
  ("user-id -- name (map/room)", `INP:` and the typed line, the version) goes out as a `feedback`
  audit event and on the `audit` log target. The player still reads "email sent"
  ([comms code](../crates/silvermere-engine/src/commands/comms.rs)).
- **The text files are installed by import, and kept.** The original's install step `WCCMISC.BAT`
  unpacked the gang house, emblem, banner, statue and stone descriptions with `pkunzip -o`,
  overwriting any edited copy, and the DLL read them (and the map files) from the BBS directory on
  every use (`tfsopn`). `silvermere import` writes them into `data/assets/` and
  keeps files already there unless `--overwrite-assets`, so a sysop's edited descriptions survive a
  re-import; the server reads the directory once when it starts ([import.md](import.md#assets)).
- **`FILE DESCRIPTION` text follows MBBSEmu's text-file scanner** *(inferred)*. The kernel's
  `tfsrdl`/`tfspfx`/`tfspst` behaviour is not known, so silvermere does what MBBSEmu does:
  each line loses its leading and trailing blanks, and the file's text ends at its first blank
  line. A real GSBL kernel may differ ([room code](../crates/silvermere-engine/src/display/room.rs)).
- **MAP sends a `%` as it stands.** The original passed each line of a room's map file to `prf` as its
  format string, so a `%` in the file was read as a format. silvermere sends the
  line literally; no stock map file has a `%` ([info code](../crates/silvermere-engine/src/commands/info.rs)).

## Kept exact

These match the original and are checked against it.

- Screens and message text, the prompt and statline, and their modes
  ([text.md](text.md)).
- Every formula: combat, spells, regeneration, experience, prices
  ([rules code](../crates/silvermere-rules/src/lib.rs)).
- Timer periods and the order of work in each tick ([engine.md](engine.md)).
- Spawn rules ([spawn code](../crates/silvermere-rules/src/spawn.rs)).
- The pending command queue and command delay
  ([pending code](../crates/silvermere-engine/src/commands/pending.rs)).
- Flood control and the inactivity kick with its two strikes.
- Hang-up penalties (`PUNHUP`, `MINHUPHP`, `MAXHUPHP`, `MAXHUPIT`)
  ([hang-up code](../crates/silvermere-rules/src/hangup.rs)).
- The MajorMUD menu, all `set` options, the in-game sysop commands (minus licensing), help topics,
  and the world data: rooms, items, monsters, spells, shops, texts, actions ([import.md](import.md)).
- The Crystal Lake and the Ancient Galleon (map 17), as the stock data has them
  ([galleon tests](../crates/silvermere-testkit/tests/stock_galleon.rs)). At level 50 and above,
  a third of the lake never sweeps a player aboard the galleon and another third almost never
  does: row to the waters near the Arlysia docks to be found by it. Every lake room has a way
  there by ordinary moves.

## Not the same numbers

The original's random sequence is not reproduced. The distributions are the same, so fights
and drops behave alike over time, but a given seed does not give the original's rolls
([rng code](../crates/silvermere-rules/src/rng.rs)). `serve --seed N` makes a run repeatable.

## Sysop commands and menu (T-055)

| Original | silvermere | Why |
|---|---|---|
| `SYSOP LANCE` prints `Demo mode enabled`, `Reg# %s`, `IP: :%s:`, `SBBS Active` | only `Lcon` and `Death Setting` | registration and host diagnostics |
| `SYSOP INITIALIZE [rate [RECOVER]]` and its help line | `Command not recognized.`; the help line is gone | rebuilt the record caches; there are none |
| `SYSOP EMULATE` / `NOEMULATE` | `Command not recognized.` | the autoplay test mode, deleted outright; unreachable in 1.11p anyway (developer level 2) |
| `SYSOP GOTO RHUDAUR/KHAZARAD/LOSTCITY` refused with `You must purchase the ...` without the add-on | the move always happens | add-on activation is licensing |
| `CODEC_OF_TWINKS` = 78, developer mode (a registration-number check), developer level 2 and the "beta" check of DUMP | the `DEBUG` permission | licensing and build switches; the developer sub-commands stay reachable for whoever holds `DEBUG` |
| every demo-mode guard | none | demo mode is licensing |
| `CONFIGURE SHOW`'s `Normal Credit Consumption`, `Exempt Credit Conusmption`, `Free time for exempt users` | not printed | no BBS credits |
| `CONFIGURE SHOW`'s `pollrate`, `initrate`, `Logfile` | `0`, `0`, `tracing` | the record caches' pace and the log file do not exist; the lines stay for scripts |
| `CONFIGURE SHOW`'s `Monster Generation Rate` and `Minutes before regenerating` (developer mode only) | printed for a `DEBUG` holder | the original's developer mode is a registration-number check that MBBSEmu does not satisfy even with `CODEC_OF_TWINKS=78` (live m4, 2026-10-16: the original prints neither); the m4 differential counts the two lines as a T-055 divergence on silvermere's side |
| `BUFFERS` statistics | the record counts the world holds | no caches |
| `REPORT MEMORY` did nothing | the process's resident set | a real figure in place of an empty line |
| `SYSOP MAP` drew into a temporary map file (never written) and the screen | the screen | the file was never used |
| the log file / `shocst` | `tracing` targets `mmlog` / `audit` and an `events` row `sysop.<keyword>` | no log file or host audit trail |
| the sysop status line on the console | none | a developer-mode console display |
| menu `U - Apply an update` | not shown, the key gives the prompt again | the importer CLI (T-009) |
| menu `B - Edit the sysop bulletin file`, `E - Edit the wccmmud.ini file` | not shown | the kernel's editors; the config file instead |
| menu `A - Addon module activation codes` | not shown | licensing |
| menu `I`, `N`, `S`: the kernel's `listing` with page breaks | the file from the assets directory, no page breaks | the host's paging |
| the `D` check's `Last logged in on ... BBS Class: %s/%s` line and `Cannot find %s -- Delete?` | not printed / never asked | no BBS account record or classes; a character cannot outlive its account |
| `D` deletes a character that is online | an online character is kept | its next save would bring it back |
| `NEWLIMITED` looks the item name up by the ownership record id (sic) | by the item number | the record ids are database ids here |
| `GOD user INFO` shows the bank books first | not yet | `display_users_bankbooks` writes to its own player; TODO(T-042) |
| `LIST EVIL`'s evil timers | not listed | the timers are T-038's |
| a stray `%` in sysop-edited help or message text (`185% ` in a custom race table) | printed as written | the original used the text as a printf format, so `% ` began a conversion and the player saw something else; only `%s`, `%c` and `%%` mean anything in these files, and a realm's own file no longer fails to load over one (a warning at load counts them) |
