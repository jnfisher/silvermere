//! T-051 through the engine: room spells on the medium cycle driven by the engine's clock,
//! the entry spell, a failed pick of a trapped lock, a Spell Trap exit, and an area attack
//! spell cast by hand in a party.
//!
//! The records are the stock ones (`crates/silvermere-import/tests/fixtures/world/*.jsonl.gz`,
//! MANIFEST 2026-10-14): map 7 room 991 Lava Pool (roomspell 526 magma heat: Auto-Combat,
//! Hot, 30-60, `SPF_ALTMSG`, WCCMSG 1553); map 8 rooms 630 Musty Room (no roomspell), 631 and
//! 632 Rancid Underwater Passage (`RF_ENTRYSPELL`, roomspell 453 black water: General, 5-15,
//! `SPF_ALTMSG`, WCCMSG 1370); spell 905 poison darts (the stock Spell Trap exits' spell:
//! Poison, 12-20, 60 rounds, Descriptive Message 8575); spell 120 fireball as T-050's tests
//! have it. The pick-lock trap spell is made up: the stock ones (640, 1040) run text blocks.

use crate::common;

use std::sync::mpsc::channel;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::commands::input::execute_input;
use silvermere_engine::spells::{CastOutcome, autocombat_cast};
use silvermere_engine::world::blank;
use silvermere_engine::{Engine, SeededRng, SessionId, World};
use silvermere_text::strip_ansi;
use silvermere_types::enums::{
    Ability, Element, ExitType, Magery, MessageLevel, ResistMode, ResistStat, SpellTarget,
    SpellType,
};
use silvermere_types::flags::{PlayerSettings, RoomFlags, SpellFlags};
use silvermere_types::ids::{ClassNo, MapId, MessageNo, RoomId, RoomKey, SpellNo};
use silvermere_types::records::{Exit, Message, Spell};

const MAGMA: i32 = 526;
const BLACK_WATER: i32 = 453;
const DARTS: i32 = 905;
const TRAP: i32 = 990;
const FBAL: SpellNo = SpellNo(120);
const MAGE: ClassNo = ClassNo(12);

const LAVA: (u32, u32) = (7, 991);
const MUSTY: (u32, u32) = (8, 630);
const RANCID: (u32, u32) = (8, 631);
const RANCID2: (u32, u32) = (8, 632);
/// Map 1: a room with a locked door north and a trapped lock, and a room with a Spell
/// Trap exit north.
const HUB: u32 = 6000;
const BEYOND: u32 = 6001;
const TRAPPED: u32 = 6002;
const PAST: u32 = 6003;
const PARTY_ROOM: u32 = 6004;

fn key((map, room): (u32, u32)) -> RoomKey {
    RoomKey::new(map, room)
}

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

