//! Profiles: what an account keeps across a character's final death or suicide.
//!
//! An account keeps two things: the permanent info and the kept evil points. silvermere keeps them
//! as `character_profiles` and `evil_points` rows (T-003), held in memory in [`World::profiles`]
//! and [`World::evil_points`] and written through [`PersistRequest::Profile`] and
//! [`PersistRequest::EvilPoints`].
//!
//! Both are keyed by account. `characters_per_account` is 1, so "the account's next
//! character" and "the same account" coincide; if it ever grows above 1, the key must
//! become the character slot (needs John), as a per-account key no longer says which
//! character inherits.
//!
//! Save points: the last-life death (`check_kill_user`), the deleting suicide
//! (`commit_suicide`), both through [`retire`], and `SET STATLINE` with a custom line
//! (`cmd_set`), through [`save`]. Every caller passes `keepsome` as the player's
//! `PS_KEEPEXP` and-ed with the sysop's `keepexp` ([`keep_exp`]).

use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, SessionId};
use silvermere_types::records::{
    BigExp, Character, Class, EvilRecord, PermanentRecord, Race, UserState,
};

use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::World;

/// The account's kept profile, if any.
pub fn lookup(world: &World, account: AccountId) -> Option<&PermanentRecord> {
    world.profiles.get(&account)
}

/// The `keepsome` argument every caller of [`save`] passes: the player's `PS_KEEPEXP`
/// setting and the sysop's `keepexp` both on.
pub fn keep_exp(world: &World, c: &Character) -> bool {
    c.settings.contains(PlayerSettings::KEEPEXP) && world.config.keep_exp
}

/// The experience a profile keeps: above level 1 with `keepsome`, the larger of "the
/// experience half the level needs, plus 1" and "a tenth of the experience" (the last
/// digit of the billions moving into the top of the remainder), compared billions first;
/// otherwise nothing.
///
/// Experience is always kept on the new chart (the old chart is not carried), so the
/// 32-bit `exp` field is always 0 in a profile.
pub fn kept_exp(c: &Character, class: &Class, race: &Race, keepsome: bool) -> BigExp {
    if !keepsome || c.level <= 1 {
        return BigExp {
            expbil: 0,
            exprem: 0,
        };
    }
    let mut needed = new_calc_exp_needed((c.level / 2) as i16, class.exp + race.exp);
    needed.exprem = needed.exprem.wrapping_add(1);
    let tenth = BigExp {
        exprem: c.exprem / 10 + (c.expbil % 10) * 100_000_000,
        expbil: c.expbil / 10,
    };
    if needed.expbil == tenth.expbil {
        if needed.exprem < tenth.exprem {
            tenth
        } else {
            needed
        }
    } else if needed.expbil < tenth.expbil {
        tenth
    } else {
        needed
    }
}

/// The profile record built from a character. The unnamed bytes and words of the
/// imported profile record (at offsets 0x40, 0x42, 0x46, 0xab, 0xad) are not carried. `ust`
/// supplies the custom statline text and the day byte.
pub fn record(
    c: &Character,
    ust: &UserState,
    class: &Class,
    race: &Race,
    keepsome: bool,
) -> PermanentRecord {
    let exp = kept_exp(c, class, race, keepsome);
    PermanentRecord {
        userid: c.userid.clone(),
        deathdate: c.deathdate,
        palette: c.palette,
        msglevel: c.msglevel,
        settings: c.settings,
        pagemax: c.pagemax,
        hairlen: c.hairlen,
        haircol: c.haircol,
        eyecol: c.eyecol,
        statline: c.statline,
        fasttalk: c.fasttalk,
        verbose: c.verbose,
        exp: 0,
        firstname: c.firstname.clone(),
        lastname: c.lastname.clone(),
        flags: c.flags,
        statfmt: ust.statline.clone(),
        gossmax: c.gossmax,
        auctmax: c.auctmax,
        daybyte: ust.daybyte,
        expbil: exp.expbil,
        exprem: exp.exprem,
    }
}

