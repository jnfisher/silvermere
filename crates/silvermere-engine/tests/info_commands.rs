//! The info commands (T-027): STATUS, HEALTH, EXPERIENCE, INVENTORY, KEYS, WEALTH, WHO,
//! TOPTEN and the top gangs, SPELLS and POWERS, HELP, LOOK {direction} and LOOK {name},
//! and the main menu's T, G, S/W, I, C, A and R. The screens are rendered to bytes and
//! compared with the vendored capture of the original (`tests/captures/status-screens`,
//! character Aldric Stonebrook, a level 1 Human Warrior in the Newhaven starter set), the
//! pattern registry (`tests/fixtures/patterns.json`) and `insta` snapshots; the paged
//! TOPTEN list and the look notices run through the engine with two sessions.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::commands::info::{
    cmd_experience, display_scan_of_users, display_users_spells, look_at, look_direction,
    show_health, show_status,
};
use silvermere_engine::commands::inventory::{
    display_inventory_items, display_user_money, display_users_keys,
};
use silvermere_engine::commands::{Cmd, CommandLine, parse_command};
use silvermere_engine::display::topten::{display_only_topten, display_top_gangs};
use silvermere_engine::help::help_text;
use silvermere_engine::items::use_item::{UseTarget, use_item};
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, SeededRng, SessionId, World, WorldSnapshot,
};
use silvermere_types::enums::{
    Ability, Alignment, DebugLevel, ExitType, Gender, GroupRole, ItemType, Magery, MessageLevel,
    MonsterType, SessionState, WeaponType, WornLocation,
};
use silvermere_types::flags::{
    ActiveMonsterFlags, GangFlags, PlayerFlags, PlayerSettings, SessionFlags, SpellFlags,
};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, SpellNo,
};
use silvermere_types::records::{
    ActiveMonster, ActiveSpell, Character, Coins, Exit, Gang, Message, Room, Spell,
};

const CAPTURE: &[u8] = include_bytes!("../../../tests/captures/status-screens/raw.bin");

// ---------------------------------------------------------------------------------------
// The world.

const ENTRANCE: u32 = 2140;
const NORTH: u32 = 2141;
const ARENA: u32 = 2150;

const WARRIOR: u16 = 1;
const MAGE: u16 = 2;
const MYSTIC: u16 = 3;

const VEST: i32 = 101;
const PANTS: i32 = 102;
const HELM: i32 = 103;
const GLOVES: i32 = 104;
const BOOTS: i32 = 105;
const STAFF: i32 = 106;
const TORCH: i32 = 107;
const DAGGER: i32 = 108;
const BRASS_KEY: i32 = 109;
const IRON_KEY: i32 = 110;

fn exit(target: u32, kind: ExitType, par1: u32, par2: i16) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1,
        par2,
        par3: 0,
        par4: 0,
    }
}

fn room(n: u32, name: &str) -> Room {
    let mut r = blank::clear_room(RoomKey::new(1, n));
    r.name = name.into();
    r
}

fn item(
    n: i32,
    name: &str,
    kind: ItemType,
    worn: WornLocation,
    ac: i16,
) -> silvermere_types::records::Item {
    let mut i = blank::clear_item(ItemNo(n));
    i.name = name.into();
    i.item_type = kind;
    i.worn = worn;
    i.ac = ac;
    i.gettable = 1;
    i
}

