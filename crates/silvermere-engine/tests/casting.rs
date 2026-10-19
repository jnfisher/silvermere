//! Casting (T-050) through the engine: CAST and a spell typed by its name
//! (`cmd_cast`; `execute_input` stage 9), the four casting engines, the auto-combat recast,
//! the spell rounds of the medium cycle and PURGE.
//!
//! The records carry the stock values: class 13 Druid (magery Druid 3), class 12 Mage
//! (magery Mage 3), race 1 Human; spells 25 vine strike (WCCMSG 14), 27 mend (8250),
//! 26 starlight (8249, Descriptive Message 2092), 14 bless (16, Descriptive Message
//! 8539), 120 fireball (99, Starting Message 100); known monsters 7 kobold thief, 1 giant
//! rat, 5 acid slime (their hit points raised here so a round never kills them).
//!
//! The auto-combat ring is T-034's: a round here is the call its round makes through
//! `RoundHooks::cast` (`spells::autocombat_cast`), with the energy pool and `PS_CANCAST`
//! set as the energy round leaves them.

use crate::common;

use common::{enter, test_config, tiny_snapshot};
use silvermere_engine::SessionId;
use silvermere_engine::combat::AutoTarget;
use silvermere_engine::commands::input::execute_input;
use silvermere_engine::spells::active::tick_player_spells;
use silvermere_engine::spells::{CastOutcome, Engage, autocombat_cast};
use silvermere_engine::world::{World, blank};
use silvermere_rules::SeededRng;
use silvermere_text::strip_ansi;
use silvermere_types::enums::{
    Ability, Alignment, Element, GroupRole, Magery, MessageLevel, MonsterType, ResistMode,
    ResistStat, SpellTarget, SpellType,
};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerSettings, PlayerStatus, PlayerStatus2, SpellFlags,
};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, SpellNo,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins, Message, Spell};

const ROOM: u32 = 2140;
const DRUID: ClassNo = ClassNo(13);
const MAGE: ClassNo = ClassNo(12);
const HUMAN: RaceNo = RaceNo(1);
const VINE: SpellNo = SpellNo(25);
const MEND: SpellNo = SpellNo(27);
const STAR: SpellNo = SpellNo(26);
const BLESS: SpellNo = SpellNo(14);
const FBAL: SpellNo = SpellNo(120);
const KOBOLD: MonsterNo = MonsterNo(7);
const RAT: MonsterNo = MonsterNo(1);
const SLIME: MonsterNo = MonsterNo(5);

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

/// A spell record from the fields that differ from a blank one.
#[allow(clippy::too_many_arguments)]
fn spell(
    n: i16,
    name: &str,
    short: &str,
    kind: SpellType,
    target: SpellTarget,
    magery: Magery,
    level: u16,
    energy: u16,
    mana: i16,
    (minval, maxval): (i16, i16),
    (mininc, minlvls, maxinc, maxlvls): (u8, u8, u8, u8),
    diff: i16,
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
        energy,
        level,
        minval,
        maxval,
        spell_type: kind,
        resmode: ResistMode::from_code(0),
        diff,
        durlvl: 0,
        target,
        duration,
        element: Element::from_code(4),
        resstat: ResistStat::from_code(0),
        magery,
        msgnum: MessageNo(msgnum),
        mana,
        maxinc,
        maxlvls,
        magelvl: 0,
        mininc,
        minlvls,
        durinc: 0,
        durlvls: 0,
        shortnm: short.into(),
    }
}

