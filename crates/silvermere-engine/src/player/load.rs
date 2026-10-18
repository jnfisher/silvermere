//! Loading a stored character into a session: [`load_player`].
//!
//! The caller (T-021, entering the realm) reads the account's character row and hands it
//! over; [`load_player`] makes the repairs and derived values due on entry and puts the character
//! in the session. Every notice goes to the player in the original's order: the hang-up pair, the
//! gang notices by `PF_` flag, the gang-gone notices, the stat reset, and the last-life warning.
//!
//! Not carried (the importer, T-008/T-009, writes version-22 rows, so a row of any other
//! version is refused with [`LoadError::Version`]):
//!
//! - the record-version conversions for versions 0 to 21, with the wealth cap and the
//!   evil-point notices that only ran inside them;
//! - the restructuring of pre-1.11 experience and the old-ability conversion, whose
//!   quest-ability retrofit is a one-time data fix;
//! - the move out of an add-on map the BBS has not activated.
//!
//! Left to their cards: ending the spells of a reset character (T-049), the carried
//! weight and `update_allowed_worn_items` (T-030).

use silvermere_rules::Rng;
use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, ItemType, StatlineMode};
use silvermere_types::flags::{
    GangFlags, PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2, SessionFlags,
};
use silvermere_types::ids::{ClassNo, MapId, RaceNo, RoomId, SessionId, SpellNo};
use silvermere_types::records::Character;
use silvermere_types::sizes::{NKEYS, NSPELLCAST};

use super::create::{class_title, roll_stats};
use super::secondary::{CalcMode, recalculate};
use super::{EVIL_OUTLAW, PLAYER_VERSION, RANK_MIDDLE};
use crate::player_session::Party;
use crate::tell::tell_user;
use crate::world::World;

/// The race an unknown race becomes (Half-Ogre in the stock data).
pub const DEFAULT_RACE: RaceNo = RaceNo(10);
/// The class an unknown class becomes (Ranger in the stock data).
pub const DEFAULT_CLASS: ClassNo = ClassNo(14);
/// The only race that keeps See Hidden (the Gaunt One).
pub const SEE_HIDDEN_RACE: RaceNo = RaceNo(13);

/// Why a character could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("no session {0:?}")]
    NoSession(SessionId),
    /// Only version-22 rows are loaded; the importer converts older records.
    #[error("character record version {0}, expected {PLAYER_VERSION}")]
    Version(i16),
}

/// Character points a level brings: 10 for levels 2-10, 15 for 11-20, then 5 more every
/// ten levels.
fn cp_for_level(level: i32) -> i32 {
    if level <= 10 {
        10
    } else if level <= 20 {
        15
    } else {
        ((level - 1) / 10 - 2) * 5 + 20
    }
}

/// While the experience does not pay for the current level, take a level away and mark
/// the character for retraining (`cp` -1, `randomhp` 0). Level 1 is the floor; nothing
/// happens without the class and race records.
pub fn reduce_level_to_exp(world: &World, c: &mut Character) {
    // levels stop at exactly 1; a level-0 record cannot exist, and here it would
    // underflow.
    if c.level <= 1 {
        return;
    }
    let (Some(class), Some(race)) = (world.classes.get(&c.class), world.races.get(&c.race)) else {
        return;
    };
    loop {
        let needed = new_calc_exp_needed(c.level as i16 - 1, class.exp + race.exp);
        let short =
            needed.expbil > c.expbil || (needed.expbil == c.expbil && needed.exprem > c.exprem);
        if !short {
            break;
        }
        c.level -= 1;
        c.cp = -1;
        c.randomhp = 0;
        if c.level <= 1 {
            break;
        }
    }
}

/// Whether a stat is out of the race's range: more than 50 above its maximum or below
/// its minimum less 30 (never below 5); fixed bounds without a race record.
fn stats_out_of_range(world: &World, c: &Character) -> bool {
    let stats = [c.hea, c.agi, c.cha, c.intel, c.wil, c.str].map(i32::from);
    match world.races.get(&c.race) {
        Some(r) => {
            let max = [r.maxhea, r.maxagi, r.maxcha, r.maxint, r.maxwil, r.maxstr];
            let min = [r.minhea, r.minagi, r.mincha, r.minint, r.minwil, r.minstr];
            stats
                .iter()
                .zip(max)
                .zip(min)
                .any(|((&s, max), min)| s > i32::from(max) + 50 || s < (i32::from(min) - 30).max(5))
        }
        None => {
            let max = [120, 120, 120, 120, 120, 150];
            stats.iter().zip(max).any(|(&s, max)| s > max || s < 5)
        }
    }
}