fn spell(n: i16, name: &str, short: &str, level: u16, mana: i16) -> Spell {
    Spell {
        number: SpellNo(n),
        name: name.into(),
        descs: ["A bolt of force.".into(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: std::array::from_fn(|_| (Ability::None, 0)),
        energy: 0,
        level,
        minval: 0,
        maxval: 0,
        spell_type: silvermere_types::enums::SpellType::from_code(0),
        resmode: silvermere_types::enums::ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target: silvermere_types::enums::SpellTarget::from_code(0),
        duration: 0,
        element: silvermere_types::enums::Element::from_code(0),
        resstat: silvermere_types::enums::ResistStat::from_code(0),
        magery: Magery::Mage,
        msgnum: MessageNo(0),
        mana,
        maxinc: 0,
        maxlvls: 0,
        magelvl: 0,
        mininc: 0,
        minlvls: 0,
        durinc: 0,
        durlvls: 0,
        shortnm: short.into(),
    }
}

fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    s.rooms.retain(|r| r.key.room.0 != ENTRANCE);
    let mut entrance = room(ENTRANCE, "Newhaven, Village Entrance");
    entrance.exits[0] = exit(NORTH, ExitType::Normal, 0, 0);
    s.rooms.push(entrance);
    let mut north = room(NORTH, "Newhaven, Market Square");
    north.exits[1] = exit(ENTRANCE, ExitType::Normal, 0, 0);
    s.rooms.push(north);
    let mut arena = room(ARENA, "Newhaven, Arena");
    arena.exits[0] = exit(2152, ExitType::Door, 2, 30);
    arena.exits[8] = exit(2146, ExitType::Normal, 0, 0);
    // a shut Door exit west and a locked Key exit east, each with a room behind it
    arena.exits[3] = exit(2152, ExitType::Door, 1, 30);
    arena.exits[2] = exit(2153, ExitType::Key, BRASS_KEY as u32, 2);
    s.rooms.push(arena);
    s.rooms.push(room(2152, "Newhaven, Armoury"));
    s.rooms.push(room(2153, "Newhaven, Vault"));
    s.classes.clear();
    for (n, name, magery) in [
        (WARRIOR, "Warrior", Magery::None),
        (MAGE, "Mage", Magery::Mage),
        (MYSTIC, "Mystic", Magery::Kai),
    ] {
        let mut c = blank::clear_class(ClassNo(n));
        c.name = name.into();
        c.magery = magery;
        s.classes.push(c);
    }
    s.races.clear();
    let mut human = blank::clear_race(RaceNo(1));
    human.name = "Human".into();
    s.races.push(human);
    let w = WornLocation::Nowhere;
    s.items.extend([
        item(
            VEST,
            "padded vest",
            ItemType::Armour,
            WornLocation::Torso,
            40,
        ),
        item(
            PANTS,
            "padded pants",
            ItemType::Armour,
            WornLocation::Legs,
            20,
        ),
        item(
            HELM,
            "padded helm",
            ItemType::Armour,
            WornLocation::Head,
            20,
        ),
        item(
            GLOVES,
            "padded gloves",
            ItemType::Armour,
            WornLocation::Hands,
            15,
        ),
        item(
            BOOTS,
            "padded boots",
            ItemType::Armour,
            WornLocation::Feet,
            15,
        ),
        {
            let mut i = item(STAFF, "quarterstaff", ItemType::Weapon, w, 0);
            i.wpntype = WeaponType::TwoHandBlunt;
            i.desc[0] = "A stout wooden staff,  ".into();
            i.desc[1] = "shod with iron.".into();
            i
        },
        item(TORCH, "torch", ItemType::Light, w, 0),
        {
            let mut i = item(DAGGER, "dagger", ItemType::Weapon, w, 0);
            i.wpntype = WeaponType::OneHandSharp;
            i
        },
        item(BRASS_KEY, "brass key", ItemType::Key, w, 0),
        item(IRON_KEY, "iron key", ItemType::Key, w, 0),
    ]);
    s.spells.extend([
        spell(1, "magic missile", "mmis", 1, 2),
        spell(2, "light", "ligh", 1, 1),
        spell(3, "fireball", "fbal", 5, 10),
        spell(4, "shield", "", 3, 4),
    ]);
    let mut rat = blank::clear_known_monster(MonsterNo(3));
    rat.name = "giant rat".into();
    rat.maxhp = 10;
    rat.gender = Gender::It;
    rat.desc[0] = "A huge, filthy rat with   ".into();
    rat.desc[1] = "yellowed teeth.".into();
    s.monsters.push(rat);
    s.messages.push(Message {
        number: MessageNo(500),
        msgs: [
            String::new(),
            "%s is surrounded by a shimmering light.".into(),
            "You are surrounded by a shimmering light.".into(),
        ],
    });
    s
}

fn world() -> World {
    World::load(test_config(), 16, snapshot()).expect("world loads")
}

/// Aldric Stonebrook as the capture shows him: a level 1 Human Warrior in the Newhaven
/// starter set, the staff wielded, nothing else.
fn aldric(world: &mut World) -> SessionId {
    let id = enter(world, "Aldric", ENTRANCE, MessageLevel::High);
    edit(world, id, |c| {
        c.lastname = "Stonebrook".into();
        c.title = "Apprentice".into();
        c.level = 1;
        c.lives = 9;
        c.cp = 80;
        c.perception = 40;
        c.hp = 35;
        c.maxhp = 35;
        c.martial = 13;
        c.mr = 40;
        c.str = 50;
        c.agi = 40;
        c.intel = 40;
        c.hea = 50;
        c.wil = 40;
        c.cha = 40;
        c.basstr = 50;
        c.basagi = 40;
        c.basint = 40;
        c.bashea = 50;
        c.baswil = 40;
        c.bascha = 40;
        c.wgt = 500;
        // the secondary stats: Strength 50 times 48
        c.maxwgt = 2400;
        c.class = ClassNo(WARRIOR);
        c.items = vec![ItemNo(0); 100];
        c.itemuses = vec![-1; 100];
        for (i, n) in [VEST, PANTS, HELM, GLOVES, BOOTS, STAFF]
            .into_iter()
            .enumerate()
        {
            c.items[i] = ItemNo(n);
        }
        c.worn = std::array::from_fn(|_| ItemNo(0));
        for (i, n) in [VEST, PANTS, HELM, GLOVES, BOOTS].into_iter().enumerate() {
            c.worn[i] = ItemNo(n);
        }
        c.weapon = ItemNo(STAFF);
        c.readied = -1;
        c.keys = vec![ItemNo(0); 50];
        c.nspellslots = 20;
        c.spellbook = vec![SpellNo(0); 20];
    });
    id
}

fn edit(world: &mut World, id: SessionId, f: impl FnOnce(&mut Character)) {
    f(world.session_mut(id).unwrap().character.as_mut().unwrap());
}

fn monster(world: &mut World, n: i32, room: u32, hp: i16) {
    let m = ActiveMonster {
        number: ActiveMonsterId(n),
        knownnum: MonsterNo(3),
        level: 1,
        map: MapId(1),
        room: RoomId(room),
        poison: 1,
        energy: 0,
        hp,
        following: String::new(),
        trail: std::array::from_fn(|_| RoomId::default()),
        hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
        hunting: ActiveMonsterId::default(),
        name: "giant rat".into(),
        wornitem: ItemNo::default(),
        weapon: ItemNo::default(),
        items: std::array::from_fn(|_| (ItemNo::default(), 0)),
        cash: Coins::default(),
        maxhp: 10,
        alignment: Alignment::Neutral,
        followpct: 0,
        dr: 0,
        ac: 0,
        exp: 0,
        maxenergy: 0,
        friendly: 0,
        genroom: RoomId(room),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: MonsterType::Lair,
        target: 0,
        hpregen: 0,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::Solo,
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: SpellNo::default(),
            value: 0,
            rounds: 0,
        }),
        downrounds: 0,
        created: 0,
    };
    world.active_monsters.insert(m);
    let state = world.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
    let slot = state.monsters.iter().position(|m| m.0 == 0).unwrap();
    state.monsters[slot] = ActiveMonsterId(n);
}

// ---------------------------------------------------------------------------------------
// Bytes, captures and patterns.

/// The bytes the session would receive now (prompts rendered empty), and the batch
/// emptied.
fn take(world: &mut World, id: SessionId) -> Vec<u8> {
    let palette = palette_of(world, id);
    let s = world.session_mut(id).unwrap();
    let out = s.output.render(palette, b"");
    s.output.clear();
    out
}

fn esc(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> usize {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
        .unwrap_or_else(|| panic!("{} not in the capture", esc(needle)))
}

/// The capture after the echo of `after`, from the first `start` (included) to the first
/// `end` after it (included).
fn slice<'a>(after: &[u8], start: &[u8], end: &[u8]) -> &'a [u8] {
    let from = find(CAPTURE, after, 0);
    let a = find(CAPTURE, start, from);
    let b = find(CAPTURE, end, a) + end.len();
    &CAPTURE[a..b]
}

/// Every `X\b` pair dropped: the anti-script letter of `dirnames[]`.
fn drop_backspace_pairs(bytes: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == 8 && out.pop().is_some() {
            continue;
        }
        out.push(b);
    }
    out
}

/// The screen lines: ANSI and `X\b` pairs removed, split at `\r\n`, trailing and
/// leading blanks off, empty lines dropped.
fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(&drop_backspace_pairs(bytes));
    String::from_utf8_lossy(&plain)
        .split("\r\n")
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