fn spells() -> Vec<Spell> {
    let mut vine = spell(
        25,
        "vine strike",
        "vine",
        SpellType::Autocombat,
        SpellTarget::MonsterOrSingle,
        Magery::Druid,
        1,
        1000,
        1,
        (4, 9),
        (0, 0, 1, 2),
        15,
        0,
        14,
        &[(Ability::DamageArmour, 0)],
    );
    vine.levelcap = 6;
    let mut mend = spell(
        27,
        "mend",
        "mend",
        SpellType::General,
        SpellTarget::SelfOrSingle,
        Magery::Druid,
        2,
        0,
        4,
        (3, 12),
        (1, 3, 1, 2),
        8,
        0,
        8250,
        &[(Ability::Heal, 0), (Ability::AffectsLiving, 0)],
    );
    mend.levelcap = 30;
    let star = spell(
        26,
        "starlight",
        "star",
        SpellType::General,
        SpellTarget::OnSelf,
        Magery::Druid,
        1,
        0,
        4,
        (175, 175),
        (0, 0, 0, 0),
        0,
        80,
        8249,
        &[(Ability::RoomLight, 0), (Ability::DescMsg, 2092)],
    );
    let bless = spell(
        14,
        "bless",
        "bles",
        SpellType::General,
        SpellTarget::SelfOrSingle,
        Magery::Priest,
        2,
        0,
        4,
        (3, 3),
        (0, 0, 0, 0),
        5,
        40,
        16,
        &[(Ability::AvBless, 0), (Ability::DescMsg, 8539)],
    );
    let mut fbal = spell(
        120,
        "fireball",
        "fbal",
        SpellType::Autocombat,
        SpellTarget::FullAttack,
        Magery::Mage,
        15,
        1000,
        34,
        (33, 45),
        (0, 0, 3, 1),
        -20,
        0,
        99,
        &[(Ability::DamageArmour, 0), (Ability::StartMsg, 100)],
    );
    fbal.flags = SpellFlags::ALTMSG;
    fbal.levelcap = 23;
    fbal.magelvl = 1;
    fbal.element = Element::from_code(1);
    vec![vine, mend, star, bless, fbal]
}

fn messages() -> Vec<Message> {
    vec![
        msg(
            14,
            "You cast %s at %s for %d damage!",
            "%s casts %s on you for %d damage!",
            "%s casts %s on %s for %s damage!",
        ),
        msg(
            16,
            "You cast %s on %s!",
            "%s casts a spell on you!",
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
        msg(
            2092,
            "Your starlight spell fades away.",
            "",
            "You are surrounded by a shimmering light!",
        ),
        msg(8249, "You cast %s!", "%s casts %s!", "%s casts %s!"),
        msg(
            8250,
            "You cast %s on %s, regenerating %d damage!",
            "%s casts %s on you, regenerating some of your wounds!",
            "%s casts %s on %s!",
        ),
        msg(
            8539,
            "The effects of bless wear off!",
            "",
            "You feel lucky!",
        ),
    ]
}

fn known(n: MonsterNo, name: &str, magicres: i16) -> silvermere_types::records::KnownMonster {
    let mut k = blank::clear_known_monster(n);
    k.name = name.into();
    k.level = 1;
    k.exp = 10;
    k.maxhp = 500;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.montype = MonsterType::Lair;
    k.followpct = 20;
    k.magicres = magicres;
    k
}

fn world() -> World {
    let mut s = tiny_snapshot();
    s.messages = messages();
    s.spells = spells();
    s.monsters.push(known(KOBOLD, "kobold thief", 32));
    s.monsters.push(known(RAT, "giant rat", 30));
    s.monsters.push(known(SLIME, "acid slime", 32));
    // two daggers for the ambiguous target name
    for (n, name) in [(1, "long dagger"), (2, "short dagger")] {
        let itm = s.items.iter_mut().find(|i| i.number == ItemNo(n)).unwrap();
        itm.name = name.into();
        itm.weight = 1;
    }
    let mut druid = blank::clear_class(DRUID);
    druid.name = "Druid".into();
    druid.magery = Magery::Druid;
    druid.magelvl = 3;
    druid.combat = 4;
    s.classes.push(druid);
    let mut mage = blank::clear_class(MAGE);
    mage.name = "Mage".into();
    mage.magery = Magery::Mage;
    mage.magelvl = 3;
    s.classes.push(mage);
    let mut human = blank::clear_race(HUMAN);
    human.name = "Human".into();
    s.races.retain(|r| r.number != HUMAN);
    s.races.push(human);
    let mut w = World::load(test_config(), 8, s).expect("world loads");
    w.rng = SeededRng::new(11);
    w
}

/// A druid of `level` in the start room with vine strike, mend and starlight learned.
fn druid(w: &mut World, name: &str, level: u16) -> SessionId {
    let id = enter(w, name, ROOM, MessageLevel::Low);
    let c = w.session_mut(id).unwrap().character.as_mut().unwrap();
    c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
    c.class = DRUID;
    c.race = HUMAN;
    c.level = level;
    (c.hp, c.maxhp) = (30, 40);
    (c.mana, c.maxmana) = (20, 20);
    (c.energy, c.maxenergy) = (1000, 1000);
    c.spellcast = 60;
    c.nspellslots = 100;
    c.spellbook = vec![SpellNo(0); 100];
    c.spellbook[0] = VINE;
    c.spellbook[1] = MEND;
    c.spellbook[2] = STAR;
    id
}

fn spawn(w: &mut World, known: MonsterNo, n: i32) -> ActiveMonsterId {
    let k = w.monsters.get(&known).unwrap().clone();
    let number = ActiveMonsterId(n);
    w.active_monsters.insert(ActiveMonster {
        number,
        knownnum: k.number,
        level: k.level,
        map: MapId(1),
        room: RoomId(ROOM),
        poison: 0,
        energy: k.energy,
        hp: k.maxhp,
        following: String::new(),
        trail: std::array::from_fn(|_| RoomId::default()),
        hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
        hunting: ActiveMonsterId::default(),
        name: k.name.clone(),
        wornitem: ItemNo::default(),
        weapon: k.weapon,
        items: std::array::from_fn(|_| (ItemNo::default(), 0)),
        cash: Coins::default(),
        maxhp: k.maxhp,
        alignment: k.alignment,
        followpct: k.followpct,
        dr: k.dr,
        ac: k.ac,
        exp: k.exp,
        maxenergy: k.energy as i16,
        friendly: 0,
        genroom: RoomId(ROOM),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: k.montype,
        target: -1,
        hpregen: 0,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::Solo,
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: Default::default(),
            value: 0,
            rounds: 0,
        }),
        downrounds: 0,
        created: 0,
    });
    let rs = w.room_state.get_mut(&RoomKey::new(1, ROOM)).unwrap();
    *rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap() = number;
    number
}

