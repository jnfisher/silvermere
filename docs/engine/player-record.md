# The player record

T-023's part of the engine (`crates/silvermere-engine/src/player/`): creating a character,
loading a stored one, deriving its secondary stats, saving it at the original's save
points, and the profile an account keeps across a character's permanent end. The engine
around it is in `docs/engine.md`.

## Modules

| Module | Covers | Callers |
|---|---|---|
| `player::create` | `create_player`, `roll_stats`, `class_title` | T-022 (creation), `load` |
| `player::load` | `load_player`, `reduce_level_to_exp` | T-021 (entering the realm) |
| `player::save` | `save_player`, `delete_player`, `save_characters` | every save point, `World::save_cycle`, `session::on_logoff` |
| `player::secondary` | pure `secondary`, wrapper `recalculate` | everything that changes stats, level, abilities or worn items |
| `player::profile` | `save`, `restore`, `restore_session`, `save_evil_points`, `retire` | T-033 (death, suicide), T-028 (`SET STATLINE`), `create` |

## Persistence

The engine never touches the database. A save is an `EngineOutput::Persist` request on the
persistence channel; `bin` (`bin/silvermere/src/store.rs`, fed in order by the bridge)
maps it to `silvermere-db`'s `Write` and the persistence worker, whose status the admin
health probe watches. The same task answers `EngineOutput::NameCheck` from
`characters.by_first_name` through `EngineInput::Custom`:

| Request | When | Row |
|---|---|---|
| `PersistRequest::Character { account, character }` | `save_player` | `characters` (+ inventory, spells), upsert by account |
| `PersistRequest::DeleteCharacter { account }` | `delete_player` | `characters` |
| `PersistRequest::Profile { account, record }` | `profile::save` | `character_profiles` |
| `PersistRequest::EvilPoints { account, record }` | `profile::save_evil_points` | `evil_points` |

All four are keyed by account: `characters_per_account` is 1. If that ever changes, the
profile key (account or character slot) needs a decision from John.

The flush after a logoff sends the logoff's save in the same event (`Engine::flush`).

Every character write and deletion goes through `player::queue_character` or
`player::queue_character_delete`, which also keep the record (or the deletion) in
`World::queued_characters` (T-075). The store writes behind, and the gate reads a login's
character before the engine sees the login, so a login straight after a hang-up could read
the row from before it: no notice, no penalty, the dropped items back in the pack.
`state::login::logged_in` takes the queued record instead of the gate's read and drops it.
All character rows are written by the engine, in order, so the queued record is never
older than the stored row.