fn pattern(name: &str) -> regex::Regex {
    let reg: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    let rx = reg["static"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no pattern {name}"))["rx"]
        .as_str()
        .unwrap()
        .to_owned();
    regex::Regex::new(&format!("^(?:{rx})$")).unwrap()
}

#[track_caller]
fn assert_line_matches(bytes: &[u8], name: &str) {
    let rx = pattern(name);
    let ls = lines(bytes);
    assert!(ls.iter().any(|l| rx.is_match(l)), "{name}: {ls:?}");
}

fn line(text: &str) -> CommandLine {
    CommandLine::parse(text)
}

// ---------------------------------------------------------------------------------------
// Against the capture.

// show_status
#[test]
fn st_equals_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    show_status(&mut w, id);
    let got = take(&mut w, id);
    let want = slice(
        b":st\r\n",
        b"\x1b[79D\x1b[K\x1b[0m\x1b[32mName:",
        b"\x1b[0;37;40m",
    );
    assert_eq!(esc(&got), esc(want));
    for p in [
        "st_name",
        "st_race",
        "st_class",
        "st_hits",
        "st_traps",
        "st_picklocks",
        "st_str",
        "st_int",
        "st_wil",
    ] {
        assert_line_matches(&got, p);
    }
    insta::assert_snapshot!("st_aldric", esc(&got));
}

#[test]
fn exp_equals_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    cmd_experience(&mut w, id, Cmd::Experience, &line("exp"));
    let got = take(&mut w, id);
    let want = slice(b":exp\r\n", b"\x1b[0;32mExp: ", b"[0%]\r\n");
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "exp");
    insta::assert_snapshot!("exp_aldric", esc(&got));
}

// the carried line wraps at 79 and, with no coins, starts with the equipped first item
#[test]
fn i_equals_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    display_inventory_items(&mut w, id);
    let got = take(&mut w, id);
    let want = slice(
        b":i\r\n",
        b"\x1b[0;37;40m\x1b[79D\x1b[KYou are carrying",
        b"[20%]\x1b[0m\r\n",
    );
    assert_eq!(esc(&got), esc(want));
    for p in [
        "inv_carrying",
        "inv_no_keys",
        "inv_wealth",
        "inv_encumbrance",
    ] {
        assert_line_matches(&got, p);
    }
    insta::assert_snapshot!("i_aldric", esc(&got));
}

#[test]
fn health_wealth_keys_spells_equal_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    show_health(&mut w, id);
    let got = take(&mut w, id);
    let want = slice(
        b":health\r\n",
        b"\x1b[79D\x1b[K\x1b[0;37;40mHealth",
        b"]\r\n",
    );
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "health");

    assert!(matches!(
        display_user_money(&mut w, id),
        silvermere_engine::commands::Handled::Done
    ));
    let got = take(&mut w, id);
    let want = slice(b":wealth\r\n", b"\x1b[79D\x1b[K\x1b[0;37;40mYou", b"!\r\n");
    assert_eq!(esc(&got), esc(want));

    display_users_keys(&mut w, id);
    let got = take(&mut w, id);
    let want = slice(b":keys\r\n", b"\x1b[79D\x1b[K\x1b[0;37;40mYou", b".\r\n");
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "inv_no_keys");

    display_users_spells(&mut w, id, Cmd::Spells);
    let got = take(&mut w, id);
    let want = slice(b":spells\r\n", b"You have no", b".\r\n");
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "spells_none");
}

// WHO with one player
#[test]
fn who_equals_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    display_scan_of_users(&mut w, id);
    let got = take(&mut w, id);
    let want = slice(b":who\r\n", b"\x1b[79D\x1b[K\x1b[1;33m", b" \r\n\r\n");
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "who_header");
    assert_line_matches(&got, "who_rule");
    insta::assert_snapshot!("who_one", esc(&got));
}

// TOP with an empty list, through the dispatcher: heading, titles, prompt
#[test]
fn top_equals_the_capture() {
    let mut w = world();
    let id = aldric(&mut w);
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Topten, &line("top"));
    let got = take(&mut w, id);
    let want = slice(
        b":top\r\n",
        b"\x1b[0;37;40m\x1b[79D\x1b[K\x1b[0;33mTop",
        b"=\r\n\x1b[0m",
    );
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "topten_header");
    let s = w.session(id).unwrap();
    assert_eq!(s.state, SessionState::Playing);
    assert!(s.output.is_empty());
}

// ---------------------------------------------------------------------------------------
// The status sheet variants.

// a Mage with the Mana ability and a raised stat
#[test]
fn st_with_mana_and_marks() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.class = ClassNo(MAGE);
        c.mana = 9;
        c.maxmana = 9;
        c.spellcast = 40;
        c.str = 55;
        c.abils[0] = (Ability::Mana, 1);
    });
    show_status(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "st_mana");
    let ls = lines(&got);
    assert!(
        ls.contains(&"Mana: *   9/9     Spellcasting: 40     Traps:           0".to_owned()),
        "{ls:?}"
    );
    assert!(ls.iter().any(|l| l.starts_with("Strength: *55")), "{ls:?}");
    assert!(esc(&got).contains("Strength: \\x1b[1;31m*55"));
    insta::assert_snapshot!("st_mage", esc(&got));
}

// ST for a Kai
#[test]
fn st_kai() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.class = ClassNo(MYSTIC);
        c.mana = 3;
        c.maxmana = 4;
    });
    show_status(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "st_mana");
    assert!(
        lines(&got)
            .contains(&"Kai:      3/4                          Traps:           0".to_owned())
    );
}

// poisoned, front rank, a spell with a descriptive message
#[test]
fn st_poisoned_ranked_and_spelled() {
    let mut w = world();
    let mut s = snapshot();
    s.spells[1].abilities[0] = (Ability::DescMsg, 500);
    w.reload_content(&s).unwrap();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.poison = 3;
        c.rank = 1;
        c.spells[0] = SpellNo(2);
        c.spellvals[0] = 10;
        c.spellrnds[0] = 40;
    });
    show_status(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "st_poisoned");
    assert_line_matches(&got, "st_rank");
    let ls = lines(&got);
    assert_eq!(
        ls[ls.len() - 1],
        "You are surrounded by a shimmering light."
    );
    // the back rank
    edit(&mut w, id, |c| c.rank = 2);
    show_status(&mut w, id);
    assert!(
        lines(&take(&mut w, id)).contains(&"You are in the back rank of your group.".to_owned())
    );
    // a sysop in debug mode sees the spells by name, value and rounds
    w.session_mut(id).unwrap().flags.debug = DebugLevel::On;
    show_status(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "st_spells_on");
    assert!(esc(&got).ends_with(
        "You have the following spell cast upon you:\\r\\n  light (10,40).\\r\\n\\r\\n"
    ));
    insta::assert_snapshot!("st_debug_spells", esc(&got));
}