fn plr(w: &mut World, id: SessionId) -> &mut silvermere_types::records::Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

fn mon_hp(w: &World, n: ActiveMonsterId) -> i16 {
    w.active_monsters
        .values()
        .find(|m| m.number == n)
        .map_or(0, |m| m.hp)
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
        .split("\r\n")
        .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

fn run(w: &mut World, id: SessionId, line: &str) -> Vec<String> {
    lines(w, id);
    execute_input(w, id, line, false);
    lines(w, id)
}

/// What the energy round leaves a player with (`energy_update_character` and
/// `background_energy`): a full pool and one General cast.
fn energy_round(w: &mut World, id: SessionId) {
    let c = plr(w, id);
    c.energy = c.maxenergy;
    c.settings.insert(PlayerSettings::CANCAST);
}

// `vine kobold` names the spell by its short name;
// an instant attack spell by hand starts the fight without casting, and auto-combat then
// casts it every round (one cast a round with 1000 energy), mana down 1 a cast.
#[test]
fn vine_kobold_engages_and_recasts_each_round() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    let watcher = druid(&mut w, "Bob", 1);
    let kob = spawn(&mut w, KOBOLD, 1);
    // the best chance there is: 98 in 100 (MAXCHANCE)
    plr(&mut w, id).spellcast = 200;
    lines(&mut w, watcher);
    let out = run(&mut w, id, "vine kobold");
    assert_eq!(out, vec!["*Combat Engaged*"]);
    assert_eq!(
        lines(&mut w, watcher),
        vec!["Thornwyn moves to cast vine strike upon kobold thief."]
    );
    assert_eq!(plr(&mut w, id).energy, 0, "the round's energy is spent");
    assert_eq!(plr(&mut w, id).mana, 20, "nothing cast yet");
    // T-034's ring holds the caster with the spell for the next round
    assert!(w.autocombat.queued(id));
    let e = w.autocombat.entry(id);
    assert!(e.casting);
    assert_eq!(e.spell, VINE);
    assert_eq!(e.target, Some(AutoTarget::Monster(kob)));
    let mut casts = 0;
    for round in 0..4 {
        energy_round(&mut w, id);
        let mana = plr(&mut w, id).mana;
        let hp = mon_hp(&w, kob);
        let r = autocombat_cast(&mut w, id, VINE, Some(AutoTarget::Monster(kob)));
        assert_eq!(r.outcome, CastOutcome::Cast, "round {round}");
        assert_eq!(
            r.engaged,
            Some(Engage {
                target: Some(AutoTarget::Monster(kob)),
                spell: VINE
            }),
            "round {round}: auto-combat goes on with the spell"
        );
        let out = lines(&mut w, id);
        assert_eq!(plr(&mut w, id).energy, 0, "round {round}: one cast a round");
        if out == vec!["You attempt to cast vine strike, but fail."] {
            // a failed roll: full energy, half the (1) mana = 0
            assert_eq!(plr(&mut w, id).mana, mana);
            assert_eq!(mon_hp(&w, kob), hp);
        } else {
            casts += 1;
            assert_eq!(plr(&mut w, id).mana, mana - 1, "round {round}: {out:?}");
            let dmg = hp - mon_hp(&w, kob);
            assert!(dmg > 0, "round {round}");
            assert_eq!(
                out,
                vec![format!(
                    "You cast vine strike at kobold thief for {dmg} damage!"
                )]
            );
            assert_eq!(
                lines(&mut w, watcher),
                vec![format!(
                    "Thornwyn casts vine strike on kobold thief for {dmg} damage!"
                )]
            );
        }
    }
    assert!(casts >= 3, "{casts} of 4 rounds cast");
}

