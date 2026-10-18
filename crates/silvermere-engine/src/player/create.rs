//! A new character: [`create_player`], [`roll_stats`] and [`class_title`].
//!
//! T-022 (character creation) calls [`create_player`] when an account without a character
//! enters, then [`roll_stats`] once race and class are known, then the stat editor, then
//! sets `PlayerSession::created` when the character is complete.

use silvermere_types::enums::{Ability, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, MapId, RaceNo, RoomId, SpellNo};
use silvermere_types::records::{Character, Coins};
use silvermere_types::sizes::{NABILITY, NINVITEMS, NKEYS, NSPELLBOOK, NSPELLCAST, NTRAIL, NWORN};

use super::profile;
use super::secondary::{CalcMode, recalculate};
use super::{MAXLIVES, PLAYER_VERSION, RANK_MIDDLE, STARTSTAT};
use crate::player_session::clear_player;
use crate::world::World;

/// A new character's hit points (set at start-up).
pub const NEW_PLAYER_HP: i16 = 20;
/// A new character's energy.
pub const NEW_PLAYER_ENERGY: u16 = 1000;
/// `pagemax`, `gossmax`, `auctmax` of a new character.
pub const NEW_PLAYER_CHAT_RATE: u8 = 3;

/// A new character record.
///
/// `userid` is the account's user name. The record
/// starts from zero, takes every default of a new character, the evil points the account
/// kept from a deleted character and the profile it kept ([`profile::restore`]). Race and
/// class are 0 and the first name is the placeholder (or the profile's); the creation
/// menus set them afterwards (T-022). `today` is the day as the database stores dates
/// (Unix seconds, UTC midnight, [`crate::event_loop::day_start`]).
///
/// The session half of the profile (custom statline text, day byte) is
/// [`profile::restore_session`]; the stats are rolled by [`roll_stats`].
pub fn create_player(world: &World, account: AccountId, userid: &str, today: i64) -> Character {
    // the record starts from zero.
    let mut c = clear_player();
    // the key, and the user-id cut to 10 as a placeholder first name.
    c.userid = userid.to_string();
    c.firstname = userid.chars().take(10).collect();
    c.race = RaceNo(0);
    c.class = ClassNo(0);
    c.level = 1;
    c.exp = 0;
    // every base and current stat at 5.
    c.basint = STARTSTAT;
    c.baswil = STARTSTAT;
    c.basstr = STARTSTAT;
    c.bashea = STARTSTAT;
    c.basagi = STARTSTAT;
    c.bascha = STARTSTAT;
    c.intel = STARTSTAT;
    c.wil = STARTSTAT;
    c.str = STARTSTAT;
    c.hea = STARTSTAT;
    c.agi = STARTSTAT;
    c.cha = STARTSTAT;
    c.maxhp = NEW_PLAYER_HP;
    c.hp = NEW_PLAYER_HP;
    c.weapon = ItemNo(0);
    c.maxwgt = 500;
    c.wgt = 0;
    c.coins = Coins::default();
    // (the two unnamed fields are not carried).
    c.maxenergy = NEW_PLAYER_ENERGY;
    c.energy = NEW_PLAYER_ENERGY;
    c.poison = 0;
    c.maxmr = 0;
    c.mr = 0;
    c.map = MapId(1);
    c.room = RoomId(world.config.start_room);
    // (the two unnamed fields are not carried).
    c.light = 0;
    c.roomlight = 0;
    c.hunger = 1000;
    c.thirst = 1000;
    c.palette = 0;
    c.hpbonus = 0;
    c.manabonus = 0;
    c.ninvslots = NINVITEMS as u8;
    c.exprem = 0;
    c.expbil = 0;
    // empty inventory, uses -1.
    c.items = vec![ItemNo(0); NINVITEMS];
    c.itemuses = vec![-1; NINVITEMS];
    // 50 key slots, uses -2.
    c.nkeyslots = NKEYS as u8;
    c.keys = vec![ItemNo(0); NKEYS];
    c.keyuses = vec![-2; NKEYS];
    c.nspellslots = NSPELLBOOK as u8;
    c.spellbook = vec![SpellNo(0); NSPELLBOOK];
    // (the unnamed fields here are not carried).
    c.lawful = 0;
    c.trailmaps = [MapId(0); NTRAIL];
    c.trailrooms = [RoomId(0); NTRAIL];
    c.bchannel = 0;
    c.verbose = 1;
    c.fasttalk = 1;
    c.statline = StatlineMode::Full;
    c.hidden = 0;
    c.perception = 0;
    c.stealth = 0;
    c.martial = 0;
    c.thievery = 0;
    c.maxmana = 0;
    c.mana = 0;
    c.spellcast = 0;
    c.traps = 0;
    c.disarm = 0;
    c.picklocks = 0;
    c.tracking = 0;
    // nothing worn, uses -1.
    c.worn = [ItemNo(0); NWORN];
    c.wornuses = [-1; NWORN];
    // (no leader and no group are session state: `Party::default()`).
    c.autoengage = 1;
    c.lives = MAXLIVES;
    c.aided = 0;
    c.rank = RANK_MIDDLE;
    c.spells = [SpellNo(0); NSPELLCAST];
    c.spellvals = [0; NSPELLCAST];
    c.spellrnds = [0; NSPELLCAST];
    c.lighttime = 0;
    c.readied = -1;
    c.status = PlayerStatus::empty();
    c.status2 = PlayerStatus2::empty();
    c.movedir = -1;
    c.paging = 1;
    c.immortal = 0;
    c.pagecnt = 0;
    c.gosscnt = 0;
    c.auctcnt = 0;
    c.pagemax = NEW_PLAYER_CHAT_RATE;
    c.gossmax = NEW_PLAYER_CHAT_RATE;
    c.auctmax = NEW_PLAYER_CHAT_RATE;
    c.settings = PlayerSettings::EVILWARN | PlayerSettings::KEEPEXP;
    c.title = "Apprentice".to_string();
    // the evil points the account kept: 0 when none was kept.
    c.evil = world.evil_points.get(&account).map_or(0, |e| e.evil);
    c.version = PLAYER_VERSION;
    c.abils = [(Ability::None, 0); NABILITY];
    c.randomhp = 0;
    c.lifemins = 0;
    c.racechg = 0;
    c.moves = 0;
    c.lastdate = today;
    // the account's kept profile.
    if let Some(rec) = profile::lookup(world, account) {
        profile::restore(&mut c, rec);
    }
    c.flags |= PlayerFlags::RACECHANGED | PlayerFlags::NEWEXP;

    c
}

