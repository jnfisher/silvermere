//! A finished character made without the game: [`make_character`], for
//! `silvermere character create` (a sysop's test character, ready to play).
//!
//! It takes the steps a new character takes in the game, then the ones a sysop's TWEAK
//! takes: [`create_player`] and [`roll_stats`] as at creation, the level with the
//! experience that pays for it (so the entry's level check keeps it), the stats set
//! directly, the hit points rolled at their highest, and each item put in the pack and on
//! through the game's own WEAR and WIELD in a session of its own, so the class, race,
//! level and slot rules hold as they do in play. The character is then whole: the next
//! entry into the realm loads it as any stored one.

use std::time::Instant;

use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_text::colour::Palette;
use silvermere_text::strip::strip_ansi_str;
use silvermere_types::enums::{InGame, ItemType, SessionState, WornLocation};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, RaceNo};
use silvermere_types::records::Character;

use super::create::{class_title, create_player, roll_stats};
use super::secondary::{CalcMode, recalculate};
use crate::items::equip::{ready_weapon, wear_armour};
use crate::items::inventory::add_item_to_inventory;
use crate::player_session::{AccountRef, Caps};
use crate::world::World;

/// The six stats, in the order `stats` takes them.
pub const STAT_NAMES: [&str; 6] = [
    "strength",
    "intellect",
    "willpower",
    "agility",
    "health",
    "charm",
];

/// What to make.
#[derive(Debug, Clone)]
pub struct CharacterSpec {
    /// Letters only, at most 10; capitalised.
    pub firstname: String,
    /// Letters only, at most 18; may be empty.
    pub lastname: String,
    pub race: RaceNo,
    pub class: ClassNo,
    /// `b'M'` or `b'F'`, the account's.
    pub sex: u8,
    /// 1 or more.
    pub level: u16,
    /// Strength, intellect, willpower, agility, health, charm ([`STAT_NAMES`]); `None`
    /// is the race's maximum.
    pub stats: [Option<u16>; 6],
    /// Each put in the pack, then worn or wielded when it can be.
    pub items: Vec<ItemNo>,
    /// Gold crowns in hand.
    pub gold: u32,
}

/// Why a character could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MakeError {
    #[error("the first name must be 1 to 10 letters, got {0:?}")]
    FirstName(String),
    #[error("the last name must be at most 18 letters, got {0:?}")]
    LastName(String),
    #[error("{0} is a game command, so it cannot be a name")]
    NameIsCommand(String),
    #[error("a monster is named {0}")]
    NameIsMonster(String),
    #[error("no race {0}")]
    Race(u16),
    #[error("no class {0}")]
    Class(u16),
    #[error("the level must be 1 to {max}, got {got}")]
    Level { got: u16, max: u16 },
    #[error("{stat} must be {min} to {max} for this race, got {got}")]
    Stat {
        stat: &'static str,
        got: u16,
        min: u16,
        max: u16,
    },
    #[error("no item {0}")]
    Item(i32),
    #[error(
        "{0} is a limited item; give it in the game (SYSOP SUMMON), where its copies are counted"
    )]
    Limited(String),
    #[error("{item} could not be put in the pack: {why}")]
    Pack { item: String, why: String },
    #[error("{gold} gold crowns weigh more than the pack has left: at most {most}")]
    Gold { gold: u32, most: u32 },
    #[error("every session is in use")]
    Full,
}

/// The made character and what happened to each item.
#[derive(Debug, Clone)]
pub struct Made {
    pub character: Character,
    /// One line per item: worn, wielded, or carried and why.
    pub items: Vec<String>,
}

/// The highest level [`make_character`] makes: the experience table's billions half
/// stays exact well past it.
pub const MAX_LEVEL: u16 = 255;