// An attack spell at a player out of the level range is refused
// before anything is spent.
#[test]
fn pvp_range_refusal() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    let bob = druid(&mut w, "Bob", 1);
    let out = run(&mut w, id, "cast vine bob");
    assert_eq!(
        out,
        vec!["Such an attack would result in a very unbalanced combat round."]
    );
    assert_eq!(plr(&mut w, id).mana, 20);
    assert_eq!(plr(&mut w, id).energy, 1000);
    assert!(lines(&mut w, bob).is_empty());
}

// Casting ends resting; mend on oneself heals and
// shows the whole roll.
#[test]
fn cast_mend_while_resting_ends_resting() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 2);
    plr(&mut w, id).status.insert(PlayerStatus::RESTING);
    let out = run(&mut w, id, "cast mend");
    let c = plr(&mut w, id);
    assert!(!c.status.contains(PlayerStatus::RESTING), "{out:?}");
    if out == vec!["You attempt to cast mend, but fail."] {
        assert_eq!(c.mana, 18, "half the mana");
    } else {
        assert_eq!(c.mana, 16);
        let healed = i32::from(c.hp) - 30;
        assert!(healed > 0);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(
            out[0].starts_with("You cast mend on Thornwyn, regenerating "),
            "{out:?}"
        );
    }
    // the second General cast in the same round is refused
    let out = run(&mut w, id, "cast mend");
    assert_eq!(out, vec!["You have already cast a spell this round!"]);
}

// Short of mana, nothing is spent.
#[test]
fn out_of_mana_refusal() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 2);
    plr(&mut w, id).mana = 3;
    let out = run(&mut w, id, "cast mend");
    assert_eq!(out, vec!["You do not have enough mana to cast that spell."]);
    assert_eq!(plr(&mut w, id).mana, 3);
    assert_eq!(plr(&mut w, id).hp, 30);
}

