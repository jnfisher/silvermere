//! A small world for the T-051 unit tests (`spells::area`, `spells::room`,
//! `monsters::area_cast`): a few rooms, a class and a race, and stock spell, message and
//! monster records transcribed from the vendored world data
//! (`crates/silvermere-import/tests/fixtures/world/*.jsonl.gz`, MANIFEST 2026-10-14):
//!
//! - spell 120 fireball (WCCMSG 99, Starting Message 100, `SPF_ALTMSG`, Full Attack Area,
//!   Inflict Damage on Armour), as in T-050's tests;
//! - spell 526 magma heat (WCCMSG 1553, `SPF_ALTMSG`, Auto-Combat, Self or Single, 30-60, Hot,
//!   Inflict Damage Ignoring Armour, Descriptive Message 66): the roomspell of the Lava
//!   Pool rooms (map 7, rooms 991-...);
//! - spell 511 freezing water (WCCMSG 1509, `SPF_ALTMSG`, General, Monster or Single, 1-4,
//!   Inflict Damage Ignoring Armour, Non Magical, Cast on ending 512, Descriptive Message 66);
//! - spell 905 poison darts (WCCMSG 66, General, Monster or Single, 12-20, 60 rounds,
//!   Poison, Non Magical, Descriptive Message 8575): the stock Spell Trap exits' spell;
//! - spell 192 quake (WCCMSG 193, Starting Message 192, `SPF_ALTMSG`, Auto-Combat, Full
//!   Attack Area, 10-20, Stone, Inflict Damage on Armour);
//! - spell 7 smite (WCCMSG 7, General, Self or Single, 1, 60 rounds, Alter damage, Descriptive
//!   Message 8532): the spell of the Cast exit of The Silver River (map 1, room 288);
//! - known monster 208 nomadic shaman (attack slot 1: quake, 100%, level 10, 1000 energy).

use std::time::Instant;

use silvermere_text::strip_ansi;
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{
    Ability, Alignment, Element, InGame, Magery, MessageLevel, MonsterType, ResistMode, ResistStat,
    SessionState, SpellTarget, SpellType, StatlineMode,
};
use silvermere_types::flags::{PlayerSettings, SpellFlags};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, SessionId,
    SpellNo,
};
use silvermere_types::records::{Character, Message, MonsterAttack, Spell};

use crate::player_session::{Caps, PlayerSession, clear_player};
use crate::world::{World, WorldSnapshot, blank};

pub const ROOM: u32 = 2140;
pub const OTHER: u32 = 2141;
pub const MAGMA: SpellNo = SpellNo(526);
pub const FREEZE: SpellNo = SpellNo(511);
pub const QUAKE: SpellNo = SpellNo(192);
pub const SMITE: SpellNo = SpellNo(7);
pub const SHAMAN: MonsterNo = MonsterNo(208);
pub const RAT: MonsterNo = MonsterNo(1);

pub fn key(room: u32) -> RoomKey {
    RoomKey::new(1, room)
}

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