The offline readers take the queued record first as well (T-088): the cleanup scan
(`cleanup::on_scan`), gang offline edits (`gangs::offline_edit`), the sysop's user checks
(`sysop_menu::on_roster`, and the item check's read again at Y) and the
account deletion's read (`session::on_account_scan`). The store answers a read after a
flush, so the read misses only writes queued after it was asked, and those are still
queued when the answer arrives: `player::queued_or_stored` replaces the row with the
queued record, or drops it for a queued deletion. Without this, an offline edit made in the
window after a hang-up rebuilt the record from the row before it and wrote it back,
losing the penalty and `PlayerStatus::HUNGUP`.

Each entry carries its place in the engine's count of character writes and deletions
(`World::character_writes`). The store counts the same requests in the same order; when
its job queue is empty it flushes the worker and sends the count committed so far
(`player::on_characters_written`), which drops every entry at or below it. A newer write
for the same account has a higher number, so an older acknowledgement never drops it.
The gate may have read a login's row before the commit and still be on its way to the
engine; that login belongs to a connection already in the login dialogue (no account,
`SessionState::Start`) when the acknowledgement arrives, so the acknowledgement waits until
each of those connections has logged in or gone (`player::settle_characters_written`, run
by each acknowledgement and each fast tick).
Acknowledgements can be held up, by a connection idling at the login prompt or by a store
whose job queue is never empty. That only delays the drop: entries stay one per account, and
the next login for an account takes its entry anyway.
After a failed character request the store acknowledges nothing more, and the entries
then live until the account's next login, as before (one record per account at most).
Shutdown (`finrou`, T-059) saves every character and sends `PersistRequest::Flush`.

`save_player` writes nothing for a session without a character, a character that is not
complete (`PlayerSession::created`, set by `load_player` and by creation) or one whose
name is not validated (`PlayerSettings::VALIDATED`). Before writing it strips the
level-gated abilities, as `load_player` does.

## Save points

The original saves a character at 30 points. Each is a `SavePoint`; the card that carries
the event calls `player::save_at(world, id, SavePoint::X)` where the original saved.

| When | `SavePoint` | Card |
|---|---|---|
| an add-on gave a life | `AddonLife` | not carried (no add-ons) |
| an add-on changed the coins | `AddonWealth` | not carried |
| out of credits | `OutOfCreditsSlow` | not carried (no credits) |
| out of credits | `OutOfCreditsExempt` | not carried |
| the `QUIT` meditation ended | `QuitMeditation` | T-021, T-037 |
| the 60 s sweep | `Sweep` | T-023 (`World::save_cycle`) |
| left a gang | `GangLeave` | T-044 |
| joined a gang | `GangJoinCommand` | T-044 |
| trained a level | `TrainLevel` | T-039 |
| gave coins: the giver | `GiveCoinsGiver` | T-030 |
| gave coins: the receiver | `GiveCoinsReceiver` | T-030 |
| gave an item: the giver | `GiveItemGiver` | T-030 |
| gave an item: the receiver | `GiveItemReceiver` | T-030 |
| `PAGE` toggled | `PageToggle` | the command's card |
| the new name was validated | `NameValidated` | T-022 |
| entered the realm | `EnterRealm` | T-022 |
| back at the main menu | `MainMenu` | T-021 |
| logoff | `Logoff` | T-023 (`session::on_logoff`) |
| hang-up, first save | `HangupFirst` | T-056 |
| hang-up, after leaving the realm | `HangupLeave` | T-056 |
| joined a gang | `GangJoin` | T-044 |
| gang leader paid | `GangLeaderPaid` | T-039 |
| shared coins: each receiver | `ShareReceiver` | T-043 |
| shared coins: the group | `ShareGroup` | T-043 |
| shared coins: the sharer | `ShareSharer` | T-043 |
| the sysop's user refresh | `SysopRefreshUsers` | T-055 |
| the sysop's user refresh | `SysopRefreshNone` | T-055 |
| died with lives left | `DeathWithLives` | T-033 |
| bank deposit | `BankDeposit` | T-042 |
| bank withdrawal | `BankWithdraw` | T-042 |

The deleting deaths call `delete_player` instead, through `profile::retire`. Until T-056
carries `huprou`, a hang-up is a logoff and saves once.

### The sweep

`save_characters`: from `World::save_next`, look at up to
`MAXSAVECHK` (300) session slots and save up to `BUFTOSAV` (5) characters in the game; past
the last slot the next sweep starts at 0. A sweep never wraps within itself. The routine
runs every `sec_oth` seconds rounded down to whole minutes, at least 60
(`Periods::from_config`). The batch size is the constant `player::save::BUFTOSAV` (5,
the stock value); `GameConfig` has no option for it (John's decision).

## Loading

Every `P`/`E` at the main menu runs `load_player` on the row the session holds
(`state::login::enter::enter_realm`), not only the first entry of a
login: the original re-read the record each time, and the held row is the one saved at
the last return to the menu. A row that does not load is logged and the
menu is shown again. The engine passes its `Rng` and the day (`event_loop::day_start`,
the UTC midnight) down through `session::on_line`.

`load_player` takes the row the caller read and returns `LoadError::Version` for any record
version but 22: the importer (T-008/T-009) converts older records, so the 22 version steps,
the experience restructuring, the old-ability conversion and its quest-ability retrofit, the
wealth cap (only inside the version-10 step) and the add-on map eviction are not done here.
What is, in order:

1. The session state of the last user is cleared.
2. An unknown race becomes 10, an unknown class 14.
3. Smash, Perfect Stealth and Meditate are stripped unless the class keeps them at this
   level; See Hidden is stripped from every race but 13.
4. `reduce_level_to_exp`.
5. No group, rank middle, no movement.
6. Notices, in this order: the hang-up pair, then `PlayerFlags::PROMOTED`, `DEMOTED`,
   `HOUSECLOSED`, `HOUSEITEMS`, then a disbanded or vanished gang, then the stat reset, and last the one-life warning.
   `status` and `status2` are cleared after the hang-up check.
7. Chat limits default to `def_chat_rate`; a readied item that is gone or no
   longer a light is put away.
8. Stats more than 50 above the race maximum or below `max(min-30, 5)` reset
   the character: `PlayerStatus2::STATRESET`, the reincarnation room on map 1 (the evil one from 40
   evil points), spells ended, `roll_stats`, and the character points of every level
   returned.
9. Random hit points above `hplevel*level`, or 0, are rolled again;
   `totalcp` is recomputed; mana and hit points are capped;
   `recalculate` with `CalcMode::KeepCurrent`.
10. `lastdate`, `PlayerSettings::VALIDATED`, the palette, evil and key-slot repairs, the
    class title, `PlayerSettings::OLDBLOCKENT`, the custom statline text from the
    profile, and the spell book packed.

## Secondary stats

`secondary(&Character, &Class, &Race, &dyn AbilityLookup) -> Secondary` is pure and reads
ability totals only through T-032's `AbilityLookup`. `recalculate(&World, &mut Character,
CalcMode)` first copies T-032's `DynamicStats` into the record and merges the two status
words (`status` minus `PlayerStatus::FROMABILS`, plus the abilities' status bits; the same
for `status2`), then writes
the figures:

| `CalcMode` | Effect |
|---|---|
| `Refill` | everything recomputed, hit points and mana filled |
| `KeepMaxHp` | `maxhp` kept; `maxmana`/`maxmr` raised to cover the current values |
| `KeepCurrent` | maximums recomputed, current hit points and mana untouched |

The original computes in 16 bits: results are truncated to the record's 16-bit fields, and
the one product that can overflow before its division, `(bashea-50)*level/16`, is wrapped
to 16 bits first. A class without magery holds `spellcast = level*2 - 150`;
the stat display does not show it for such a class.

## Profiles

`profile::retire(world, id, today, Retire::LastLife | Retire::Suicide)` is the bookkeeping
of a permanent end in the original's order: evil points kept (`evilkeep` percent unless
the previous death was today), `deathdate = today`, the profile (with kept experience when
both the player's `PlayerSettings::KEEPEXP` and the sysop's `keep_exp` are on), the reroll removed for a
suicide (T-039), then the character row deleted. Gang, group and bank-book removal, the
messages and the return to the menu belong to the calling cards.

Kept experience: above level 1, the larger of
`new_calc_exp_needed(level/2) + 1` and a tenth of the experience, compared billions first.
`create_player` restores the account's profile (`profile::restore`) without
`PlayerSettings::VALIDATED` and `NOTOPTEN`, and its kept evil points; `profile::restore_session` puts
the custom statline text and (on the day of the death only) the day byte into the session.