// An area attack spell cast by hand out of combat only
// starts the fight; auto-combat's cast then hits every monster in the room, the caster and
// the room told once.
#[test]
fn cast_fbal_engages_then_hits_all_three() {
    let mut w = world();
    let id = druid(&mut w, "Merlin", 15);
    let watcher = druid(&mut w, "Bob", 1);
    {
        let c = plr(&mut w, id);
        c.class = MAGE;
        c.spellbook[3] = FBAL;
        c.mana = 100;
        c.maxmana = 100;
        c.spellcast = 200;
    }
    let mons = [
        spawn(&mut w, KOBOLD, 1),
        spawn(&mut w, RAT, 2),
        spawn(&mut w, SLIME, 3),
    ];
    lines(&mut w, watcher);
    let out = run(&mut w, id, "cast fbal");
    assert_eq!(out, vec!["*Combat Engaged*"]);
    assert_eq!(
        lines(&mut w, watcher),
        vec!["Merlin moves to attack everyone in the room."]
    );
    energy_round(&mut w, id);
    let before: Vec<i16> = mons.iter().map(|&m| mon_hp(&w, m)).collect();
    let r = autocombat_cast(&mut w, id, FBAL, None);
    assert_eq!(r.outcome, CastOutcome::Cast);
    assert_eq!(
        r.engaged,
        Some(Engage {
            target: None,
            spell: FBAL
        })
    );
    for (i, &m) in mons.iter().enumerate() {
        assert!(mon_hp(&w, m) < before[i], "monster {i} was hit");
    }
    let out = lines(&mut w, id);
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0], "You throw a fireball!");
    assert!(
        out[1].starts_with("The fireball explodes, causing "),
        "{out:?}"
    );
    let room = lines(&mut w, watcher);
    assert_eq!(room[0], "Merlin throws a fireball!");
    assert!(
        room[1].starts_with("An explosion rocks the room for "),
        "{room:?}"
    );
    assert_eq!(plr(&mut w, id).mana, 66);
}

#[test]
fn purge_vine() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    let out = run(&mut w, id, "purge vine");
    assert_eq!(out, vec!["Purging spell vine strike from your spellbook."]);
    assert!(!plr(&mut w, id).spellbook.contains(&VINE));
    let out = run(&mut w, id, "vine");
    assert_ne!(out.first().map(String::as_str), Some("*Combat Engaged*"));
}

// A 3-round spell put on at a medium tick
// runs 3 more ticks; the third ends it with the Descriptive Message text.
#[test]
fn a_three_round_buff_expires_with_its_ending() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    {
        let c = plr(&mut w, id);
        c.spells[0] = BLESS;
        c.spellvals[0] = 3;
        c.spellrnds[0] = 3;
    }
    for tick in 1..=2 {
        assert!(!tick_player_spells(&mut w, id));
        assert_eq!(plr(&mut w, id).spellrnds[0], 3 - tick);
        assert_eq!(plr(&mut w, id).spells[0], BLESS, "tick {tick}");
        assert!(lines(&mut w, id).is_empty(), "tick {tick}");
    }
    w.medium_cycle();
    assert_eq!(plr(&mut w, id).spells[0], SpellNo(0));
    assert_eq!(lines(&mut w, id), vec!["The effects of bless wear off!"]);
}

// A lasting spell on oneself goes into a slot with the
// spell's texts; again, it is renewed.
#[test]
fn starlight_goes_into_a_slot() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    plr(&mut w, id).spellcast = 200;
    let out = run(&mut w, id, "cast star");
    assert_eq!(
        out,
        vec![
            "You cast starlight!",
            "You are surrounded by a shimmering light!"
        ]
    );
    let c = plr(&mut w, id);
    assert_eq!(c.spells[0], STAR);
    assert!(c.spellrnds[0] >= 80);
    assert_eq!(c.mana, 16);
}

#[test]
fn unknown_spell_and_wrong_kind() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    assert_eq!(
        run(&mut w, id, "cast fireball"),
        vec!["You do not know how to cast fireball."]
    );
    assert_eq!(
        run(&mut w, id, "invoke vine"),
        vec!["You may not invoke... You are not KAI!  You must cast your spells."]
    );
    assert_eq!(
        run(&mut w, id, "cast"),
        vec!["Syntax: CAST {spell} [{target}]"]
    );
}