/// The session's character stored as its account's profile, replacing any older one.
/// Nothing is saved without a character, an account, or the character's class and race
/// records.
pub fn save(world: &mut World, id: SessionId, keepsome: bool) {
    let Some(s) = world.sessions.get(&id) else {
        return;
    };
    let (Some(account), Some(c)) = (s.account.as_ref(), s.character.as_ref()) else {
        return;
    };
    let (Some(class), Some(race)) = (world.classes.get(&c.class), world.races.get(&c.race)) else {
        return;
    };
    let account = account.id;
    let rec = record(c, &s.ust, class, race, keepsome);
    world.profiles.insert(account, rec.clone());
    world
        .outbox
        .push(EngineOutput::Persist(PersistRequest::Profile {
            account,
            record: Box::new(rec),
        }));
}

/// The character half of restoring a kept profile: the profile's settings without
/// `PS_VALIDATED` and `PS_NOTOPTEN`, looks, modes, kept experience and names.
pub fn restore(c: &mut Character, rec: &PermanentRecord) {
    c.deathdate = rec.deathdate;
    c.palette = rec.palette;
    c.msglevel = rec.msglevel;
    c.settings = rec.settings - (PlayerSettings::VALIDATED | PlayerSettings::NOTOPTEN);
    c.flags = rec.flags;
    c.pagemax = rec.pagemax;
    c.gossmax = rec.gossmax;
    c.auctmax = rec.auctmax;
    c.hairlen = rec.hairlen;
    c.haircol = rec.haircol;
    c.eyecol = rec.eyecol;
    c.statline = rec.statline;
    c.fasttalk = rec.fasttalk;
    c.verbose = rec.verbose;
    c.expbil = rec.expbil;
    c.exprem = rec.exprem;
    c.exp = rec.exp;
    c.firstname.clone_from(&rec.firstname);
    c.lastname.clone_from(&rec.lastname);
}

/// The session half of restoring a kept profile: the custom statline text, and the day byte
/// only when the new character is made on the day the old one died. `deathdate` is the new
/// character's, restored by [`restore`].
pub fn restore_session(ust: &mut UserState, rec: &PermanentRecord, deathdate: i64, today: i64) {
    ust.statline.clone_from(&rec.statfmt);
    ust.daybyte = if today == deathdate { rec.daybyte } else { 0 };
}

/// The evil points the account's next character starts with. With `evilkeep` 0 nothing is
/// kept; otherwise all of them on the day of the previous death, else
/// `evil * evilkeep / 100`.
pub fn save_evil_points(world: &mut World, id: SessionId, today: i64) {
    let Some(s) = world.sessions.get(&id) else {
        return;
    };
    let (Some(account), Some(c)) = (s.account.as_ref(), s.character.as_ref()) else {
        return;
    };
    let evil = silvermere_rules::evil::keep_on_death(
        c.evil,
        world.config.evil_keep as i32,
        today == c.deathdate,
    );
    let account = account.id;
    let record = EvilRecord {
        userid: c.userid.clone(),
        evil,
    };
    world.evil_points.insert(account, record.clone());
    world
        .outbox
        .push(EngineOutput::Persist(PersistRequest::EvilPoints {
            account,
            record,
        }));
}

/// Why a character is retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retire {
    /// Killed with no lives left (`check_kill_user`).
    LastLife,
    /// `SUICIDE` on the last life or with `ST_SUICIDE` (`commit_suicide`).
    Suicide,
}

/// The bookkeeping of a character's permanent end, in this order: the evil points,
/// `deathdate`, the profile with kept experience, leaving the top list, then
/// `delete_player` and `clear_player`.
///
/// The gang (`remove_from_gang`, T-044), the group (`stop_following`, T-043), the bank
/// books (T-042), the messages and the return to the menu
/// belong to the death and suicide card (T-033), which calls this.
pub fn retire(world: &mut World, id: SessionId, today: i64, _why: Retire) {
    save_evil_points(world, id, today);
    let keepsome = match world.sessions.get_mut(&id) {
        Some(s) => match s.character.as_mut() {
            Some(c) => {
                c.deathdate = today;
                c.settings.contains(PlayerSettings::KEEPEXP) && world.config.keep_exp
            }
            None => return,
        },
        None => return,
    };
    save(world, id, keepsome);
    // leaving the top list, on suicide only in the original: a character deleted on his
    // last life stayed listed until the next start-up. Every deleted character leaves the
    // list here (T-039).
    world.remove_from_top_list(id);
    super::save::delete_player(world, id);
}