/// Makes the character `spec` describes for `account` (user name `userid`). `today` is
/// the day as the database stores dates. The world is left as it was: the session used
/// for the items is gone again.
pub fn make_character(
    world: &mut World,
    account: AccountId,
    userid: &str,
    spec: &CharacterSpec,
    today: i64,
) -> Result<Made, MakeError> {
    let firstname =
        name(&spec.firstname, 10).ok_or_else(|| MakeError::FirstName(spec.firstname.clone()))?;
    if firstname.is_empty() {
        return Err(MakeError::FirstName(spec.firstname.clone()));
    }
    let lastname =
        name(&spec.lastname, 18).ok_or_else(|| MakeError::LastName(spec.lastname.clone()))?;
    if crate::commands::parse_command(&firstname.to_ascii_uppercase()).is_some() {
        return Err(MakeError::NameIsCommand(firstname));
    }
    if world
        .monsters
        .values()
        .any(|m| m.name.eq_ignore_ascii_case(&firstname))
    {
        return Err(MakeError::NameIsMonster(firstname));
    }
    let race = world
        .races
        .get(&spec.race)
        .cloned()
        .ok_or(MakeError::Race(spec.race.0))?;
    let class = world
        .classes
        .get(&spec.class)
        .cloned()
        .ok_or(MakeError::Class(spec.class.0))?;
    if spec.level == 0 || spec.level > MAX_LEVEL {
        return Err(MakeError::Level {
            got: spec.level,
            max: MAX_LEVEL,
        });
    }
    for &n in &spec.items {
        let itm = world.items.get(&n).ok_or(MakeError::Item(n.0))?;
        if itm.limit > 0 {
            return Err(MakeError::Limited(itm.name.clone()));
        }
    }

    // A new character, as the game makes one.
    let mut c = create_player(world, account, userid, today);
    c.firstname = firstname;
    c.lastname = lastname;
    c.race = spec.race;
    c.class = spec.class;
    roll_stats(world, &mut c, spec.sex);

    // The stats: the race's minimum and maximum bound them, as the stat editor does.
    let mins = [
        race.minstr,
        race.minint,
        race.minwil,
        race.minagi,
        race.minhea,
        race.mincha,
    ];
    let maxs = [
        race.maxstr,
        race.maxint,
        race.maxwil,
        race.maxagi,
        race.maxhea,
        race.maxcha,
    ];
    let mut stats = [0u16; 6];
    for i in 0..6 {
        let (min, max) = (mins[i].max(1) as u16, maxs[i].max(1) as u16);
        let v = spec.stats[i].unwrap_or(max);
        if v < min || v > max {
            return Err(MakeError::Stat {
                stat: STAT_NAMES[i],
                got: v,
                min,
                max,
            });
        }
        stats[i] = v;
    }
    let [st, int, wil, agi, hea, cha] = stats;
    (c.str, c.basstr) = (st, st);
    (c.intel, c.basint) = (int, int);
    (c.wil, c.baswil) = (wil, wil);
    (c.agi, c.basagi) = (agi, agi);
    (c.hea, c.bashea) = (hea, hea);
    (c.cha, c.bascha) = (cha, cha);
    // Every point is spent: the stats were set, not trained.
    c.cp = 0;

    // The level, with exactly the experience that reaches it: the entry takes away a level
    // the experience does not pay for.
    c.level = spec.level;
    let exp = new_calc_exp_needed(spec.level as i16 - 1, class.exp + race.exp);
    c.expbil = exp.expbil;
    c.exprem = exp.exprem;
    // The hit points of every level rolled at their highest (a byte, as stored).
    c.randomhp = (u32::from(class.hplevel.max(0) as u16) * u32::from(spec.level)).min(255) as u8;
    c.title = class_title(world, c.class, c.level, c.sex);
    c.settings.insert(PlayerSettings::VALIDATED);
    recalculate(world, &mut c, CalcMode::Refill);

    // The items, through the game's own pack, WEAR and WIELD.
    let mut lines = Vec::new();
    let mut c = if spec.items.is_empty() {
        c
    } else {
        let id = crate::session::on_connect(world, Caps::default(), Instant::now())
            .map_err(|_| MakeError::Full)?;
        let s = world.session_mut(id).expect("the session just made");
        s.account = Some(AccountRef {
            id: account,
            username: userid.to_string(),
            sysop: false,
            gameop: false,
            sex: spec.sex,
        });
        s.character = Some(c);
        s.created = true;
        s.state = SessionState::Playing;
        s.ingame = InGame::Realm;
        let result = equip_all(world, id, &spec.items, &mut lines);
        let c = world
            .session_mut(id)
            .and_then(|s| s.character.take())
            .expect("the character stays in its session");
        world.sessions.remove(&id);
        world.ids.free(id);
        result?;
        c
    };
    // The gold last: coins weigh (three to a unit), and the pack takes the gear first.
    let left = crate::items::inventory::max_weight(world, &c)
        - i32::from(c.wgt)
        - silvermere_rules::economy::coin_weight(c.coins);
    let most = u32::try_from(left.max(0) * 3).unwrap_or(0);
    if spec.gold > most {
        return Err(MakeError::Gold {
            gold: spec.gold,
            most,
        });
    }
    c.coins.gold = spec.gold;
    recalculate(world, &mut c, CalcMode::Refill);
    Ok(Made {
        character: c,
        items: lines,
    })
}

fn equip_all(
    world: &mut World,
    id: crate::SessionId,
    items: &[ItemNo],
    lines: &mut Vec<String>,
) -> Result<(), MakeError> {
    for &n in items {
        let itm = world.items.get(&n).cloned().ok_or(MakeError::Item(n.0))?;
        if !add_item_to_inventory(world, id, n, -2) {
            return Err(MakeError::Pack {
                item: itm.name,
                why: said(world, id),
            });
        }
        said(world, id);
        let uses = world
            .sessions
            .get(&id)
            .and_then(|s| s.player())
            .and_then(|p| p.items.iter().position(|&i| i == n).map(|k| p.itemuses[k]))
            .unwrap_or(itm.charges);
        let (on, what) = if itm.item_type == ItemType::Weapon {
            (
                ready_weapon(world, id, Some((&itm.name, Some(n)))),
                "wielded",
            )
        } else if itm.worn != WornLocation::Nowhere {
            (wear_armour(world, id, &itm.name, Some((n, uses))), "worn")
        } else {
            lines.push(format!("{}: carried", itm.name));
            continue;
        };
        let why = said(world, id);
        lines.push(if on {
            format!("{}: {what}", itm.name)
        } else {
            format!("{}: carried, not {what} ({why})", itm.name)
        });
    }
    Ok(())
}

/// What the game told the session since the last call, as plain text on one line.
fn said(world: &mut World, id: crate::SessionId) -> String {
    let Some(s) = world.sessions.get(&id) else {
        return String::new();
    };
    let bytes = s.output.render(Palette::default(), b"");
    if let Some(s) = world.session_mut(id) {
        s.output.clear();
    }
    let text = strip_ansi_str(&String::from_utf8_lossy(&bytes));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `raw` trimmed, letters only, at most `max`, first letter upper case; `None` when it
/// is not.
fn name(raw: &str, max: usize) -> Option<String> {
    let t = raw.trim();
    if t.chars().count() > max || !t.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = t.to_string();
    if let Some(first) = out.get(..1) {
        let up = first.to_ascii_uppercase();
        out.replace_range(..1, &up);
    }
    Some(out)
}