// CAST alone in a fight breaks off the round
// (`*Combat Off*`) and repeats the spell as a typed cast: the instant attack spell only
// engages again (`*Combat Engaged*`, energy emptied), it does not cast on the spot.
#[test]
fn cast_alone_restarts_the_spell_as_a_typed_cast() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    let kob = spawn(&mut w, KOBOLD, 1);
    plr(&mut w, id).spellcast = 200;
    assert_eq!(run(&mut w, id, "vine kobold"), vec!["*Combat Engaged*"]);
    // one auto-combat round
    energy_round(&mut w, id);
    let r = autocombat_cast(&mut w, id, VINE, Some(AutoTarget::Monster(kob)));
    assert_eq!(r.outcome, CastOutcome::Cast);
    lines(&mut w, id);
    let mana = plr(&mut w, id).mana;
    let hp = mon_hp(&w, kob);
    energy_round(&mut w, id);
    let out = run(&mut w, id, "cast");
    assert_eq!(out, vec!["*Combat Off*", "*Combat Engaged*"]);
    assert_eq!(plr(&mut w, id).energy, 0, "the energy is emptied");
    assert_eq!(plr(&mut w, id).mana, mana, "nothing cast");
    assert_eq!(mon_hp(&w, kob), hp, "nothing cast");
    assert!(w.autocombat.queued(id));
    assert_eq!(w.autocombat.entry(id).spell, VINE);
}

// The second search (anything) keeps its candidate when
// it is ambiguous; the item goes on to cast_item_target, which refuses the kind.
#[test]
fn an_ambiguous_second_search_keeps_its_candidate() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    {
        let c = plr(&mut w, id);
        c.items[0] = ItemNo(1);
        c.itemuses[0] = -1;
        c.items[1] = ItemNo(2);
        c.itemuses[1] = -1;
    }
    assert_eq!(
        run(&mut w, id, "cast vine dagger"),
        vec!["You may not cast that spell on an item!"]
    );
}

// An area attack spell that would
// hit a good monster is an evil act; with evil warnings on it is refused before anything
// is spent or the fight starts.
#[test]
fn evil_warning_refusal() {
    let mut w = world();
    let id = druid(&mut w, "Merlin", 15);
    {
        let c = plr(&mut w, id);
        c.class = MAGE;
        c.spellbook[3] = FBAL;
        c.mana = 100;
        c.settings.insert(PlayerSettings::EVILWARN);
    }
    let rat = spawn(&mut w, RAT, 1);
    w.active_monsters
        .values_mut()
        .find(|m| m.number == rat)
        .unwrap()
        .alignment = Alignment::Good;
    let out = run(&mut w, id, "cast fbal");
    assert_eq!(
        out,
        vec!["To do this action, you must turn off your evil warnings."]
    );
    let c = plr(&mut w, id);
    assert_eq!((c.energy, c.mana, c.evil), (1000, 100, 0));
    assert!(!w.autocombat.queued(id), "no fight started");
    // with the warnings off the cast goes ahead and costs evil points
    plr(&mut w, id).settings.remove(PlayerSettings::EVILWARN);
    // the charge is announced
    assert_eq!(
        run(&mut w, id, "cast fbal"),
        vec!["A dark cloud passes over you", "*Combat Engaged*"]
    );
    assert!(plr(&mut w, id).evil > 0);
}

// T-036, the T-035 hand-off: a summon is forced past the room's
// spawn rules (`force` 1: this room takes no monsters, `maxmonsters` 0) and the new
// monster follows its caster as a friendly, charmed servant.
#[test]
fn a_summoned_monster_is_forced_and_charmed() {
    let mut w = world();
    let mut summon = spell(
        131,
        "summon rat",
        "summ",
        SpellType::General,
        SpellTarget::OnSelf,
        Magery::Druid,
        1,
        0,
        1,
        (1, 1),
        (0, 0, 0, 0),
        100,
        0,
        0,
        &[(Ability::Summon, 0)],
    );
    summon.levelcap = 1;
    w.spells.insert(SpellNo(131), summon);
    let id = druid(&mut w, "Aldric", 5);
    plr(&mut w, id).spellbook[3] = SpellNo(131);
    let key = RoomKey::new(1, ROOM);
    assert_eq!(w.room(key).unwrap().maxmonsters, 0);
    let said = run(&mut w, id, "summ");
    let n = w.room_state[&key]
        .monsters
        .iter()
        .copied()
        .find(|m| m.0 != 0)
        .unwrap_or_else(|| panic!("not summoned: {said:?}"));
    let m = w.active_monsters.by_number(n).unwrap();
    assert_eq!(m.knownnum, RAT);
    assert_eq!(m.following, "Aldric");
    assert_eq!(m.friendly, 1);
    assert!(m.flags.contains(ActiveMonsterFlags::CHARMED));
}

