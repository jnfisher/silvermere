# Sysop commands (`SYSOP ...` and the sysop menu)

What an operator can do in the game: the `SYSOP` command and the `[MAJORMUD Sysop]` menu,
added by T-055. The code is
`crates/silvermere-engine/src/commands/sysop.rs` (the sub-commands, in the original's order),
`display/sysop.rs` (the reports and lists) and `state/sysop_menu.rs` (the menu and its jobs).

## Roles and permissions

The original's sysop and game-operator flags are the account's `sysop` and `gameop` roles. The 21
`SYS_*` keys of `WCCMMUD.INI` are permissions (`silvermere account grant NAME SYS_GOTO_KEY`);
the original's developer gate (`CODEC_OF_TWINKS` = 78, a registration-number check, the
developer level and "beta" build checks) is the permission `DEBUG`.

An INI key was an extra lock on top of the role test and was empty by default, so:

- a **sysop** holds every `SYS_*` key;
- a **game operator** holds every `SYS_*` key except `SYS_DISBAND_KEY`, which let a game
  operator in only when it was set and held;
- anyone else holds a key only when granted it;
- **nobody** holds `DEBUG` unless granted it (`silvermere account grant NAME DEBUG`), a sysop
  included: `CODEC_OF_TWINKS` was in neither stock
  `WCCMMUD.MSG` nor `WCCMMUD.RLN`, so no stock sysop passed the developer gate. This keeps
  EXPRESS_TRAIN's room-number teleport off by default.

What is the sysop's alone is the original's own sysop-role test, kept at each sub-command. A
player who is neither gets nothing from `SYSOP`: the line goes on through `execute_input`
and ends as `Your command had no effect.`. An operator whose
gate fails falls through the chain to `Command not recognized.`.

Every `SYSOP` line is logged before dispatch as `Sysop Command by <user-id>: <line>` on the
`tracing` target `mmlog` when `log_sysop_commands` (`LOG_SYSOP_COMMANDS`) is on. Notable
actions write an audit line (target `audit`) and an `events` row of kind `sysop.<keyword>`
with the operator's user-id and a detail (`shocst` in the original).

## Sub-commands

Prefix matching, ignoring case: a word may be abbreviated, and the first sub-command in this order whose
word and gate match runs.

