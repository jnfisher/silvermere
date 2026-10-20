# Accounts, login and permissions

`silvermere-auth` replaces what Worldgroup (and, for the captures, MBBSEmu) supplied around
the module's idea of a user: the account record, the password check, the `Username:` /
`Password:` / `NEW` dialogue legacy login scripts drive, the keys the module tested with
`haskey`, and the sysop's account tools. `bin`'s login gate (`bin/silvermere/src/gate.rs`,
T-021) runs the dialogue on each connection before the engine sees its lines; this crate
ends at "authenticated, account loaded". Passwords travel inside the dialogue as
`password::Secret`, whose `Debug` prints `Secret(***)`.

## Accounts

| Field | Stored in | Notes |
|---|---|---|
| `id` | `accounts.id` | |
| `username` | `accounts.username` | 1-29 printable ASCII characters (32-126, what the host's `isuidc` accepted), no leading or trailing space, not `NEW`. Unique ignoring case, stored with the case given. **Immutable**: `characters.userid` keeps a copy as the join key the game looks players up by (the original's 29-character `userid`, key 0 of `WCCUSERS`), so there is no rename command. |
| `password_hash` | `accounts.password_hash` | argon2id PHC string, `argon2` crate defaults (19 MiB, 2 passes, 1 lane), 16-byte random salt. Passwords are 1-128 bytes; the BBS's 5-9 character rule is not enforced. |
| `email` | `accounts.email` | Optional; the game never reads it. |
| `sex` | bit 0 of `accounts.flags` | `M`/`F`: the game copies the account's sex into the character and picks pronouns from it. |
| `flags` | bits 1-3 of `accounts.flags` | `no_gossip`, `no_auction`, `no_loan`. |
| `roles` | `account_roles.role` (lowercase) | `player`, `sysop`, `gameop`. |
| `perms` | `account_roles.role` (INI name) | Explicit grants, e.g. `SYS_GOTO_KEY`. |
| `created_at`, `last_login_at` | `accounts.created`, `accounts.last_login` | |
| `failed_logins`, `locked_until` | the server's lockout tracker | In memory, not stored (see Lockout). |

An account is loaded once at login and read from then on, as the module set a
player's sysop flag once at entry. A role change takes effect at the
account's next login.

One account has one character (`game.characters_per_account = 1`, John 2026-10-14) and one
session at a time.

## The dialogue

The texts are MBBSEmu's (`MBBSEmu/HostProcess/HostRoutines/MenuRoutines.cs`), byte for
byte as the captures `entry-existing` and `entry-new-account` show them, except where
noted. They live in `crates/silvermere-auth/messages/auth.toml` (the `silvermere-text`
catalogue format); `\e[..m` below are the escape sequences in that file. MBBSEmu's
`login.ans` welcome screen and `signup.ans` box are not reproduced: the server sends its
own login art (the SILVERMERE logo, or the sysop's `LOGIN.ANS`; `docs/sysop.md`
"The login screen", T-118) and its `compat.login_banner` line (empty by default, then
nothing) instead. What follows the art is byte for byte as before.

```
[login art: art::DEFAULT or <data_dir>/assets/LOGIN.ANS]
[compat.login_banner]\r\n            (when set)
\r\n                                 (welcome_end: the tail of MBBSEmu's welcome screen)
\r\nEnter Username or enter "NEW" to create a new Account\r\n
Username: _
```

| Input at `Username:` | Response |
|---|---|
| blank or whitespace only | the intro line and `Username:` again; not a failed attempt. The name is otherwise not trimmed |
| more than 29 characters | `Please enter a Username with less than 30 characters.`, then the prompt again |
| `NEW` (any case), `game.open_signup = true` (default) | signup, below |
| `NEW`, signup closed | `New accounts are created by the sysop on this system.` (silvermere), prompt again |
| anything else | secure echo on, `Password: ` |

A blank or whitespace-only line at `Password:` shows `Password: ` again (silvermere); it is not a failed attempt.

At `Password:` the line editor echoes one `*` per byte typed (MBBSEmu set `wid = 0xFF`;
the module's own `echsec` call capped it at 8, but the login layer was MBBSEmu's, so its
behaviour is kept: a nine-letter password shows `*********`). Echo returns to normal when
the line is in. Then:

| Outcome | Response |
|---|---|
| right password | logged in (`State::Done`): the BBS menu shim follows (T-021) |
| unknown name or wrong password | `Invalid Credentials`, the intro and `Username:` again |
| an account with no password (an import creates its players' accounts without one) | `This account has no password yet. Ask the sysop to set one.` (silvermere), the intro and `Username:` again, whatever was typed; never logs in and never counts towards the lockout |
| third failure on this connection | `Invalid Credentials`, then the connection is closed (MBBSEmu let a client retry forever) |
| account locked | `Too many failed logins: this account is locked for N more minute(s).` (silvermere), counted as a failure of the connection |
| right password, account already logged on | `This account is already logged on.` (silvermere, `compat.duplicate_login`), connection closed |

MBBSEmu refused a second session at the username prompt (`{user} is already logged in --
only 1 connection allowed per user.`); silvermere checks after the password so that the
answer says nothing about an account to someone who does not know its password. An
unknown name costs the same argon2 work as a wrong password.

### Signup (`NEW`)

Each prompt is re-asked after its refusal; the order is MBBSEmu's.

| Prompt | Refusals |
|---|---|
| `Please enter a unique Username (Max. 29 Characters):` | blank, bad character, edge space or `NEW`: `Please enter a valid Username.`; over 29: `Please enter a Username with less than 30 characters.`; taken (any case): `That Username is unavailable, please choose another.` |
| `Please enter a strong Password:` (secure echo) | empty, whitespace-only or over 128: `Passwords must be between 1 and 128 characters long.` (silvermere; MBBSEmu's said 5 and 9); `password` in any case: `Please enter a better password. Seriously.` |
| `Please re-enter your password to confirm:` (secure echo) | different: `The passwords you entered did not match, please try again.`, back to the password prompt |
| `Please enter a valid e-Mail Address:` | blank is accepted; otherwise it needs one `@` with text either side and no spaces, or `Please enter a valid e-Mail address.` |
| `Please enter your gender 'M' or 'F' (can be changed later):` | not `M`/`F` (either case): `Please enter a valid gender selection ('M' or 'F').` |

Then the account is created (roles `player`) and logged in. MBBSEmu's NOTE paragraph
before the password prompt (SHA-512 storage, the BBS's 9-character limit) is not printed:
neither is true here.

### Shape

`LoginDialogue` is a pure state machine: `start()` and `on_line(&str)` return
`Vec<Output>` where `Output` is `Text(AuthMsg)`, `SetEcho(EchoMode)`, `Request(Request)`
or `Close`. A `Request` (authenticate, is this name free, create) is answered with
`on_reply`; `LoginService::drive` does that against an `AccountStore` (the database, or
`MemoryStore` in tests), applying lockout, the one-session rule and the audit trail. The
session releases its account with `LoginService::online().release(id)` when it ends.

## Lockout

The BBS had no such rule; the numbers are silvermere's.

| Setting | Default | Meaning |
|---|---|---|
| `lockout.max_failures` | 10 | wrong passwords for one account ... |
| `lockout.window_secs` | 900 | ... within this many seconds lock it ... |
| `lockout.lock_secs` | 900 | ... for this long. While locked even the right password is refused. |
| `LoginConfig::max_attempts` | 3 | failures per connection before it is closed |

A successful login forgets the account's failures; an expired lock is lifted with its
failures. The tracker is in memory and shared by every session: a restart lifts all locks.

## Roles and permissions

The engine asks `has_permission(&account, Perm)` where the module called `haskey`,
and `has_role(&account, Role)` where it read the player's sysop and game-operator flags.

| Original | silvermere | Default |
|---|---|---|
| `SYSOPKEY` -> the sysop flag | `Role::Sysop` | holds every permission but `DEBUG` |
| `GAMOPKEY` -> the game-operator flag | `Role::GameOp` | holds every `SYS_*` permission (T-055: SYS DISBAND's own test still wants `SYS_DISBAND_KEY` granted) |
| `CODEC_OF_TWINKS` = 78 and developer mode (a registration-number check) | `Perm::Debug` (`DEBUG`) | explicit grant only, a sysop included (`CODEC_OF_TWINKS` was in neither stock file); the developer sub-commands (`docs/sysop.md`) |
| the per-key disable check | none | never disabled a key |
| `PLAYKEY` / `NOPLAYKY` | none | dropped: everyone may play |
| `GOSSKEY` takes gossip away | `Perm::Gossip`, flag `no_gossip` | granted; sysops and game operators keep it |
| `AUCTKEY` takes auctions away | `Perm::Auction`, flag `no_auction` | granted; sysops and game operators keep it |
| `LOANKEY` needed for loans | `Perm::Loan`, flag `no_loan` | granted |
| BBS credits, `access[]`, `curcls/prmcls`, `/SYS`; `SAVEKEY`, `NODMOKEY`, `NOKICKEY`, `NOPAYKEY` | none | dropped |

The `SYS_*` keys of `WCCMMUD.INI`, each a
`Perm` serialised as its INI name. A player holds one only when granted it.

| INI key | Guards | `Perm` |
|---|---|---|
| `SYS_MAP_KEY` | SYS MAP | `SysMap` |
| `SYS_REPORT_KEY` | SYS REPORT | `SysReport` |
| `SYS_GOTO_KEY` | SYS GOTO | `SysGoto` |
| `SYS_LIGHTNING_KEY` | SYS LIGHTNING | `SysLightning` |
| `SYS_STATUS_KEY` | SYS STATUS | `SysStatus` |
| `SYS_RESET_ROOMS` | SYS RESET | `SysResetRooms` |
| `SYS_ADDLIFE_KEY` | GOD user ADD LIFE | `SysAddLife` |
| `SYS_RETRAIN_KEY` | GOD user RETRAIN | `SysRetrain` |
| `SYS_ALIGNMENT_KEY` | GOD user GOOD/SAINT/NEUTRAL/LAWFUL/UNLAWFUL | `SysAlignment` |
| `SYS_EDITEVIL_KEY` | GOD user ADD EVIL | `SysEditEvil` |
| `SYS_JAIL_KEY` | GOD user JAIL | `SysJail` |
| `SYS_SPEAK_KEY` | GOD user SPEAK | `SysSpeak` |
| `SYS_LISTUSER_KEY` | LIST USERS, LIST EVIL, LIST GANG | `SysListUser` |
| `SYS_LISTLIMIT_KEY` | LIST LIMITED_ITEMS | `SysListLimit` |
| `SYS_USERINFO_KEY` | the GOD user commands that show details | `SysUserInfo` |
| `SYS_CHANGE_KEY` | GOD user CHANGE (name or sex) | `SysChange` |
| `SYS_TOPTEN_KEY` | GOD user ENABLE/DISABLE TOPTEN | `SysTopTen` |
| `SYS_DISBAND_KEY` | SYS DISBAND gang | `SysDisband` |
| `SYS_BUFFERS_KEY` | SYS BUFFERS | `SysBuffers` |
| `SYS_CLEARSUICIDE_KEY` | GOD user CLEAR SUICIDE | `SysClearSuicide` |
| `SYS_DISABLE_GANGS` | SYS DISABLE gang | `SysDisableGangs` |

Nineteen of these end in `_KEY`; `SYS_RESET_ROOMS` and `SYS_DISABLE_GANGS` are keys too
(the module tested both with `haskey`), so all 21 are permissions.

## Command line

`silvermere account ...` (the binary) calls `silvermere_auth::cli`:

| Command | Function |
|---|---|
| `account create NAME [--sysop] [--gameop] [--password-stdin]` | `cli::create` (the password is one line on stdin) |
| `account passwd NAME` | `cli::set_password` (stdin) |
| `account passwd --missing [--dry-run] [--out FILE]` | `cli::without_password` and `cli::set_missing_password`: every account with no password (as an import creates them) gets a random one, 14 lower-case letters and digits without the look-alikes `0`, `o`, `1`, `l`, `i`; accounts that have a password are never touched. Prints `name<TAB>password` once to stdout and nothing to the log; `--out FILE` writes the lines to a new file instead (mode 600; refused if it exists, before anything changes). Each line is written and flushed before its password is stored, so a failed write (stdout closed by `| head`, a full disk) stops the run with no account given a password nobody saw; `--dry-run` lists the accounts and changes nothing. Each account gets the same `auth.password` audit line and row as `account passwd`. It is a flag of `passwd` rather than its own subcommand because it is the same change, made for the accounts that need it. |
| `account grant NAME WHAT`, `account revoke NAME WHAT` | `cli::grant` / `cli::revoke`; `WHAT` is a role (`sysop`), an INI key (`SYS_GOTO_KEY`, `GOSSIP`) or a flag (`no_gossip`) |
| `account list` | `cli::list`, one line per account: id, name, sex, roles/perms/flags |
| `account delete NAME --yes` | `cli::delete`: the character first (the hook T-023 owns), then the account and its `account_roles` rows |

## Audit

Every login, failure, lockout, signup, password and role change writes a
`tracing::info!(target: "audit", ...)` line and an `events` row:

| `kind` | When |
|---|---|
| `auth.login` | a successful login |
| `auth.login_failed` | unknown name (`reason: unknown`), wrong password (`reason: password`) or an account with no password (`reason: no_password`) |
| `auth.lockout` | a failure that locked the account (`until`) |
| `auth.login_locked` | a login refused because the account is locked |
| `auth.login_duplicate` | the right password for an account already logged on |
| `auth.signup` | an account created with `NEW` |
| `auth.account_created`, `auth.password`, `auth.role`, `auth.account_deleted` | the `cli` functions |