// T-036, the T-035 hand-off: a monster a spell round makes flee
// runs out through a random usable exit (`move_monster(number, dir, -1, 0)`, the
// direction from `monsters::move_monster::pick_valid_random_direction`).
#[test]
fn a_monster_a_spell_round_makes_flee_runs_out() {
    let mut w = world();
    let terror = spell(
        132,
        "terror",
        "terr",
        SpellType::General,
        SpellTarget::Single,
        Magery::Druid,
        1,
        0,
        1,
        (100, 100),
        (0, 0, 0, 0),
        0,
        3,
        0,
        &[(Ability::Flee, 0)],
    );
    w.spells.insert(SpellNo(132), terror);
    let next = common::special_rooms()[1];
    w.rooms.get_mut(&RoomKey::new(1, ROOM)).unwrap().exits[0] = silvermere_types::records::Exit {
        target: RoomId(next),
        kind: silvermere_types::enums::ExitType::Normal,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    };
    let n = spawn(&mut w, RAT, 1);
    {
        let m = w.active_monsters.by_number_mut(n).unwrap();
        m.spells[0] = ActiveSpell {
            spell: SpellNo(132),
            value: 100,
            rounds: 3,
        };
    }
    silvermere_engine::spells::active::tick_monster_spells(&mut w, n);
    let m = w.active_monsters.by_number(n).unwrap();
    assert_eq!(m.room, RoomId(next), "fled north");
    assert!(w.room_state[&RoomKey::new(1, next)].monsters.contains(&n));
    assert!(!w.room_state[&RoomKey::new(1, ROOM)].monsters.contains(&n));
}

// The Flee moves the monster inside the
// ability walk, before a spell on its last round is cleared, so `move_monster` still sees
// that spell's own Hold Person and the monster stays put.
#[test]
fn a_fleeing_monster_held_by_the_same_spell_stays_on_its_last_round() {
    let mut w = world();
    let snare = spell(
        133,
        "panic snare",
        "snar",
        SpellType::General,
        SpellTarget::Single,
        Magery::Druid,
        1,
        0,
        1,
        (100, 100),
        (0, 0, 0, 0),
        0,
        3,
        0,
        &[(Ability::Flee, 0), (Ability::HoldPerson, 1)],
    );
    w.spells.insert(SpellNo(133), snare);
    let next = common::special_rooms()[1];
    w.rooms.get_mut(&RoomKey::new(1, ROOM)).unwrap().exits[0] = silvermere_types::records::Exit {
        target: RoomId(next),
        kind: silvermere_types::enums::ExitType::Normal,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    };
    let n = spawn(&mut w, RAT, 1);
    w.active_monsters.by_number_mut(n).unwrap().spells[0] = ActiveSpell {
        spell: SpellNo(133),
        value: 100,
        rounds: 1,
    };
    silvermere_engine::spells::active::tick_monster_spells(&mut w, n);
    let m = w.active_monsters.by_number(n).unwrap();
    assert_eq!(m.room, RoomId(ROOM), "held: no move");
    assert_eq!(
        m.spells[0].spell,
        SpellNo(0),
        "the spell ran out after the walk"
    );
}

fn summon_spell(n: i16, short: &str, target: SpellTarget) -> Spell {
    let mut s = spell(
        n,
        "summon rat",
        short,
        SpellType::General,
        target,
        Magery::Druid,
        1,
        0,
        1,
        (1, 1),
        (0, 0, 0, 0),
        100,
        0,
        0,
        &[(Ability::Summon, 0)],
    );
    s.levelcap = 1;
    s
}

/// The monster a summon made: the one in the room that was not there before.
fn summoned(w: &World, before: &[ActiveMonsterId]) -> ActiveMonsterId {
    w.room_state[&RoomKey::new(1, ROOM)]
        .monsters
        .iter()
        .copied()
        .find(|m| m.0 != 0 && !before.contains(m))
        .expect("summoned")
}