#[test]
fn health_with_mana_and_kai() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.class = ClassNo(MAGE);
        c.hp = 30;
        c.mana = 5;
        c.maxmana = 10;
    });
    show_health(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "health");
    assert_eq!(
        lines(&got),
        ["Health:    30/35    [85%]  Mana:   5/10  [50%]"]
    );
    edit(&mut w, id, |c| c.class = ClassNo(MYSTIC));
    show_health(&mut w, id);
    assert_eq!(
        lines(&take(&mut w, id)),
        ["Health:    30/35    [85%]  Kai:   5/10  [50%]"]
    );
}

// below and above a billion
#[test]
fn exp_below_and_above_a_billion() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| c.exprem = 346);
    cmd_experience(&mut w, id, Cmd::Experience, &line("exp"));
    assert_eq!(
        lines(&take(&mut w, id)),
        ["Exp: 346 Level: 1 Exp needed for next level: 654 (1000) [34%]"]
    );
    // a level that needs more than a billion: the billions part and nine digits
    let level = (1..300)
        .find(|&l| silvermere_rules::progression::new_calc_exp_needed(l, 0).expbil > 0)
        .unwrap();
    let need = silvermere_rules::progression::new_calc_exp_needed(level, 0);
    edit(&mut w, id, |c| {
        c.level = level as u16;
        c.expbil = need.expbil;
        c.exprem = 5;
    });
    cmd_experience(&mut w, id, Cmd::Experience, &line("exp"));
    let got = take(&mut w, id);
    assert_line_matches(&got, "exp");
    let l = &lines(&got)[0];
    assert!(
        l.starts_with(&format!("Exp: {}000000005 Level: {level} ", need.expbil)),
        "{l}"
    );
    // with something after it, EXPERIENCE is no command
    assert_eq!(
        cmd_experience(&mut w, id, Cmd::Experience, &line("exp now")),
        silvermere_engine::commands::Handled::NotCommand
    );
}

// ---------------------------------------------------------------------------------------
// Inventory, keys, wealth.

// coins, worn, readied, weapon in hand, duplicates
#[test]
fn inventory_coins_readied_weapon_and_duplicates() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.coins = Coins {
            copper: 3,
            silver: 18,
            gold: 1,
            platinum: 0,
            runic: 2,
        };
        c.items[5] = ItemNo(DAGGER);
        c.items[6] = ItemNo(TORCH);
        c.items[7] = ItemNo(TORCH);
        c.items[8] = ItemNo(TORCH);
        c.itemuses[6] = 57;
        c.readied = 6;
        c.weapon = ItemNo(DAGGER);
        c.worn = std::array::from_fn(|_| ItemNo(0));
        c.worn[0] = ItemNo(VEST);
        c.keys[0] = ItemNo(BRASS_KEY);
        c.keys[1] = ItemNo(IRON_KEY);
        c.keys[2] = ItemNo(IRON_KEY);
    });
    display_inventory_items(&mut w, id);
    let got = take(&mut w, id);
    let ls = lines(&got);
    // the duplicates get a count but no plural
    assert_eq!(
        ls[..3].join(" "),
        "You are carrying 2 runic coins, 1 gold crown, 18 silver nobles, 3 copper \
         farthings, padded vest (Torso), torch (Readied/5), dagger (Weapon Hand), padded \
         pants, padded helm, padded gloves, padded boots, 2 torch"
    );
    assert!(ls[..3].iter().all(|l| l.len() <= 79));
    // the newest key kind first, the plural by an added `s`
    assert_eq!(
        ls[3],
        "You have the following keys:  2 iron keys, brass key."
    );
    assert_line_matches(&got, "inv_keys");
    insta::assert_snapshot!("i_mixed", esc(&got));
}

// the inventory's get_encumbrance_percent stores `encumpct`, which combat, the medium cycle
// and movement read
#[test]
fn inventory_stores_the_encumbrance_percentage() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| c.encumpct = 77);
    display_inventory_items(&mut w, id);
    take(&mut w, id);
    assert_eq!(w.session(id).unwrap().player().unwrap().encumpct, 20);
}

// nothing at all
#[test]
fn inventory_nothing() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.items = vec![ItemNo(0); 100];
        c.worn = std::array::from_fn(|_| ItemNo(0));
        c.weapon = ItemNo(0);
        c.wgt = 0;
    });
    display_inventory_items(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "inv_nothing");
    assert_line_matches(&got, "inv_encumbrance");
    assert!(lines(&got).contains(&"Encumbrance: 0/2400 - None [0%]".to_owned()));
}

#[test]
fn keys_and_wealth() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.keys[3] = ItemNo(BRASS_KEY);
        c.coins = Coins {
            copper: 23,
            silver: 0,
            gold: 2,
            platinum: 0,
            runic: 0,
        };
    });
    display_users_keys(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "inv_keys");
    assert_eq!(lines(&got), ["You have the following keys:  brass key."]);
    display_user_money(&mut w, id);
    let got = take(&mut w, id);
    assert_eq!(
        lines(&got),
        [
            "You have 2 gold crowns, 23 copper farthings.",
            "Wealth: 223 copper farthings"
        ]
    );
    assert_line_matches(&got, "inv_wealth");
}

// ---------------------------------------------------------------------------------------
// WHO.

// nobody in the game (the menu's S with the asker not in the realm)
#[test]
fn who_with_nobody() {
    let mut w = world();
    let id = aldric(&mut w);
    w.session_mut(id).unwrap().state = SessionState::MainMenu;
    display_scan_of_users(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "who_none");
    assert_eq!(
        esc(&got),
        "\\r\\n\\x1b[35mThere are no users in the game at the moment.\\r\\n"
    );
}