| Sub-command | Gate | Notes | Audit kind |
|---|---|---|---|
| `LANCE` | anyone (sic) | `Lcon n`, `Death Setting: n`; then an operator also gets `Command not recognized.` (sic). **Dropped**: the Reg#, demo, IP and SBBS lines | |
| (none) | operator | the help list; the second half for a sysop. The `INITIALIZE` line is dropped | |
| `DEBUG [OFF\|ON\|PARTIAL\|BUFFERS]` | sysop + `DEBUG` | `Sysop debug mode set to n` | |
| `MONSTER [n [room [map [min [max]]]]]` | sysop + `DEBUG` | `generate_monster` (T-035's hook) | `sysop.monster` |
| `DUMP` | sysop + `DEBUG` | the active monsters to the `mmlog` log | |
| `RESET [ROOM r [map] \| AREA \| EXPERIENCE]` | sysop + `SYS_RESET_ROOMS` | the room's or area's generation count to 0, saved at once | `sysop.reset` |
| `BUFFERS [CLEAR\|SAVE]` | operator + `SYS_BUFFERS_KEY` | no caches: the counts the world holds | |
| `RELOAD` | operator | the game options of the config file read again (`INI Settings reloaded.`) | `sysop.reload` |
| `INITIALIZE` | | **Dropped**: rebuilt the record caches; there are none | |
| `SUMMON <name>` | sysop + `DEBUG` | an item into the pack, else a spell into the book | `sysop.summon` |
| `EXPRESS_TRAIN <room> [map]` | sysop + `DEBUG` | silent move | |
| `GOTO <place>` | operator + `SYS_GOTO_KEY` | NEWHAVEN 1:2150, SILVERMERE 1:`reinc_room`, SUPPORT 1:164 (sysop only), RHUDAUR 2:2519, KHAZARAD 6:1249, LOSTCITY 16:426; no message on success; refused in autocombat, while attacked or in a PvP retaliation window. **Dropped**: the add-on purchase gates | |
| `LIGHTNING <user>` | operator + `SYS_LIGHTNING_KEY` | 25-50% of max hp to a victim at legal level 5 unless `light_evy`; knocked out below 0 | `sysop.lightning` |
| `TRANSFER` | sysop | `Info: <last line>` | |
| `VALUES [user]` | sysop + `DEBUG` (the original's developer level 2) | the AV/DV line | |
| `STATUS [ROOM r [map] \| user]` | operator + `SYS_STATUS_KEY` | a room's data or a player's status page | |
| `DISBAND <gang>` | sysop, or operator + `SYS_DISBAND_KEY` | flagged disbanded and deleted at cleanup | `sysop.disband` |
| `SPEAK <text>` | operator + `SYS_SPEAK_KEY` | a line to everyone | `sysop.speak` |
| `ARENA [STATUS\|NORMAL\|COMBAT]` | sysop | whether the arena rooms are death-free | `sysop.arena` |
| `DISABLE` / `ENABLE <gang>` | sysop + `SYS_DISABLE_GANGS` | off/on the top gangs | `sysop.disable`, `sysop.enable` |
| `GANGSIZE <n> <gang>` | sysop | the member count | `sysop.gangsize` |
| `GOD <user> ...` | operator; each option its key | `INVENTORY PROFILE SPELLS POWERS ABILITIES STATUS INFO` (`SYS_USERINFO_KEY`), `RETRAIN` (`SYS_RETRAIN_KEY`), `GOOD SAINT NEUTRAL LAWFUL UNLAWFUL` (`SYS_ALIGNMENT_KEY`), `JAIL` (`SYS_JAIL_KEY`), `ADD LIFE` (`SYS_ADDLIFE_KEY`), `ADD EVIL [n]` (`SYS_EDITEVIL_KEY`), `CLEAR SUICIDE` (`SYS_CLEARSUICIDE_KEY`), `SPEAK` (`SYS_SPEAK_KEY`), `CHANGE NAME\|GANG\|SEX` (`SYS_CHANGE_KEY`), `DISABLE\|ENABLE TOPTEN` (`SYS_TOPTEN_KEY`), `VERIFY ABILITIES` | `sysop.god` |
| `EMULATE`, `NOEMULATE` | | **Dropped**: the autoplay test mode | |
| `LIST <what>` | operator | `USERS`, `EVIL` (`SYS_LISTUSER_KEY`); `LIMITED_ITEMS`, `NEWLIMITED ...`, `GANG <gang>` (`SYS_LISTLIMIT_KEY`); with `DEBUG` and the sysop role also `ITEMS`, `KNOWN_MONSTERS`, `ACTIVE_MONSTERS [first [max]]`, `SPELLS`, `TEXT n`, `RESTOCK [shop]` | |
| `REPORT [USERS [filter] \| NOTOPTEN \| MEMORY]` | operator + `SYS_REPORT_KEY` | the paged user report (`X` stops it); `MEMORY` the process's resident set | |
| `MAP` | operator + `SYS_MAP_KEY` | the area map on screen (the original's map file was never written) | |
| `CONFIGURE SHOW \| MAXUSERS n` | sysop | `GENRATE n` and `MINWAIT n` with `DEBUG`. **Dropped**: the three credit lines of `SHOW` | `sysop.configure` |
| `TWEAK CLEAR \| <field> <value>` | sysop + `DEBUG` | the operator's own record | `sysop.tweak` |
| anything else | | `Command not recognized.` | |

## The sysop menu

The hidden word `sysop` at the `[MAJORMUD]:` menu opens it for the sysop role.

| Key | Here |
|---|---|
| `C` | clears every kept evil-point record, one a tick (`Clearing:` and the spinner); `X` cancels. Audit `sysop.clear_evil` per record |
| `CP` | the same for the saved profiles. Audit `sysop.clear_profile` |
| `R` | the original's "obsoleted" notice |
| `D` | walks every stored character (asked of `bin`), shows those below the defunct limits and deletes on `Y` (not a player online). Audit `sysop.delete_user` |
| `H` | walks every stored character with a limited item and strips them on `Y` (not a player online). Audit `sysop.strip_limited` |
| `I`, `N`, `S` | `WCCMMUD.RLN`, `WCCMMSYS.REL`, `WCCMMSYS.NOT` from the assets directory (written there by `silvermere import`, [import.md](import.md#assets)), then the main menu |
| `L` | the three defunct-user limits (minutes played, days away, experience) |
| `X` | back to the main menu |
| `U`, `B`, `E`, `A` | **Dropped**: an update file (the importer CLI instead), the bulletin and INI editors (the config file instead), add-on codes |

Here a job is session state advanced one record per engine tick.

## The login screen

Before the `Enter Username` prompt, every connection sees the login art: by default the
SILVERMERE logo below (`crates/silvermere-auth/art/LOGIN.ANS`, made by
`tools/art/make_logos.py`, T-118).

![The default login art](images/silvermere-logo.png)

To use your own screen, the way BBS sysops swapped their ANSI screens, put a file named
`LOGIN.ANS` in the assets directory, `<data_dir>/assets/LOGIN.ANS`. The server reads it at
start, so restart after changing it. An empty `LOGIN.ANS` shows no art at all.

The file must be safe for MegaMud and other DOS-era terminals:

- at most 20 lines, so the prompt below still fits on a 25-line screen;
- at most 79 columns on every line;
- CP437 text with CR LF line ends, and no control bytes but ESC, CR and LF;
- colour only with plain `ESC[...m` codes using 0, 1, 5, 30-37 and 40-47 (the 16 standard
  colours): no iCE colours, 256-colour or 24-bit codes, cursor movement or clear-screen;
- `ESC[0m` at the end.

A SAUCE record (what TheDraw, PabloDraw and Moebius append after a `0x1A` byte) is
dropped before the check. A file that breaks a rule, or cannot be read, is not sent: the
default art is, and the log says why at `WARN` (`login art not used, sending the default:
line 3: SGR parameter 38`). The checks are `silvermere_auth::art::check`;
`python3 -I tools/art/ansi2png.py LOGIN.ANS out.png` previews a file as a PNG and rejects
escape codes outside that set.