// A summon aimed at a player makes a monster that follows that
// player, unfriendly and not charmed, so the monsters' round sets it on him.
#[test]
fn a_summon_on_a_player_follows_and_attacks_him() {
    let mut w = world();
    w.spells
        .insert(SpellNo(134), summon_spell(134, "sumo", SpellTarget::Single));
    // one real attack slot, so a strike spends the round's energy (this file's rat has none)
    let atk = &mut w.monsters.get_mut(&RAT).unwrap().attacks[0];
    atk.kind = silvermere_types::enums::MonsterAttackType::Normal;
    atk.pct = 100;
    atk.accuracy = 10;
    (atk.min, atk.max) = (1, 2);
    atk.energy = 1000;
    let id = druid(&mut w, "Aldric", 5);
    let victim = druid(&mut w, "Bob", 5);
    plr(&mut w, id).spellbook[3] = SpellNo(134);
    let said = run(&mut w, id, "sumo bob");
    let n = w.room_state[&RoomKey::new(1, ROOM)]
        .monsters
        .iter()
        .copied()
        .find(|m| m.0 != 0)
        .unwrap_or_else(|| panic!("not summoned: {said:?}"));
    let m = w.active_monsters.by_number(n).unwrap();
    assert_eq!(m.following, "Bob");
    assert_eq!(m.friendly, 0);
    assert!(!m.flags.contains(ActiveMonsterFlags::CHARMED));
    // the monsters' round: its one real attack goes to Bob (a second visit for Aldric
    // finds it out of energy)
    silvermere_engine::monsters::combat_ai::do_monster_combat(&mut w);
    let attacked = |w: &mut World, i| plr(w, i).status2.contains(PlayerStatus2::ATTACKED);
    assert!(attacked(&mut w, victim));
    assert!(!attacked(&mut w, id));
}

// A summon aimed at a monster makes a monster that hunts it,
// unfriendly; (sic) its number goes into every free hunter slot of the target.
#[test]
fn a_summon_on_a_monster_hunts_it() {
    let mut w = world();
    w.spells.insert(
        SpellNo(135),
        summon_spell(135, "sumk", SpellTarget::Monster),
    );
    let id = druid(&mut w, "Aldric", 5);
    plr(&mut w, id).spellbook[3] = SpellNo(135);
    let kobold = spawn(&mut w, KOBOLD, 7);
    w.active_monsters.by_number_mut(kobold).unwrap().hunters[2] = ActiveMonsterId(99);
    let said = run(&mut w, id, "sumk kobold");
    let n = summoned(&w, &[kobold]);
    let m = w.active_monsters.by_number(n).unwrap();
    assert_eq!(m.hunting, kobold, "{said:?}");
    assert_eq!(m.friendly, 0);
    assert!(m.following.is_empty());
    assert!(!m.flags.contains(ActiveMonsterFlags::CHARMED));
    let hunters = w.active_monsters.by_number(kobold).unwrap().hunters;
    for (j, h) in hunters.iter().enumerate() {
        let want = if j == 2 { ActiveMonsterId(99) } else { n };
        assert_eq!(*h, want, "slot {j}");
    }
}

/// The bytes written to `id` for `line`, prompts left out (rendered empty).
fn raw(w: &mut World, id: SessionId, line: &str) -> Vec<u8> {
    lines(w, id);
    execute_input(w, id, line, false);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, b"");
    s.output.clear();
    b
}

// the ambiguous-match handling clears the output buffer, which drops the
// dispatcher's ANSI_NORMAL, so CAST's refusals start with their own colour
// (m4 run 38823959731, `cast vine` and `cast vine nobody`).
#[test]
fn cast_refusals_carry_no_pending_reset() {
    let mut w = world();
    let id = druid(&mut w, "Thornwyn", 1);
    // an attack spell needs a target
    assert_eq!(
        raw(&mut w, id, "cast vine"),
        b"\x1b[79D\x1b[K\x1b[1;31;40mYou must specify a target for that spell!\r\n"
    );
    // nothing by that name
    assert_eq!(
        raw(&mut w, id, "cast vine nobody"),
        b"\x1b[1;31;40mYou do not see nobody here!\r\n"
    );
}
