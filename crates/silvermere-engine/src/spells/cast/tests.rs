//! `display_spell_success` with three stock spells' WCCMSG triples (vine strike, minor
//! healing, fireball) and the built-in texts: the caster's, the target's and the room's
//! lines, plain and alternate-message forms, area and single targets.
//!
//! The records are the stock WCCSPELS and WCCMSG ones: spells 25 vine strike (WCCMSG 14),
//! 13 minor healing (15), 120 fireball (99, Starting Message 100, alternate messages, Full
//! Attack Area).

use silvermere_text::colour::Palette;
use silvermere_text::render::render;
use silvermere_text::strip::strip_ansi;
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{
    Ability, Element, Magery, ResistMode, ResistStat, SpellTarget, SpellType,
};
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{MessageNo, RoomKey, SpellNo};
use silvermere_types::records::{Message, Spell};

use crate::spells::messages::{SpellMsg, coloured_line, spell_success_texts};
use crate::world::{World, WorldSnapshot, blank};

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

fn spell(n: i16, name: &str, kind: SpellType, target: SpellTarget, msgnum: i32) -> Spell {
    Spell {
        number: SpellNo(n),
        name: name.into(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: std::array::from_fn(|_| (Ability::None, 0)),
        energy: 0,
        level: 1,
        minval: 0,
        maxval: 0,
        spell_type: kind,
        resmode: ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target,
        duration: 0,
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

fn world() -> World {
    let mut s = WorldSnapshot::default();
    let g = GameConfig::default();
    for n in [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room] {
        s.rooms.push(blank::clear_room(RoomKey::new(1, n)));
    }
    s.classes
        .push(blank::clear_class(silvermere_types::ids::ClassNo(1)));
    s.races
        .push(blank::clear_race(silvermere_types::ids::RaceNo(1)));
    s.messages = vec![
        msg(
            14,
            "You cast %s at %s for %d damage!",
            "%s casts %s on you for %d damage!",
            "%s casts %s on %s for %s damage!",
        ),
        msg(
            15,
            "You cast %s on %s, healing %d damage!",
            "%s casts %s on you!",
            "%s casts %s on %s!",
        ),
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
    ];
    let mut vine = spell(
        25,
        "vine strike",
        SpellType::Autocombat,
        SpellTarget::MonsterOrSingle,
        14,
    );
    vine.abilities[0] = (Ability::DamageArmour, 0);
    let mut mihe = spell(
        13,
        "minor healing",
        SpellType::General,
        SpellTarget::SelfOrSingle,
        15,
    );
    mihe.abilities[0] = (Ability::Heal, 0);
    mihe.abilities[1] = (Ability::AffectsLiving, 0);
    let mut fbal = spell(
        120,
        "fireball",
        SpellType::Autocombat,
        SpellTarget::FullAttack,
        99,
    );
    fbal.flags = SpellFlags::ALTMSG;
    fbal.abilities[0] = (Ability::DamageArmour, 0);
    fbal.abilities[1] = (Ability::StartMsg, 100);
    let plain = spell(900, "glow", SpellType::General, SpellTarget::OnSelf, 0);
    s.spells = vec![vine, mihe, fbal, plain];
    World::load(GameConfig::default(), 4, s).expect("world loads")
}

fn texts(w: &World, n: i16, target: &str, amount: i32, player: bool, own: bool) -> SpellMsg {
    let spl = w.spells.get(&SpellNo(n)).unwrap();
    spell_success_texts(w, spl, "Thornwyn", target, amount, player, own)
}

/// The three lines rendered as the screen shows them (ANSI off).
fn screen(w: &World, n: i16, m: &SpellMsg) -> String {
    let spl = w.spells.get(&SpellNo(n)).unwrap();
    let mut out = String::new();
    for (who, t) in [
        ("caster", &m.caster),
        ("target", &m.target),
        ("room", &m.room),
    ] {
        let line = match t {
            Some(text) => {
                let msg = if who == "room" {
                    silvermere_text::Msg::SpellLineRoom { text: text.clone() }
                } else {
                    coloured_line(spl, text.clone())
                };
                let bytes = render(&msg, Palette::default()).bytes;
                String::from_utf8_lossy(&strip_ansi(&bytes)).replace("\r\n", "|")
            }
            None => "-".into(),
        };
        out.push_str(&format!("{who}: {line}\n"));
    }
    out
}

#[test]
fn stock_spell_lines() {
    let w = world();
    let mut out = String::new();
    let cases: [(&str, i16, &str, i32, bool, bool); 7] = [
        (
            "vine strike at a monster",
            25,
            "acid slime",
            7,
            false,
            false,
        ),
        ("vine strike at a player", 25, "Bob", 5, true, false),
        ("minor healing on a player", 13, "Bob", 6, true, false),
        ("minor healing on oneself", 13, "Thornwyn", 6, true, true),
        (
            "fireball, alternate texts, area",
            120,
            "orc",
            41,
            false,
            false,
        ),
        (
            "fireball on a player in the area",
            120,
            "Bob",
            41,
            true,
            false,
        ),
        (
            "no WCCMSG record: the built-in texts",
            900,
            "Thornwyn",
            3,
            false,
            false,
        ),
    ];
    for (label, n, target, amount, player, own) in cases {
        let m = texts(&w, n, target, amount, player, own);
        out.push_str(&format!("== {label}\n{}", screen(&w, n, &m)));
    }
    insta::assert_snapshot!("stock_spell_lines", out);
}

// The stock vine strike reads as the pattern registry's examples.
#[test]
fn vine_strike_matches_the_registry_examples() {
    let w = world();
    let m = texts(&w, 25, "acid slime", 7, false, false);
    assert_eq!(
        m.caster.as_deref(),
        Some("You cast vine strike at acid slime for 7 damage!\r")
    );
    assert_eq!(
        m.room.as_deref(),
        Some("Thornwyn casts vine strike on acid slime for 7 damage!\r")
    );
    assert_eq!(m.target, None, "a monster target gets no line");
}

// The area kinds leave the target name out; the starting message opens the caster's and the
// room's lines; SPF_ALTMSG passes (target, amount).
#[test]
fn fireball_area_alternate_texts() {
    let w = world();
    let m = texts(&w, 120, "orc", 41, false, false);
    assert_eq!(
        m.caster.as_deref(),
        Some("You throw a fireball!\rThe fireball explodes, causing 41 damage!\r")
    );
    assert_eq!(
        m.room.as_deref(),
        Some("Thornwyn throws a fireball!\rAn explosion rocks the room for 41 damage!\r")
    );
}

// The caster as his own target gets the colour and nothing else.
#[test]
fn own_target_line_is_empty() {
    let w = world();
    let m = texts(&w, 13, "Thornwyn", 6, true, true);
    assert_eq!(m.target.as_deref(), Some(""));
}

fn in_realm(w: &mut World, name: &str, room: u32) -> silvermere_types::ids::SessionId {
    use silvermere_types::enums::{InGame, SessionState};
    let id = w.ids.alloc().expect("a free session id");
    let mut s = crate::player_session::PlayerSession::new(
        id,
        crate::player_session::Caps::default(),
        std::time::Instant::now(),
    );
    let mut c = crate::player_session::clear_player();
    c.firstname = name.into();
    c.map = silvermere_types::ids::MapId(1);
    c.room = silvermere_types::ids::RoomId(room);
    c.hp = 10;
    c.maxhp = 10;
    s.character = Some(c);
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    w.sessions.insert(id, s);
    id
}

// A caster teleported alone stops leading: his own followers stop and are told, his list is
// cleared, and he moves
#[test]
fn a_single_teleport_ends_the_casters_party() {
    let mut w = world();
    let g = GameConfig::default();
    let (here, there) = (g.start_room, g.reinc_room);
    let caster = in_realm(&mut w, "Aldric", here);
    let follower = in_realm(&mut w, "Bob", here);
    w.session_mut(caster).unwrap().party.group[0] = Some(follower);
    let fs = w.session_mut(follower).unwrap();
    fs.party.leader = Some(caster);
    fs.party.group[0] = Some(caster);
    super::teleport(&mut w, caster, RoomKey::new(1, there), false);
    assert_eq!(w.session(caster).unwrap().player().unwrap().room.0, there);
    assert_eq!(w.session(follower).unwrap().player().unwrap().room.0, here);
    assert_eq!(w.session(follower).unwrap().party.leader, None);
    assert!(w.session(caster).unwrap().party.members().next().is_none());
    assert!(
        w.session(follower)
            .unwrap()
            .party
            .members()
            .next()
            .is_none()
    );
    let out = w
        .session_mut(follower)
        .unwrap()
        .output
        .render(Palette::default(), b"");
    let text = String::from_utf8_lossy(&strip_ansi(&out)).into_owned();
    assert!(
        text.contains("You are no longer following Aldric."),
        "{text:?}"
    );
}