// alphabetical by first name; the name column as wide as the longest
// name; reputation, gossip, gang and EDITED; a hiding player is listed
#[test]
fn who_three_players_and_a_hidden_one() {
    let mut w = world();
    let id = aldric(&mut w);
    let zed = enter(&mut w, "zed", ENTRANCE, MessageLevel::High);
    edit(&mut w, zed, |c| {
        c.lastname = "Longername".into();
        c.title = "Squire".into();
        c.evil = 45;
        c.gang = "Old Guard".into();
        c.flags |= PlayerFlags::NOGOSSIP;
        c.hidden = 1;
    });
    let bo = enter(&mut w, "Bo", NORTH, MessageLevel::High);
    edit(&mut w, bo, |c| {
        c.title = "Novice".into();
        c.evil = -100;
        c.lawful = -10;
        c.immortal = 1;
    });
    display_scan_of_users(&mut w, id);
    let got = take(&mut w, id);
    let ls: Vec<String> = silvermere_text::strip_ansi(&got)
        .split(|&b| b == b'\n')
        .map(|l| {
            String::from_utf8_lossy(l)
                .trim_end_matches(['\r'])
                .to_owned()
        })
        .collect();
    assert_eq!(
        ls,
        [
            "         Current Adventurers",
            "         ===================",
            "",
            "         Aldric Stonebrook  -  Apprentice   ",
            "  Lawful Bo                 -  Novice  EDITED",
            "  Outlaw zed Longername     x  Squire  of Old Guard  ",
            "",
            "",
        ]
    );
    assert!(esc(&got).contains("\\x1b[31m  Outlaw "), "{}", esc(&got));
    insta::assert_snapshot!("who_three", esc(&got));
}

// the Technical message style
#[test]
fn who_technical_style() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.settings |= PlayerSettings::TECHNICAL;
        c.flags |= PlayerFlags::NOAUCTION;
    });
    display_scan_of_users(&mut w, id);
    let got = take(&mut w, id);
    let ls = lines(&got);
    assert_eq!(
        ls[0],
        "Title           Name                   Reputation Gang/Guild"
    );
    assert!(ls[1].starts_with("=-=-"));
    assert_eq!(
        ls[2],
        "Apprentice      Aldric Stonebrook    a Neutral    None"
    );
    insta::assert_snapshot!("who_technical", esc(&got));
}

// ---------------------------------------------------------------------------------------
// TOPTEN and the top gangs.

fn top_world(w: &mut World) {
    // the top-list rebuild (T-039's TopTen::rebuild): highest experience first, immortals
    // marked EDITED; Ann has the most, Bob the billions with a gang
    let chars: Vec<Character> = [
        ("Ann", WARRIOR),
        ("Bob", MAGE),
        ("Cy", WARRIOR),
        ("Di", MYSTIC),
        ("Ed", WARRIOR),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (n, class))| {
        let mut c = silvermere_engine::clear_player();
        c.userid = n.to_uppercase();
        c.firstname = n.into();
        c.lastname = "Smith".into();
        c.class = ClassNo(class);
        c.exprem = 5000 - i as u32 * 1000;
        match i {
            0 => c.expbil = 4,
            1 => {
                c.gang = "Old Guard".into();
                c.expbil = 3;
            }
            4 => c.immortal = 1,
            _ => {}
        }
        c
    })
    .collect();
    w.topten = silvermere_engine::progression::topten::TopTen::rebuild(&chars);
}

// one line per poll; the next tick shows the rest and the prompt
#[test]
fn top_default_pages_on_the_tick() {
    let mut h = Harness::new();
    let id = h.player("Aldric");
    top_world(&mut h.engine.world);
    h.line(id, "top");
    let first =
        String::from_utf8_lossy(&silvermere_text::strip_ansi(h.text(id).as_bytes())).into_owned();
    assert!(first.contains("  1. Ann Smith"), "{first}");
    assert!(!first.contains("Bob"), "{first}");
    assert_eq!(
        h.engine.world.session(id).unwrap().state,
        SessionState::Listing
    );
    h.tick();
    let mut rest = h.text(id);
    let prompt = h.prompt(id);
    assert!(rest.ends_with(&prompt), "{rest:?}");
    rest.truncate(rest.len() - prompt.len());
    let ls = lines(rest.as_bytes());
    assert_eq!(
        ls,
        [
            "2. Bob Smith             Mage       Old Guard           3000004000",
            "3. Cy Smith              Warrior    None                3000",
            "4. Di Smith              Mystic     None                2000",
            "5. Ed Smith              Warrior    None                1000         EDITED",
        ]
    );
    assert_eq!(
        h.engine.world.session(id).unwrap().state,
        SessionState::Playing
    );
}

// `X` alone stops the list, without a prompt; other lines are
// swallowed
#[test]
fn top_interrupted_by_x() {
    let mut h = Harness::new();
    let id = h.player("Aldric");
    top_world(&mut h.engine.world);
    h.line(id, "top");
    h.text(id);
    h.line(id, "look");
    assert_eq!(h.text(id), "");
    h.line(id, "x");
    assert_eq!(h.text(id), "Command interrupted.\r\n");
    let s = h.engine.world.session(id).unwrap();
    assert_eq!(s.state, SessionState::Playing);
    assert!(!s.ust.flags.contains(SessionFlags::TOPTEN));
    h.tick();
    assert!(!h.text(id).contains("Bob"));
}

// TOPTEN {class}, numbered within the class, `Class` before `Rank`
#[test]
fn top_filtered_by_class() {
    let mut w = world();
    let id = aldric(&mut w);
    top_world(&mut w);
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Topten, &line("top war"));
    silvermere_engine::state::listing::on_tick(&mut w);
    let got = take(&mut w, id);
    let ls = lines(&got);
    assert!(ls[2].starts_with("ClassRank Name"), "{ls:?}");
    assert_eq!(
        &ls[4..],
        [
            "1. Ann Smith             Warrior    None                4000005000",
            "2. Cy Smith              Warrior    None                3000",
            "3. Ed Smith              Warrior    None                1000         EDITED",
        ]
    );
    insta::assert_snapshot!("top_warriors", esc(&got));
}

// the menu's T, all at once, then the prompt
#[test]
fn menu_t_lists_without_paging() {
    let mut w = world();
    let id = aldric(&mut w);
    top_world(&mut w);
    display_only_topten(&mut w, id);
    let got = take(&mut w, id);
    assert_line_matches(&got, "topten_header");
    let ls = lines(&got);
    assert_eq!(ls[0], "Heroes of The Realm");
    assert_eq!(ls[2], "Ran Name                  Class      Gang/Guild");
    assert_eq!(ls.len(), 9);
    assert_eq!(
        ls[8],
        "5. Ed Smith              Warrior    None                EDITED"
    );
}