/// Rolls the stats: they start at the race's minimums, the race's starting character
/// points, the class's hit points per level as `randomhp` below level 2, then the secondary
/// stats in mode 0 and the title. The sex is the caller's: silvermere's account
/// carries none. Nothing happens when the race is
/// unknown.
pub fn roll_stats(world: &World, c: &mut Character, sex: u8) {
    let Some(race) = world.races.get(&c.race) else {
        return;
    };
    c.intel = race.minint as u16;
    c.wil = race.minwil as u16;
    c.str = race.minstr as u16;
    c.hea = race.minhea as u16;
    c.agi = race.minagi as u16;
    c.cha = race.mincha as u16;
    c.basint = c.intel;
    c.baswil = c.wil;
    c.basstr = c.str;
    c.bashea = c.hea;
    c.basagi = c.agi;
    c.bascha = c.cha;
    c.cp = race.basecp;
    c.totalcp = race.basecp as u16;
    if c.level < 2
        && let Some(class) = world.classes.get(&c.class)
    {
        c.randomhp = class.hplevel as u8;
    }
    c.sex = sex;
    recalculate(world, c, CalcMode::Refill);
    c.title = class_title(world, c.class, c.level, c.sex);
}

/// The class title: line `level` of the class's title text block, `male|female`, the
/// female part for `'F'` when there is one; otherwise the built-in titles. Blank lines do
/// not count (the text is split on CR and LF) and trailing blanks are trimmed.
pub fn class_title(world: &World, class: ClassNo, level: u16, sex: u8) -> String {
    let Some(cls) = world.classes.get(&class) else {
        return "Apprentice".to_string();
    };
    if cls.titles.0 != 0
        && let Some(block) = world.text_blocks.get(&(0, cls.titles))
        && level >= 1
        && let Some(line) = block
            .text
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .nth(usize::from(level) - 1)
    {
        let mut parts = line.splitn(2, '|');
        let male = parts.next().unwrap_or("");
        let female = parts.next().map(|f| f.split('|').next().unwrap_or(""));
        if sex == b'F'
            && let Some(f) = female
            && !f.is_empty()
        {
            return f.trim_end().to_string();
        }
        if !male.is_empty() {
            return male.trim_end().to_string();
        }
    }
    match level {
        1 => "Apprentice",
        ..=7 => "Adventurer",
        8..=11 if sex == b'F' => "Lady",
        8..=11 => "Lord",
        _ => "Master",
    }
    .to_string()
}