#[allow(clippy::too_many_arguments)]
fn spell(
    n: i32,
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
        number: SpellNo(n as i16),
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

fn spells() -> Vec<Spell> {
    let mut magma = spell(
        MAGMA,
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
    let mut water = spell(
        BLACK_WATER,
        "black water",
        SpellType::General,
        SpellTarget::SelfOrSingle,
        (5, 15),
        0,
        1370,
        &[
            (Ability::DamageNoArmour, 0),
            (Ability::NonMagical, 0),
            (Ability::DescMsg, 66),
        ],
    );
    water.flags = SpellFlags::ALTMSG;
    water.resist = 1;
    water.diff = 100;
    water.element = Element::from_code(5);
    let mut darts = spell(
        DARTS,
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
    let trap = spell(
        TRAP,
        "needle",
        SpellType::General,
        SpellTarget::Single,
        (8, 8),
        0,
        0,
        &[(Ability::DamageNoArmour, 0)],
    );
    let mut fbal = spell(
        120,
        "fireball",
        SpellType::Autocombat,
        SpellTarget::FullAttack,
        (33, 45),
        0,
        99,
        &[(Ability::DamageArmour, 0), (Ability::StartMsg, 100)],
    );
    fbal.shortnm = "fbal".into();
    fbal.magery = Magery::Mage;
    fbal.level = 15;
    fbal.energy = 1000;
    fbal.mana = 34;
    fbal.maxinc = 3;
    fbal.maxlvls = 1;
    fbal.diff = -20;
    fbal.flags = SpellFlags::ALTMSG;
    fbal.levelcap = 23;
    fbal.magelvl = 1;
    fbal.element = Element::from_code(1);
    vec![magma, water, darts, trap, fbal]
}

fn messages() -> Vec<Message> {
    vec![
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
        msg(
            1370,
            "",
            "You swallow some of the scummy black water, and feel sick!",
            "",
        ),
        msg(
            1553,
            "You are seared by the flames for %d damage!",
            "You are seared by the flames for %d damage!",
            "",
        ),
        msg(
            8575,
            "The effects of the poison wear off!",
            "",
            "You feel ill.",
        ),
    ]
}

fn exit(target: u32, kind: ExitType, par1: u32) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

fn world() -> World {
    let mut s = tiny_snapshot();
    s.spells = spells();
    s.messages = messages();
    let mut lava = blank::clear_room(key(LAVA));
    lava.name = "Lava Pool".into();
    lava.roomspell = MAGMA;
    s.rooms.push(lava);
    let mut musty = blank::clear_room(key(MUSTY));
    musty.name = "Musty Room".into();
    s.rooms.push(musty);
    for k in [RANCID, RANCID2] {
        let mut r = blank::clear_room(key(k));
        r.name = "Rancid Underwater Passage".into();
        r.flags = RoomFlags::ENTRYSPELL;
        r.roomspell = BLACK_WATER;
        s.rooms.push(r);
    }
    let mut hub = blank::clear_room(RoomKey::new(1, HUB));
    hub.name = "Hub".into();
    hub.picktrap = TRAP as i16;
    hub.exits[0] = exit(
        BEYOND,
        ExitType::Door,
        silvermere_engine::movement::exits::LOCK_LOCKED,
    );
    s.rooms.push(hub);
    let mut beyond = blank::clear_room(RoomKey::new(1, BEYOND));
    beyond.exits[1] = exit(
        HUB,
        ExitType::Door,
        silvermere_engine::movement::exits::LOCK_LOCKED,
    );
    s.rooms.push(beyond);
    let mut trapped = blank::clear_room(RoomKey::new(1, TRAPPED));
    trapped.name = "Trapped".into();
    trapped.exits[0] = exit(PAST, ExitType::SpellTrap, DARTS as u32);
    s.rooms.push(trapped);
    for n in [PAST, PARTY_ROOM] {
        s.rooms.push(blank::clear_room(RoomKey::new(1, n)));
    }
    let rat = s
        .monsters
        .iter_mut()
        .find(|m| m.number == silvermere_types::ids::MonsterNo(1))
        .unwrap();
    rat.name = "giant rat".into();
    rat.maxhp = 500;
    rat.energy = 1000;
    rat.alignment = silvermere_types::enums::Alignment::ChaoticEvil;
    let mut mage = blank::clear_class(MAGE);
    mage.name = "Mage".into();
    mage.magery = Magery::Mage;
    mage.magelvl = 3;
    s.classes.push(mage);
    let mut w = World::load(test_config(), 16, s).expect("world loads");
    w.rng = SeededRng::new(17);
    w
}

fn put(world: &mut World, name: &str, (map, room): (u32, u32)) -> SessionId {
    let id = enter(world, name, room, MessageLevel::Low);
    let c = plr(world, id);
    c.map = MapId(map);
    (c.hp, c.maxhp) = (1000, 1000);
    c.level = 10;
    c.movedir = -1;
    id
}

fn plr(world: &mut World, id: SessionId) -> &mut silvermere_types::records::Character {
    world.session_mut(id).unwrap().character.as_mut().unwrap()
}

/// The screen lines written to `id` since the last call: ANSI off, prompts and blank lines
/// dropped.
fn lines(w: &mut World, id: SessionId) -> Vec<String> {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    let text = String::from_utf8_lossy(&b).replace("\x1b[79D\x1b[K", "\r\n");
    String::from_utf8_lossy(&strip_ansi(text.as_bytes()))
        .split(['\r', '\n'])
        .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

mod spells {
    mod room {
        use super::super::*;

        // The room's spell on every other medium pass
        // (every 6 s), never on the passes between (every 3 s).
        #[test]
        fn lava_pool_burns_every_six_seconds() {
            let (tx, _rx) = channel();
            let clock = TestClock::new();
            let mut engine = Engine::new(world(), clock.clone(), SeededRng::new(3), tx);
            let id = put(&mut engine.world, "Alf", LAVA);
            let mut hits = Vec::new();
            let mut hp = plr(&mut engine.world, id).hp;
            for second in 1..=24 {
                clock.advance_ticks(1, &mut engine);
                let now = plr(&mut engine.world, id).hp;
                if now != hp {
                    // (the engine sends each tick's output on, so the lines are T-051's
                    // unit tests' to check)
                    let lost = i32::from(hp - now);
                    assert!((30..=60).contains(&lost), "{lost}");
                    hits.push(second);
                }
                hp = now;
            }
            assert_eq!(hits.len(), 4, "{hits:?}");
            for pair in hits.windows(2) {
                assert_eq!(pair[1] - pair[0], 6, "{hits:?}");
            }
        }

        // Entering an RF_ENTRYSPELL room casts its spell, unless the
        // room left has the same one; the medium pass is the only other cast.
        #[test]
        fn entry_spell_rancid_passage() {
            let mut w = world();
            let id = put(&mut w, "Alf", MUSTY);
            silvermere_engine::movement::move_user::set_user_location(
                &mut w,
                id,
                key(RANCID),
                false,
            );
            let lost = 1000 - plr(&mut w, id).hp;
            assert!((5..=15).contains(&lost), "{lost}");
            assert_eq!(
                lines(&mut w, id),
                vec!["You swallow some of the scummy black water, and feel sick!"]
            );
            assert_eq!(plr(&mut w, id).room, RoomId(RANCID.1));
            // on to the next passage: the same spell, no cast
            let hp = plr(&mut w, id).hp;
            silvermere_engine::movement::move_user::set_user_location(
                &mut w,
                id,
                key(RANCID2),
                false,
            );
            assert_eq!(plr(&mut w, id).hp, hp);
            // a move with no history (a teleport by the sysop, say) casts nothing
            silvermere_engine::movement::move_user::set_user_location(
                &mut w,
                id,
                key(MUSTY),
                false,
            );
            silvermere_engine::movement::move_user::set_user_location(
                &mut w,
                id,
                key(RANCID),
                true,
            );
            assert_eq!(plr(&mut w, id).hp, hp);
        }

        // A failed pick of a trapped lock casts the room's picktrap
        // instead of "Your skill fails you this time."
        #[test]
        fn failed_picklock_springs_the_trap() {
            let mut w = world();
            let id = put(&mut w, "Alf", (1, HUB));
            plr(&mut w, id).picklocks = 0;
            lines(&mut w, id);
            execute_input(&mut w, id, "picklock north", false);
            assert_eq!(plr(&mut w, id).hp, 992);
            assert_eq!(
                lines(&mut w, id),
                vec!["The room cast needle on you for 8 damage."]
            );
        }

        // Crossing a Spell Trap exit casts exitpar1 (stock poison
        // darts: the poison and the lasting spell, the descriptive line only).
        #[test]
        fn spell_trap_exit_casts_exitpar1() {
            let mut w = world();
            let id = put(&mut w, "Alf", (1, TRAPPED));
            lines(&mut w, id);
            assert!(silvermere_engine::movement::move_user(&mut w, id, 0, 0));
            let p = plr(&mut w, id);
            assert_eq!(p.room, RoomId(PAST));
            assert!((12..=20).contains(&p.poison), "{}", p.poison);
            let k = p.spells.iter().position(|&s| s == SpellNo(DARTS as i16));
            let k = k.expect("the darts last");
            assert_eq!(p.spellrnds[k], 60);
            assert!(lines(&mut w, id).contains(&"You feel ill.".to_owned()));
        }
    }

    mod area {
        use super::super::*;

        // `cast fbal` in a party of two with a stranger in
        // pvp range and a monster: the stranger and the monster are hit, the partner never.
        #[test]
        fn cast_fbal_spares_the_partner() {
            let mut w = world();
            let room = (1, PARTY_ROOM);
            let a = put(&mut w, "Merlin", room);
            let b = put(&mut w, "Bea", room);
            let c = put(&mut w, "Cyd", room);
            {
                let p = plr(&mut w, a);
                p.class = MAGE;
                p.level = 15;
                (p.mana, p.maxmana) = (100, 100);
                p.spellcast = 200;
                p.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
                p.nspellslots = 10;
                p.spellbook = vec![SpellNo(0); 10];
                p.spellbook[0] = FBAL;
            }
            plr(&mut w, c).level = 12;
            let rat = silvermere_engine::monsters::generate::generate_monster(
                &mut w,
                RoomKey::new(1, PARTY_ROOM),
                silvermere_engine::monsters::generate::Request {
                    areatype: -1,
                    monnum: silvermere_types::ids::MonsterNo(1),
                    minlevel: 0,
                    maxlevel: 0x7FFF,
                    owner: None,
                    ally: false,
                    force: true,
                },
            )
            .unwrap();
            let rat_hp = |w: &World| {
                w.active_monsters
                    .values()
                    .find(|m| m.number == rat)
                    .map_or(0, |m| m.hp)
            };
            w.session_mut(b).unwrap().party.leader = Some(a);
            silvermere_engine::party::add_to_group(&mut w, a, b, false);
            execute_input(&mut w, a, "cast fbal", false);
            // (the evil the cast earns on the stranger is announced first)
            assert_eq!(lines(&mut w, a).last().unwrap(), "*Combat Engaged*");
            // the energy round: a full pool for the fight's round
            {
                let p = plr(&mut w, a);
                (p.energy, p.maxenergy) = (1000, 1000);
                p.settings.insert(PlayerSettings::CANCAST);
            }
            let r = autocombat_cast(&mut w, a, FBAL, None);
            assert_eq!(r.outcome, CastOutcome::Cast);
            assert_eq!(plr(&mut w, b).hp, 1000, "the partner is spared");
            assert!(plr(&mut w, c).hp < 1000, "the stranger is hit");
            assert!(rat_hp(&w) < 500, "the monster is hit");
            assert!(
                lines(&mut w, c)
                    .iter()
                    .any(|l| l.starts_with("You take ")
                        && l.ends_with(" damage from the explosion!"))
            );
        }
    }
}