#[test]
fn top_gangs() {
    let mut w = world();
    let id = aldric(&mut w);
    display_top_gangs(&mut w, id, 10);
    assert_eq!(
        lines(&take(&mut w, id)),
        ["There are no gangs currently established!"]
    );
    for (key, exp, flags) in [
        ("OLD GUARD", 500, GangFlags::empty()),
        ("NIGHT", 900, GangFlags::empty()),
        ("GONE", 2000, GangFlags::DISBANDED),
    ] {
        w.gangs.insert(
            key.into(),
            Gang {
                key: key.into(),
                name: key.to_lowercase(),
                exp,
                leader: "Nester".into(),
                created: 1_791_936_000,
                members: 3,
                flags,
                overexp: 0,
                overflows: 0,
            },
        );
    }
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Topten, &line("top gangs"));
    let got = take(&mut w, id);
    assert_line_matches(&got, "topten_header");
    let ls = lines(&got);
    assert_eq!(
        ls[2],
        "Rank Gangname            Leader      Members Created"
    );
    assert_eq!(ls[4], "1. night               Nester         3    10/14/26");
    assert_eq!(ls[5], "2. old guard           Nester         3    10/14/26");
    assert_eq!(ls.len(), 6);
    insta::assert_snapshot!("top_gangs", esc(&got));
}

// ---------------------------------------------------------------------------------------
// SPELLS and POWERS.

// sorted by level then name
#[test]
fn spells_for_a_mage() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| {
        c.class = ClassNo(MAGE);
        c.spellbook[0] = SpellNo(3);
        c.spellbook[1] = SpellNo(1);
        c.spellbook[2] = SpellNo(4);
        c.spellbook[3] = SpellNo(2);
        c.spellbook[4] = SpellNo(77); // gone from WCCSPELS
    });
    display_users_spells(&mut w, id, Cmd::Spells);
    let got = take(&mut w, id);
    assert_line_matches(&got, "spells_header");
    assert_line_matches(&got, "spells_columns");
    let plain = String::from_utf8_lossy(&silvermere_text::strip_ansi(&got)).into_owned();
    assert_eq!(
        plain,
        "You have the following spells:\r\n\
         Level Mana Short Spell Name\r\n  \
         1   1    ligh  light                         \r\n  \
         1   2    mmis  magic missile                 \r\n  \
         3   4          shield                        \r\n  \
         5   10   fbal  fireball                      \r\n\r\n"
    );
    let book = &w.session(id).unwrap().player().unwrap().spellbook;
    assert_eq!(
        &book[..5],
        [SpellNo(2), SpellNo(1), SpellNo(4), SpellNo(3), SpellNo(0)]
    );
    insta::assert_snapshot!("spells_mage", esc(&got));
}

#[test]
fn spells_and_powers_refused() {
    let mut w = world();
    let id = aldric(&mut w);
    edit(&mut w, id, |c| c.class = ClassNo(MYSTIC));
    display_users_spells(&mut w, id, Cmd::Spells);
    let got = take(&mut w, id);
    assert_line_matches(&got, "spells_wrong_kind");
    assert_eq!(
        lines(&got),
        ["You may not list your spells. You are KAI! You must list your powers."]
    );
    display_users_spells(&mut w, id, Cmd::Powers);
    assert_eq!(lines(&take(&mut w, id)), ["You have no powers."]);
    edit(&mut w, id, |c| c.class = ClassNo(WARRIOR));
    display_users_spells(&mut w, id, Cmd::Powers);
    assert_eq!(
        lines(&take(&mut w, id)),
        ["You may not list your powers. You are not KAI! You must list your spells."]
    );
}

// ---------------------------------------------------------------------------------------
// HELP.

#[test]
fn help_overview_topic_unknown_and_topics() {
    let mut w = world();
    let id = aldric(&mut w);
    // HELP alone and `?` (which parses to HELP)
    assert_eq!(parse_command("?"), Some(Cmd::Help));
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Help, &line("help"));
    let got = take(&mut w, id);
    assert!(lines(&got)[0].starts_with("Type HELP followed by a topic"));
    // HELP BASH: the combat topic, whose words include "bash"
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Help, &line("help bash"));
    let got = take(&mut w, id);
    assert!(
        lines(&got).iter().any(|l| l.starts_with("Bash <OPPONENT>")),
        "{:?}",
        lines(&got)
    );
    // HELP XYZZY
    silvermere_engine::commands::dispatch(&mut w, id, Cmd::Help, &line("help xyzzy"));
    let got = take(&mut w, id);
    assert_line_matches(&got, "help_none");
    // the dispatcher's ANSI_NORMAL first (handle_commands)
    assert_eq!(
        esc(&got),
        "\\x1b[0;37;40mSorry, no help is available for that topic!\\r\\n"
    );
    // HELP TOPICS
    help_text(&mut w, id, Some("topics"), 2, "");
    let got = take(&mut w, id);
    assert_line_matches(&got, "help_topics");
    let ls = lines(&got);
    assert!(
        ls[1].starts_with("Commands, Communications, Combat, "),
        "{ls:?}"
    );
    assert!(ls.last().unwrap().ends_with("Help ?."));
    assert!(ls.iter().all(|l| l.len() <= 81));
    // HELP EXPERIENCE: levels 1 to 10 for a level 1 character
    help_text(&mut w, id, Some("experience"), 2, "");
    let ls = lines(&take(&mut w, id));
    let at = ls.iter().position(|l| l == "Level   Experience").unwrap();
    assert_eq!(ls[at + 2], "2     1000");
    assert_eq!(ls.len() - at, 12);
}

// ---------------------------------------------------------------------------------------
// LOOK {direction}.

// a closed door; no exit, per direction wording
#[test]
fn look_through_a_closed_door_and_into_nothing() {
    let mut w = world();
    let id = enter(&mut w, "Aldric", ARENA, MessageLevel::High);
    look_direction(&mut w, id, 0);
    let got = take(&mut w, id);
    assert_line_matches(&got, "look_door_closed");
    assert_eq!(esc(&got), "The door is closed in that direction!\\r\\n");
    look_direction(&mut w, id, 1);
    let got = take(&mut w, id);
    assert_line_matches(&got, "look_no_exit");
    look_direction(&mut w, id, 9);
    assert_eq!(lines(&take(&mut w, id)), ["There are no exits downwards!"]);
    edit(&mut w, id, |c| c.room = RoomId(NORTH));
    look_direction(&mut w, id, 8);
    assert_eq!(lines(&take(&mut w, id)), ["There are no exits upwards!"]);
}