/// Loads a version-22 record: the repairs, the notices, the derived values, then the
/// character goes into the session, which is marked created. `today` is the day as the
/// database stores dates; `rng` rolls the random hit points when they must be rolled
/// again.
pub fn load_player(
    world: &mut World,
    id: SessionId,
    mut c: Character,
    today: i64,
) -> Result<(), LoadError> {
    if !world.sessions.contains_key(&id) {
        return Err(LoadError::NoSession(id));
    }
    if c.version != PLAYER_VERSION {
        return Err(LoadError::Version(c.version));
    }
    let mut notices = Vec::new();

    // forget the channel's last user.
    if let Some(s) = world.sessions.get_mut(&id) {
        s.ust.flags = SessionFlags::empty();
        s.ust.listmax = 0;
        s.ust.listidx = 0;
        s.ust.pending.clear();
    }
    if !world.races.contains_key(&c.race) {
        c.race = DEFAULT_RACE;
    }
    if !world.classes.contains_key(&c.class) {
        c.class = DEFAULT_CLASS;
    }
    super::save::strip_level_gated(&mut c);
    reduce_level_to_exp(world, &mut c);
    // nobody is in a group or moving on entry (the group is session state).
    if let Some(s) = world.sessions.get_mut(&id) {
        s.party = Party::from_record(c.dragging);
    }
    c.rank = RANK_MIDDLE;
    c.movedir = -1;
    if c.status.contains(PlayerStatus::HUNGUP) {
        notices.push(Msg::EnterHangupNote);
        notices.push(Msg::EnterHangupPunished);
    }
    c.status = PlayerStatus::empty();
    c.status2 = PlayerStatus2::empty();
    // gang notices set while offline.
    if c.flags.contains(PlayerFlags::PROMOTED) {
        c.flags |= PlayerFlags::LIEUTENANT;
        c.flags -= PlayerFlags::PROMOTED;
        notices.push(Msg::EnterPromoted);
    }
    if c.flags.contains(PlayerFlags::DEMOTED) {
        c.flags -= PlayerFlags::LIEUTENANT | PlayerFlags::DEMOTED;
        notices.push(Msg::EnterDemoted);
    }
    if c.flags.contains(PlayerFlags::HOUSECLOSED) {
        c.flags -= PlayerFlags::HOUSECLOSED;
        notices.push(Msg::EnterHouseClosed);
    }
    if c.flags.contains(PlayerFlags::HOUSEITEMS) {
        c.flags -= PlayerFlags::HOUSEITEMS;
        notices.push(Msg::EnterHouseItems);
    }
    let rate = world.config.def_chat_rate as u8;
    for max in [&mut c.pagemax, &mut c.gossmax, &mut c.auctmax] {
        if *max == 0 {
            *max = rate;
        }
    }
    // a readied item that is gone or no longer a light.
    if c.readied != -1 {
        let light = usize::try_from(c.readied)
            .ok()
            .and_then(|slot| c.items.get(slot))
            .and_then(|n| world.items.get(n))
            .is_some_and(|item| item.item_type == ItemType::Light);
        if !light {
            c.readied = -1;
            c.lighttime = 0;
            c.light = 0;
        }
    }
    // is the gang still there?
    let mut gang_changed = None;
    if !c.gang.is_empty() {
        match world.gangs.get_mut(&c.gang.to_ascii_uppercase()) {
            Some(g) => {
                if g.flags.contains(GangFlags::DISBANDED) {
                    c.flags -= PlayerFlags::LIEUTENANT;
                    g.members = g.members.wrapping_sub(1);
                    gang_changed = Some(g.key.clone());
                    notices.push(Msg::EnterGangDisbanded {
                        gang: g.name.clone(),
                    });
                    c.gang.clear();
                }
            }
            None => {
                if c.flags.contains(PlayerFlags::LIEUTENANT) {
                    c.flags -= PlayerFlags::LIEUTENANT;
                    notices.push(Msg::EnterGangRankStripped);
                }
                notices.push(Msg::EnterGangGone {
                    gang: c.gang.clone(),
                });
                c.gang.clear();
            }
        }
    }
    // the gang record changed: save it
    if let Some(key) = gang_changed {
        crate::gangs::save_gang(world, &key);
    }
    if stats_out_of_range(world, &c) {
        c.cp = -1;
    }
    // retrain.
    if c.cp < 0 {
        notices.push(Msg::EnterStatsReset);
        c.status2 |= PlayerStatus2::STATRESET;
        c.room = RoomId(if c.evil >= EVIL_OUTLAW {
            world.config.evil_reinc_room
        } else {
            world.config.reinc_room
        });
        c.map = MapId(1);
        // (the termination messages and effects are T-049's).
        c.spells = [SpellNo(0); NSPELLCAST];
        c.spellvals = [0; NSPELLCAST];
        c.spellrnds = [0; NSPELLCAST];
        let sex = c.sex;
        roll_stats(world, &mut c, sex);
        // the points of every level since 1; the running maxima stop a wrap.
        let (mut lastcp, mut lasttotcp) = (0i32, 0i32);
        for level in 2..=i32::from(c.level) {
            let gain = cp_for_level(level);
            c.cp = c.cp.wrapping_add(gain as i16);
            c.totalcp = c.totalcp.wrapping_add(gain as u16);
            if i32::from(c.cp) < lastcp {
                c.cp = lastcp as i16;
            } else {
                lastcp = i32::from(c.cp);
            }
            // `totalcp` is unsigned but compared signed.
            if i32::from(c.totalcp as i16) < lasttotcp {
                c.totalcp = lasttotcp as u16;
            } else {
                lasttotcp = i32::from(c.totalcp as i16);
            }
        }
    }
    // impossible or missing random hit points are rolled again.
    if let Some(class) = world.classes.get(&c.class) {
        if i32::from(class.hplevel) * i32::from(c.level) < i32::from(c.randomhp) {
            c.randomhp = 0;
        }
        if c.randomhp == 0 {
            c.randomhp = class.hplevel as u8;
            let mut lastrhp = 0u8;
            for _ in 1..c.level {
                let roll = world.rng.range(0, (i32::from(class.hplevel) + 1).max(0));
                c.randomhp = (roll as u8).wrapping_add(c.randomhp);
                if c.randomhp < lastrhp {
                    c.randomhp = lastrhp;
                } else {
                    lastrhp = c.randomhp;
                }
            }
        }
    }
    // the total of character points ever earned.
    if let Some(race) = world.races.get(&c.race) {
        let mut total = race.basecp as u16;
        for level in 2..=i32::from(c.level) {
            total = total.wrapping_add(cp_for_level(level) as u16);
        }
        c.totalcp = total;
    }
    // (`maxmana` unsigned: the comparison is unsigned).
    if c.mana as u16 > c.maxmana {
        c.mana = c.maxmana as i16;
    }
    if c.hp > c.maxhp {
        c.hp = c.maxhp;
    }
    recalculate(world, &mut c, CalcMode::KeepCurrent);
    // (`INACTWARN` went with the session flags cleared above).
    c.version = PLAYER_VERSION;
    c.moves = 0;
    c.lastdate = today;
    c.settings |= PlayerSettings::VALIDATED;
    if c.palette >= 4 {
        c.palette = 0;
    }
    if c.evil > 312 {
        c.evil = 312;
    }
    if usize::from(c.nkeyslots) < NKEYS {
        c.nkeyslots = NKEYS as u8;
    }
    if c.keys.len() < NKEYS {
        c.keys.resize(NKEYS, Default::default());
        c.keyuses.resize(NKEYS, -2);
    }
    c.title = class_title(world, c.class, c.level, c.sex);
    if c.settings.contains(PlayerSettings::OLDBLOCKENT) {
        c.flags |= PlayerFlags::NOENTRYMSG;
        c.settings -= PlayerSettings::OLDBLOCKENT;
    }
    // the custom statline text from the kept profile.
    if matches!(c.statline, StatlineMode::Custom | StatlineMode::FullCustom) {
        let account = world
            .sessions
            .get(&id)
            .and_then(|s| s.account.as_ref())
            .map(|a| a.id);
        let text = account
            .and_then(|a| world.profiles.get(&a))
            .map(|p| p.statfmt.clone());
        if let (Some(text), Some(s)) = (text, world.sessions.get_mut(&id)) {
            s.ust.statline = text;
        }
    }
    if c.race != SEE_HIDDEN_RACE {
        for slot in &mut c.abils {
            if slot.0 == Ability::SeeHidden {
                *slot = (Ability::None, 0);
            }
        }
    }
    // pack the spell book; after a move the scan resumes past the slot filled.
    let mut free: Option<usize> = None;
    let mut i = 0;
    while i < c.spellbook.len() {
        if c.spellbook[i] == SpellNo(0) {
            free.get_or_insert(i);
        } else if let Some(f) = free {
            c.spellbook[f] = c.spellbook[i];
            c.spellbook[i] = SpellNo(0);
            i = f;
            free = None;
        }
        i += 1;
    }
    if c.lives == 1 {
        notices.push(Msg::EnterLifeEbbing);
    }

    for msg in notices {
        tell_user(world, id, msg);
    }
    if let Some(s) = world.sessions.get_mut(&id) {
        s.character = Some(c);
        s.created = true;
    }
    Ok(())
}