/// A spell record: the stock fields given, the rest blank.
#[allow(clippy::too_many_arguments)]
pub fn spell(
    n: i16,
    name: &str,
    kind: SpellType,
    target: SpellTarget,
    (minval, maxval): (i16, i16),
    duration: i16,
    msgnum: i32,
    abilities: &[(Ability, i16)],
) -> Spell {
    let mut a: [(Ability, i16); 10] = std::array::from_fn(|_| (Ability::None, 0));
    for (i, x) in abilities.iter().enumerate() {
        a[i] = *x;
    }
    Spell {
        number: SpellNo(n),
        name: name.into(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: a,
        energy: 0,
        level: 0,
        minval,
        maxval,
        spell_type: kind,
        resmode: ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target,
        duration,
        element: Element::from_code(4),
        resstat: ResistStat::from_code(0),
        magery: Magery::None,
        msgnum: MessageNo(msgnum),
        mana: 0,
        maxinc: 0,
        maxlvls: 0,
        magelvl: 0,
        mininc: 0,
        minlvls: 0,
        durinc: 0,
        durlvls: 0,
        shortnm: String::new(),
    }
}

pub fn stock_spells() -> Vec<Spell> {
    let mut fbal = spell(
        120,
        "fireball",
        SpellType::Autocombat,
        SpellTarget::FullAttack,
        (10, 20),
        0,
        99,
        &[(Ability::DamageArmour, 0), (Ability::StartMsg, 100)],
    );
    fbal.flags = SpellFlags::ALTMSG;
    fbal.shortnm = "fbal".into();
    let mut magma = spell(
        526,
        "magma heat",
        SpellType::Autocombat,
        SpellTarget::SelfOrSingle,
        (30, 60),
        0,
        1553,
        &[(Ability::DamageNoArmour, 0), (Ability::DescMsg, 66)],
    );
    magma.flags = SpellFlags::ALTMSG;
    magma.diff = 200;
    magma.element = Element::from_code(1);
    magma.shortnm = "magm".into();
    let mut freeze = spell(
        511,
        "freezing water",
        SpellType::General,
        SpellTarget::MonsterOrSingle,
        (1, 4),
        0,
        1509,
        &[
            (Ability::DamageNoArmour, 0),
            (Ability::NonMagical, 0),
            (Ability::EndCast, 512),
            (Ability::DescMsg, 66),
        ],
    );
    freeze.flags = SpellFlags::ALTMSG;
    freeze.shortnm = "free".into();
    let mut darts = spell(
        905,
        "poison darts",
        SpellType::General,
        SpellTarget::MonsterOrSingle,
        (12, 20),
        60,
        66,
        &[
            (Ability::Poison, 0),
            (Ability::NonMagical, 0),
            (Ability::DescMsg, 8575),
        ],
    );
    darts.resist = 1;
    darts.diff = 100;
    let mut quake = spell(
        192,
        "quake",
        SpellType::Autocombat,
        SpellTarget::FullAttack,
        (10, 20),
        0,
        193,
        &[(Ability::DamageArmour, 0), (Ability::StartMsg, 192)],
    );
    quake.flags = SpellFlags::ALTMSG;
    quake.resist = 1;
    quake.energy = 1000;
    quake.diff = 100;
    quake.element = Element::from_code(2);
    quake.shortnm = "quak".into();
    let mut smite = spell(
        7,
        "smite",
        SpellType::General,
        SpellTarget::SelfOrSingle,
        (1, 1),
        60,
        7,
        &[(Ability::Damage, 0), (Ability::DescMsg, 8532)],
    );
    smite.resist = 1;
    smite.resmode = ResistMode::from_code(1);
    smite.level = 3;
    smite.diff = 5;
    smite.shortnm = "smit".into();
    vec![fbal, magma, freeze, darts, quake, smite]
}

pub fn stock_messages() -> Vec<Message> {
    vec![
        msg(
            7,
            "You cast %s on %s!",
            "%s casts %s upon you!",
            "%s casts %s on %s!",
        ),
        msg(66, "", "", ""),
        msg(
            99,
            "The fireball explodes, %scausing %d damage!",
            "You take %d damage from the explosion!",
            "An explosion rocks %sthe room for %s damage!",
        ),
        msg(
            100,
            "You throw a fireball!",
            "%s throws a fireball!",
            "%s throws a fireball!",
        ),
        msg(192, "", "%s raises his staff in the air!", ""),
        msg(193, "", "A quake rocks the room for %d damage!", ""),
        msg(1509, "", "You are freezing cold!", ""),
        msg(
            1553,
            "You are seared by the flames for %d damage!",
            "You are seared by the flames for %d damage!",
            "",
        ),
        msg(
            8532,
            "The effects of smite wear off!",
            "",
            "You feel powerful!",
        ),
        msg(
            8575,
            "The effects of the poison wear off!",
            "",
            "You feel ill.",
        ),
    ]
}

/// The nomadic shaman (208): its first two attack slots (a weapon, then quake).
pub fn shaman() -> silvermere_types::records::KnownMonster {
    let mut k = blank::clear_known_monster(SHAMAN);
    k.name = "nomadic shaman".into();
    k.montype = MonsterType::from_code(16);
    k.level = 1;
    k.genindex = 6;
    k.dr = 2;
    k.ac = 45;
    k.followpct = 60;
    k.magicres = 50;
    k.exp = 365;
    k.maxhp = 110;
    k.energy = 1000;
    k.hpregen = 12;
    k.alignment = Alignment::from_code(1);
    k.charmlvl = 20;
    k.charmres = 66;
    k.attacks[0] = MonsterAttack {
        kind: k.attacks[0].kind,
        accuracy: 60,
        pct: 70,
        min: 6,
        max: 16,
        energy: 500,
        ..k.attacks[0]
    };
    k.attacks[1] = MonsterAttack {
        accuracy: 192,
        pct: 85,
        min: 100,
        max: 10,
        energy: 1000,
        ..k.attacks[1]
    };
    k
}

/// A rat for the area tests: chaotic evil, plain.
pub fn rat() -> silvermere_types::records::KnownMonster {
    let mut k = blank::clear_known_monster(RAT);
    k.name = "giant rat".into();
    k.level = 1;
    k.exp = 10;
    k.maxhp = 500;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.montype = MonsterType::Lair;
    k
}

/// The world: the special rooms, two test rooms (east/west of each other), the stock
/// records above.
pub fn world() -> World {
    let mut s = WorldSnapshot::default();
    let g = GameConfig::default();
    for n in [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room] {
        s.rooms.push(blank::clear_room(key(n)));
    }
    for n in [ROOM, OTHER] {
        let mut r = blank::clear_room(key(n));
        r.name = format!("Room {n}");
        r.maxmonsters = 10;
        s.rooms.push(r);
    }
    s.items
        .push(blank::clear_item(silvermere_types::ids::ItemNo(1)));
    s.classes.push(blank::clear_class(ClassNo(1)));
    s.races.push(blank::clear_race(RaceNo(1)));
    s.spells = stock_spells();
    s.messages = stock_messages();
    s.monsters = vec![rat(), shaman()];
    let mut w = World::load(g, 16, s).expect("world loads");
    w.rng = silvermere_rules::SeededRng::new(5);
    w
}

/// A playing character named `name` in `room` on a fresh session.
pub fn enter(world: &mut World, name: &str, room: u32) -> SessionId {
    let id = world.ids.alloc().expect("a free session id");
    let mut s = PlayerSession::new(id, Caps::default(), Instant::now());
    let mut c = clear_player();
    c.firstname = name.to_string();
    c.userid = name.to_ascii_uppercase();
    c.map = MapId(1);
    c.room = RoomId(room);
    c.msglevel = MessageLevel::Low;
    (c.hp, c.maxhp) = (100, 100);
    (c.energy, c.maxenergy) = (1000, 1000);
    c.level = 10;
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.sex = b'M';
    c.statline = StatlineMode::On;
    c.settings = PlayerSettings::VALIDATED;
    s.character = Some(c);
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    world.sessions.insert(id, s);
    id
}

pub fn plr(world: &mut World, id: SessionId) -> &mut Character {
    world.session_mut(id).unwrap().character.as_mut().unwrap()
}

/// A forced known monster `n` in `room`.
pub fn spawn(world: &mut World, n: MonsterNo, room: u32) -> ActiveMonsterId {
    crate::monsters::generate::generate_monster(
        world,
        key(room),
        crate::monsters::generate::Request {
            areatype: -1,
            monnum: n,
            minlevel: 0,
            maxlevel: 0x7FFF,
            owner: None,
            ally: false,
            force: true,
        },
    )
    .expect("the monster is made")
}

/// The screen lines written to `id` since the last call: ANSI off, prompts and blank lines
/// dropped.
pub fn lines(world: &mut World, id: SessionId) -> Vec<String> {
    let prompt = crate::tell::prompt_bytes(world, id);
    let palette = crate::tell::palette_of(world, id);
    let s = world.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    let text = String::from_utf8_lossy(&b).replace("\x1b[79D\x1b[K", "\r\n");
    String::from_utf8_lossy(&strip_ansi(text.as_bytes()))
        .split(['\r', '\n'])
        .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}