// the room beyond as on arrival, and the two notices
#[test]
fn look_into_the_next_room() {
    let mut h = Harness::new();
    let id = h.player("Aldric");
    let here = h.player("Bob");
    let there = h.player("Cy");
    edit(&mut h.engine.world, there, |c| c.room = RoomId(NORTH));
    h.line(id, "look n");
    let got = h.text(id);
    let ls = lines(got.as_bytes());
    assert_eq!(ls[0], "Newhaven, Market Square");
    assert!(ls.iter().any(|l| l == "Also here: Cy."), "{ls:?}");
    assert!(
        ls.iter().any(|l| l.starts_with("Obvious exits: s")),
        "{ls:?}"
    );
    let ls = lines(h.text(here).as_bytes());
    assert!(pattern("look_others").is_match(&ls[0]), "{ls:?}");
    assert!(ls[0].starts_with("Aldric is looking to the "), "{ls:?}");
    let ls = lines(h.text(there).as_bytes());
    assert!(pattern("peek_in").is_match(&ls[0]), "{ls:?}");
    assert!(ls[0].starts_with("Aldric peeks in from the "), "{ls:?}");
    // and back where he was
    let p = h.engine.world.session(id).unwrap().player().unwrap();
    assert_eq!(p.room, RoomId(ENTRANCE));
}

// OPEN and CLOSE change the live exit, and LOOK reads it
#[test]
fn look_through_a_door_opened_and_closed_in_play() {
    for (dir, word, room_name) in [
        ("w", "west", "Newhaven, Armoury"),
        ("e", "east", "Newhaven, Vault"),
    ] {
        let mut h = Harness::new();
        let id = h.player("Aldric");
        edit(&mut h.engine.world, id, |c| c.room = RoomId(ARENA));
        h.text(id);
        h.line(id, &format!("look {dir}"));
        assert_eq!(
            lines(h.text(id).as_bytes())[0],
            "The door is closed in that direction!"
        );
        if dir == "e" {
            // locked: the key unlocks it, then it opens
            h.line(id, "open e");
            for _ in 0..5 {
                h.tick();
            }
            h.text(id);
            use_item(
                &mut h.engine.world,
                id,
                ItemNo(BRASS_KEY),
                UseTarget::Direction(silvermere_types::enums::Direction::East),
            );
            h.text(id);
        }
        h.line(id, &format!("open {dir}"));
        for _ in 0..5 {
            h.tick();
        }
        h.text(id);
        let here = h.player("Bob");
        edit(&mut h.engine.world, here, |c| c.room = RoomId(ARENA));
        let there = h.player("Cy");
        edit(&mut h.engine.world, there, |c| {
            c.room = RoomId(if dir == "w" { 2152 } else { 2153 })
        });
        h.text(here);
        h.text(there);
        h.line(id, &format!("look {dir}"));
        let ls = lines(h.text(id).as_bytes());
        assert_eq!(ls[0], room_name, "{ls:?}");
        assert!(ls.iter().any(|l| l == "Also here: Cy."), "{ls:?}");
        let ls = lines(h.text(here).as_bytes());
        assert!(
            ls[0].starts_with(&format!("Aldric is looking to the {word}")),
            "{ls:?}"
        );
        let ls = lines(h.text(there).as_bytes());
        assert!(ls[0].starts_with("Aldric peeks in from the "), "{ls:?}");
        h.line(id, &format!("close {dir}"));
        for _ in 0..5 {
            h.tick();
        }
        h.text(id);
        h.line(id, &format!("look {dir}"));
        assert_eq!(
            lines(h.text(id).as_bytes())[0],
            "The door is closed in that direction!"
        );
    }
}

// ---------------------------------------------------------------------------------------
// LOOK {name}.

#[test]
fn look_at_a_monster() {
    let mut w = world();
    let id = aldric(&mut w);
    monster(&mut w, 9, ENTRANCE, 7);
    look_at(&mut w, id, "rat");
    let got = take(&mut w, id);
    assert_line_matches(&got, "look_monster");
    assert_line_matches(&got, "look_poisoned");
    assert_eq!(
        lines(&got),
        [
            "giant rat",
            "A huge, filthy rat with yellowed teeth.",
            "It appears to be moderately wounded.",
            "It is poisoned!"
        ]
    );
    insta::assert_snapshot!("look_monster", esc(&got));
}

#[test]
fn look_at_an_item() {
    let mut w = world();
    let id = aldric(&mut w);
    look_at(&mut w, id, "quarter");
    let got = take(&mut w, id);
    assert_eq!(
        esc(&got),
        "\\x1b[79D\\x1b[K\\x1b[37mA stout wooden staff, shod with iron. \\r\\n"
    );
}

// looking at nothing, and at the spell book
#[test]
fn look_at_nothing_and_at_a_spell() {
    let mut w = world();
    let id = aldric(&mut w);
    look_at(&mut w, id, "unicorn");
    assert_eq!(lines(&take(&mut w, id)), ["You do not see unicorn here!"]);
    edit(&mut w, id, |c| c.spellbook[0] = SpellNo(1));
    look_at(&mut w, id, "magic");
    assert_eq!(
        lines(&take(&mut w, id)),
        ["magic missile (mmis):", "A bolt of force."]
    );
}

// the description and the two notices
#[test]
fn look_at_a_player() {
    let mut h = Harness::new();
    let id = h.player("Aldric");
    let bob = h.player("Bob");
    let cy = h.player("Cy");
    let w = &mut h.engine.world;
    let c = w.session(id).unwrap().player().unwrap().clone();
    edit(w, bob, |b| {
        b.lastname = "TheDupe".into();
        b.gang = "Old Guard".into();
        b.race = RaceNo(1);
        b.class = ClassNo(WARRIOR);
        b.sex = b'M';
        b.str = 55;
        b.agi = 45;
        b.hea = 50;
        b.cha = 75;
        b.intel = 62;
        b.wil = 85;
        b.hp = 10;
        b.maxhp = 10;
        b.hairlen = 1;
        b.haircol = 4;
        b.eyecol = 13;
        b.items = c.items.clone();
        b.itemuses = c.itemuses.clone();
        b.items[0] = ItemNo(VEST);
        b.items[1] = ItemNo(STAFF);
        b.items[2] = ItemNo(TORCH);
        b.worn[0] = ItemNo(VEST);
        b.weapon = ItemNo(STAFF);
        b.readied = 2;
    });
    h.line(id, "look bob");
    let mut got = h.text(id);
    let prompt = h.prompt(id);
    assert!(got.ends_with(&prompt));
    got.truncate(got.len() - prompt.len());
    assert_line_matches(got.as_bytes(), "look_player_header");
    assert_line_matches(got.as_bytes(), "look_player");
    assert_line_matches(got.as_bytes(), "look_equipped");
    let ls = lines(got.as_bytes());
    assert_eq!(ls[0], "[ Bob TheDupe ] (Old Guard)");
    assert!(
        ls[1].starts_with("Bob is a healthy, moderately built Human Warrior with short brown hair"),
        "{ls:?}"
    );
    assert!(ls[1..].iter().all(|l| l.len() <= 79));
    // inventory order: the worn vest in slot 0, the readied torch in slot 2, the weapon
    assert_eq!(
        &ls[ls.len() - 3..],
        [
            "padded vest                    (Torso)",
            "torch                          (Readied)",
            "quarterstaff                   (Weapon Hand)"
        ]
    );
    let all: Vec<&str> = ls.iter().flat_map(|l| l.split_whitespace()).collect();
    let all = all.join(" ");
    for needle in [
        "hazel eyes. He moves slugishly, and is charismatic and outgoing. You can't help but like him.",
        "Bob appears to be bright and seems to possess a wisdom beyond his years. He is unwounded.",
        "He is equipped with:",
    ] {
        assert!(all.contains(needle), "{all}");
    }
    let ls = lines(h.text(bob).as_bytes());
    assert_eq!(ls[0], "Aldric is looking at you.");
    let ls = lines(h.text(cy).as_bytes());
    assert!(pattern("look_up_down").is_match(&ls[0]));
    assert_eq!(ls[0], "Aldric looks Bob up and down.");
    insta::assert_snapshot!("look_player", esc(got.as_bytes()));
}

// ---------------------------------------------------------------------------------------
// The main menu.

// the screens without their copyright lines, then the bare prompt
#[test]
fn menu_screens() {
    let mut h = Harness::new();
    let id = h.player("Aldric");
    h.engine.world.session_mut(id).unwrap().state = SessionState::MainMenu;
    for key in ["I", "C", "A"] {
        h.line(id, key);
        let t = h.text(id);
        assert!(!t.contains("Copyright"), "{key}: {t}");
        assert!(t.contains("M A J O R"), "{key}");
        assert!(
            t.ends_with("\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m"),
            "{key}"
        );
    }
    h.engine.world.session_mut(id).unwrap().caps.ansi = false;
    h.line(id, "c");
    let t = h.text(id);
    assert!(t.contains(" |      Rooms:    over 19,000"), "{t}");
    assert!(!t.contains("Copyright"));
    // T, G and S/W: the lists, then `\r[MAJORMUD]: `
    for key in ["T", "G", "S", "W", "R"] {
        h.line(id, key);
        let t = h.text(id);
        assert!(
            t.ends_with("\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m"),
            "{key}: {t:?}"
        );
    }
    h.line(id, "g");
    assert!(
        h.text(id)
            .contains("There are no gangs currently established!")
    );
    // the player is at the menu, not in the game: WHO lists nobody
    h.line(id, "w");
    assert!(
        h.text(id)
            .contains("There are no users in the game at the moment.")
    );
    h.line(id, "t");
    assert!(h.text(id).contains("Heroes of The Realm"));
    // H: the help overview and its prompt
    h.line(id, "h");
    let t = h.text(id);
    assert!(t.contains("Type \x1b[1;33mHELP"), "{t}");
    assert!(t.ends_with("Enter Help Topic (x=exit,?=list) >\x1b[0;37;40m"));
}

// ---------------------------------------------------------------------------------------
// The engine harness: lines in, bytes out.

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    pending: Vec<EngineOutput>,
}

impl Harness {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let engine = Engine::new(world(), clock.clone(), SeededRng::new(7), tx);
        Self {
            engine,
            clock,
            rx,
            pending: Vec::new(),
        }
    }

    fn player(&mut self, name: &str) -> SessionId {
        let w = &mut self.engine.world;
        let id = if name == "Aldric" {
            aldric(w)
        } else {
            enter(w, name, ENTRANCE, MessageLevel::High)
        };
        edit(w, id, |c| c.movedir = -1);
        id
    }

    fn line(&mut self, id: SessionId, text: &str) {
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
    }

    fn tick(&mut self) {
        self.clock.advance_ticks(1, &mut self.engine);
    }

    /// Everything written to `id` since the last call, as text.
    fn text(&mut self, id: SessionId) -> String {
        while let Ok(o) = self.rx.try_recv() {
            self.pending.push(o);
        }
        // writes for the other sessions are kept for their own call
        let mut out = Vec::new();
        let mut rest = Vec::new();
        for o in std::mem::take(&mut self.pending) {
            match o {
                EngineOutput::Write { id: to, bytes } if to == id => out.extend(bytes),
                other => rest.push(other),
            }
        }
        self.pending = rest;
        String::from_utf8_lossy(&out).into_owned()
    }

    fn prompt(&self, id: SessionId) -> String {
        String::from_utf8_lossy(&silvermere_engine::tell::prompt_bytes(
            &self.engine.world,
            id,
        ))
        .into_owned()
    }
}

// the plain `C` screen is the original's text byte for byte, less the copyright
// row, then the bare prompt
#[test]
fn menu_counts_plain_equals_the_text() {
    const PRF: [&str; 19] = [
        " +----------------------------------------------------------------------------+\r",
        " |                                                                            |\r",
        " |                           M A J O R   M U D                                |\r",
        " |                                                                            |\r",
        " |    Here are some statistical pieces of information regarding the 1.1       |\r",
        " |    release of MajorMUD (with all the addons activated):                    |\r",
        " |                                                                            |\r",
        " |      Rooms:    over 19,000                                                 |\r",
        " |      Monsters: over 750                                                    |\r",
        " |      Items:    over 1800                                                   |\r",
        " |      Spells:   over 1200                                                    |\r",
        " |      Shops:    over 150                                                    |\r",
        " |      Classes:  15                                                          |\r",
        " |      Races:    13                                                          |\r",
        " |                                                                            |\r",
        " |                                                                            |\r",
        " |                                                                            |\r",
        " |                                                                            |\r",
        " +----------------------------------------------------------------------------+\r\r",
    ];
    let mut h = Harness::new();
    let id = h.player("Aldric");
    {
        let s = h.engine.world.session_mut(id).unwrap();
        s.state = SessionState::MainMenu;
        s.caps.ansi = false;
    }
    h.line(id, "C");
    let want = PRF.concat().replace('\r', "\r\n")
        + "\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m";
    assert_eq!(h.text(id), want);
}
