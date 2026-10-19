//! T-103: the commands that were placeholders until now, through `execute_input`:
//! SNEAK (`cmd_sneak`), HIDE alone (`cmd_hide`), TRACK (`cmd_track`;
//! `track_player`/`track_monster`), DISARM TRAP (`cmd_disarm`), MAP (`cmd_map`),
//! FEEDBACK, and TAME, MESMERIZE and IDENTITY, which 1.11p never
//! implemented (they are not commands); with them `monster_could_attack`, which
//! HIDE and SNEAK ask. The output is compared byte for byte; where a roll decides, the
//! world's rng is seeded and the test reads the next draws from a clone first.

use crate::common;

use common::{enter, test_config, tiny_snapshot};
use silvermere_engine::SessionId;
use silvermere_engine::combat::AutoTarget;
use silvermere_engine::combat::autocombat::engage;
use silvermere_engine::commands::input::execute_input;
use silvermere_engine::commands::{Cmd, CommandLine, Handled, misc, parse_command};
use silvermere_engine::world::{World, blank};
use silvermere_rules::combat::Move;
use silvermere_rules::{Rng, SeededRng};
use silvermere_types::enums::{Ability, DebugLevel, ExitType, GroupRole, MessageLevel};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{
    ActiveMonsterId, ItemNo, MapId, MessageNo, MonsterNo, RoomId, RoomKey,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Character, Coins, Exit, Message};

// The stock distribution, downloaded once and checked (the helper is declared in `all.rs`).
use crate::stock;

const HALL: u32 = 3000;
const NORTH: u32 = 3001;
const ORC: MonsterNo = MonsterNo(30);

/// The dispatcher's `ANSI_NORMAL`.
const NORMAL: &str = "\x1b[0;37;40m";
const CLR: &str = "\x1b[79D\x1b[K";
/// The prompt of a 10 HP character.
const PROMPT: &str = "\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:";
/// `dirnames[DIR_NORTH]` as the world holds it (letter slot and backspace).
const NORTH_NAME: &str = "n \u{8}orth";
const NO_EFFECT: &str = "\x1b[0;37mYour command had no effect.\r\n";

fn exit(target: u32, kind: ExitType) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

/// The hall with `north` as its exit to the north room, and back.
fn world_with(north: Exit) -> World {
    let mut s = tiny_snapshot();
    let mut hall = blank::clear_room(RoomKey::new(1, HALL));
    hall.name = "Hall".into();
    hall.exits[0] = north;
    let mut room = blank::clear_room(RoomKey::new(1, NORTH));
    room.name = "North Room".into();
    room.exits[1] = exit(HALL, ExitType::Normal);
    s.rooms.push(hall);
    s.rooms.push(room);
    let mut k = blank::clear_known_monster(ORC);
    k.name = "orc".into();
    k.maxhp = 20;
    k.energy = 1000;
    s.monsters.push(k);
    for (n, texts) in [
        (
            50,
            [
                "You hear a click.",
                "%s trips a wire!",
                "Something clicks near %s.",
            ],
        ),
        (
            51,
            [
                "The needle is safe.",
                "%s makes the needle safe.",
                "%s works on a lock.",
            ],
        ),
    ] {
        s.messages.push(Message {
            number: MessageNo(n),
            msgs: texts.map(str::to_owned),
        });
    }
    let mut w = World::load(test_config(), 8, s).expect("world loads");
    w.rng = SeededRng::new(103);
    w
}

fn world() -> World {
    world_with(exit(NORTH, ExitType::Normal))
}

fn plr(w: &mut World, id: SessionId) -> &mut Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

/// An orc in `room`, its slot filled.
fn spawn(w: &mut World, number: i32, room: u32) -> ActiveMonsterId {
    let k = w.monsters.get(&ORC).unwrap().clone();
    let number = ActiveMonsterId(number);
    let mon = ActiveMonster {
        number,
        knownnum: k.number,
        level: k.level,
        map: MapId(1),
        room: RoomId(room),
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
        genroom: RoomId(room),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: k.montype,
        target: -1,
        hpregen: k.hpregen,
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
    };
    w.active_monsters.insert(mon);
    let rs = w.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
    let slot = rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap();
    *slot = number;
    number
}

fn monster(w: &mut World, n: ActiveMonsterId) -> &mut ActiveMonster {
    let k = w.active_monsters.slot_of(n).unwrap();
    w.active_monsters.get_mut(k).unwrap()
}

/// Everything written to `id` since the last call, ANSI kept, the prompt rendered.
fn raw(w: &mut World, id: SessionId) -> String {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    String::from_utf8_lossy(&b).into_owned()
}

fn run(w: &mut World, id: SessionId, line: &str) -> String {
    raw(w, id);
    execute_input(w, id, line, false);
    raw(w, id)
}

/// `out` up to the prompt.
fn before_prompt(out: &str) -> &str {
    out.rfind("\x1b[79D\x1b[K\x1b[0;37m[HP=")
        .map_or(out, |i| &out[..i])
}

/// The next `n` rolls in [0, 100) the world will make.
fn next_rolls(w: &World, n: usize) -> Vec<i32> {
    let mut r = w.rng.clone();
    (0..n).map(|_| r.range(0, 100)).collect()
}

fn aldric(w: &mut World) -> SessionId {
    let id = enter(w, "Aldric", HALL, MessageLevel::High);
    plr(w, id).perception = 0;
    id
}

// ---------------------------------------------------------------------------------------
// TAME, MESMERIZE, IDENTITY: not commands.

#[test]
fn tame_mesmerize_and_identity_are_not_commands() {
    let mut w = world();
    let a = aldric(&mut w);
    let unknown = run(&mut w, a, "xyzzy bob");
    assert_eq!(unknown, format!("{NO_EFFECT}{PROMPT}"));
    for line in ["tame bob", "mesmerize bob", "identity bob", "tame", "id"] {
        assert_eq!(run(&mut w, a, line), unknown, "{line}");
    }
}

// ---------------------------------------------------------------------------------------
// SNEAK.

#[test]
fn sneak_that_fails_unnoticed_looks_like_success() {
    let mut w = world();
    let a = aldric(&mut w);
    // stealth 0: chance 0, the first roll always fails; perception 0: never noticed
    plr(&mut w, a).status.insert(PlayerStatus::RESTING);
    assert_eq!(
        run(&mut w, a, "sneak"),
        format!("{NORMAL}{CLR}Attempting to sneak...\r\n")
    );
    let p = plr(&mut w, a);
    assert!(!p.status.contains(PlayerStatus::SNEAKING));
    assert!(!p.status.contains(PlayerStatus::RESTING));
    assert_eq!(p.delay, 1, "one tick of delay");
}

#[test]
fn sneak_that_fails_and_is_noticed() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).perception = 100;
    assert_eq!(
        run(&mut w, a, "sneak"),
        format!("{NORMAL}{CLR}Attempting to sneak...You don't think you're sneaking.\r\n")
    );
    assert!(!plr(&mut w, a).status.contains(PlayerStatus::SNEAKING));
}

#[test]
fn sneak_rolls_against_the_hide_sneak_chance() {
    // stealth 50 alone in the room: chance 50 (calc_hide_sneak_chance)
    for seed in 0..40 {
        let mut w = world();
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        plr(&mut w, a).stealth = 50;
        plr(&mut w, a).perception = 50;
        let r = next_rolls(&w, 2);
        let out = run(&mut w, a, "sneak");
        let sneaking = plr(&mut w, a).status.contains(PlayerStatus::SNEAKING);
        assert_eq!(sneaking, r[0] < 50, "seed {seed}");
        let noticed = !sneaking && r[1] < 50;
        let tail = if noticed {
            "You don't think you're sneaking.\r\n"
        } else {
            "\r\n"
        };
        assert_eq!(
            out,
            format!("{NORMAL}{CLR}Attempting to sneak...{tail}"),
            "seed {seed}"
        );
    }
}

#[test]
fn sneak_caps_the_chance_at_95() {
    let mut seen_fail = false;
    for seed in 0..200 {
        let mut w = world();
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        plr(&mut w, a).stealth = 300;
        let r = next_rolls(&w, 1);
        run(&mut w, a, "sneak");
        let sneaking = plr(&mut w, a).status.contains(PlayerStatus::SNEAKING);
        assert_eq!(sneaking, r[0] < 95, "seed {seed}");
        seen_fail |= !sneaking;
    }
    assert!(seen_fail, "a roll of 95 or more fails even at stealth 300");
}

#[test]
fn perfect_stealth_always_sneaks_and_a_debugging_sysop_is_told() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).abils[0] = (Ability::PerfectStealth, 1);
    let r = w.rng.clone();
    assert_eq!(
        run(&mut w, a, "sneak"),
        format!("{NORMAL}{CLR}Attempting to sneak...\r\n")
    );
    assert!(plr(&mut w, a).status.contains(PlayerStatus::SNEAKING));
    assert_eq!(w.rng, r, "no roll is made");
    plr(&mut w, a).delay = 0;
    w.session_mut(a).unwrap().flags.debug = DebugLevel::On;
    let out = run(&mut w, a, "sneak");
    assert!(
        out.ends_with(&format!(
            "{CLR}Attempting to sneak...\r\n{NORMAL}PERFECT STEALTH\r\n"
        )),
        "{out:?}"
    );
}

/// `id` types `line` straight into its handler (a line typed while a delay runs is
/// queued by `execute_input`, so this is how a handler meets a running delay).
fn handler(
    w: &mut World,
    id: SessionId,
    line: &str,
    f: fn(&mut World, SessionId, Cmd, &CommandLine) -> Handled,
) -> (Handled, String) {
    raw(w, id);
    let line = CommandLine::parse(line);
    let cmd = parse_command(line.arg(0)).unwrap();
    let rc = f(w, id, cmd, &line);
    (rc, raw(w, id))
}

#[test]
fn sneak_must_wait_for_the_delay() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).delay = 3;
    plr(&mut w, a).status.insert(PlayerStatus::SNEAKING);
    assert_eq!(
        handler(&mut w, a, "sneak", misc::cmd_sneak),
        (
            Handled::Done,
            format!("{CLR}You must wait before you may do that!\r\n")
        )
    );
    let p = plr(&mut w, a);
    assert!(
        !p.status.contains(PlayerStatus::SNEAKING),
        "the earlier sneak ended"
    );
    assert_eq!(p.delay, 3, "no delay added");
}

#[test]
fn sneak_with_a_word_is_not_this_command_and_more_words_show_the_syntax() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).status.insert(PlayerStatus::SNEAKING);
    assert_eq!(
        run(&mut w, a, "sneak north"),
        format!("{NO_EFFECT}{PROMPT}")
    );
    assert!(
        !plr(&mut w, a).status.contains(PlayerStatus::SNEAKING),
        "SNEAK {{word}} still ends the sneak"
    );
    assert_eq!(
        run(&mut w, a, "sneak to the north"),
        format!("{NORMAL}\x1b[1;31;40mSyntax: SNEAK\r\n{PROMPT}")
    );
    assert_eq!(plr(&mut w, a).delay, 0);
}

#[test]
fn sneak_is_refused_next_to_a_hostile_monster() {
    let mut w = world();
    let a = aldric(&mut w);
    spawn(&mut w, 1, HALL);
    assert_eq!(
        run(&mut w, a, "sneak"),
        format!("{NORMAL}You may not sneak right now!\r\n{PROMPT}")
    );
    assert_eq!(plr(&mut w, a).delay, 1, "the refusal costs a tick");
}

#[test]
fn sneak_is_refused_while_engaged() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).attackers = 1;
    assert_eq!(
        run(&mut w, a, "sneak"),
        format!("{NORMAL}You may not sneak right now!\r\n{PROMPT}")
    );
}

#[test]
fn sneak_breaks_off_autocombat() {
    let mut w = world();
    let a = aldric(&mut w);
    let orc = spawn(&mut w, 1, NORTH);
    engage(
        &mut w,
        a,
        Some(AutoTarget::Monster(orc)),
        Move::Attack,
        None,
    );
    let out = run(&mut w, a, "sneak");
    assert!(
        out.starts_with(&format!("{NORMAL}{CLR}\x1b[0;33m*Combat Off*\r\n")),
        "{out:?}"
    );
    assert!(out.ends_with("Attempting to sneak...\r\n"), "{out:?}");
    assert!(!silvermere_engine::combat::autocombat::is_inside_autocombat(&w, a));
}

// ---------------------------------------------------------------------------------------
// HIDE alone.

#[test]
fn hide_that_fails_unnoticed_looks_like_success() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).hidden = 1;
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}{CLR}Attempting to hide...\r\n")
    );
    let p = plr(&mut w, a);
    assert_eq!(p.hidden, 0, "the earlier hiding is given up");
    assert_eq!(p.delay, 1);
}

#[test]
fn hide_that_fails_and_is_noticed() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).perception = 100;
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}{CLR}Attempting to hide... You don't think you are hidden.\r\n")
    );
}

#[test]
fn hide_rolls_against_the_chance_capped_at_95() {
    for seed in 0..60 {
        let mut w = world();
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        plr(&mut w, a).stealth = 300;
        plr(&mut w, a).perception = 100;
        let r = next_rolls(&w, 1);
        let out = run(&mut w, a, "hide");
        let hidden = plr(&mut w, a).hidden == 1;
        assert_eq!(hidden, r[0] < 95, "seed {seed}");
        let tail = if hidden {
            "\r\n"
        } else {
            " You don't think you are hidden.\r\n"
        };
        assert_eq!(out, format!("{NORMAL}{CLR}Attempting to hide...{tail}"));
    }
}

#[test]
fn hide_in_a_fight_fails_at_once() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).stealth = 300;
    spawn(&mut w, 1, HALL);
    let r = w.rng.clone();
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}Attempting to hide... You don't think you are hidden.\r\n")
    );
    assert_eq!(plr(&mut w, a).hidden, 0);
    assert_eq!(plr(&mut w, a).delay, 1);
    assert_eq!(w.rng, r, "no roll");
}

#[test]
fn hide_held_or_knocked_down_cannot_move() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).status2.insert(PlayerStatus2::HELD);
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}You can't seem to move anywhere to hide!\r\n")
    );
    plr(&mut w, a).status2 = PlayerStatus2::KNOCKEDDOWN;
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}You are too stunned to move anywhere to hide!\r\n")
    );
    assert_eq!(plr(&mut w, a).delay, 0, "no delay for either");
}

#[test]
fn hide_must_wait_for_the_delay() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).delay = 2;
    assert_eq!(
        handler(&mut w, a, "hide", misc::cmd_hide),
        (
            Handled::Wait,
            format!("{CLR}You must wait before you may do that!\r\n")
        )
    );
    assert_eq!(plr(&mut w, a).delay, 2);
}

// ---------------------------------------------------------------------------------------
// monster_could_attack, through HIDE.

#[test]
fn friendly_and_charmed_monsters_do_not_stop_a_hide() {
    let mut w = world();
    let a = aldric(&mut w);
    let m = spawn(&mut w, 1, HALL);
    monster(&mut w, m).friendly = 1;
    let m2 = spawn(&mut w, 2, HALL);
    monster(&mut w, m2)
        .flags
        .insert(ActiveMonsterFlags::CHARMED);
    monster(&mut w, m2).following = "aldric".into();
    let out = run(&mut w, a, "hide");
    assert_eq!(out, format!("{NORMAL}{CLR}Attempting to hide...\r\n"));
}

#[test]
fn a_monster_held_back_by_an_item_makes_the_room_safe_carried_or_not() {
    let mut w = world();
    let a = aldric(&mut w);
    let mut k = w.monsters.get(&ORC).unwrap().clone();
    k.abilities[0] = (Ability::NoAttackItem, 77);
    w.monsters.insert(ORC, k);
    spawn(&mut w, 1, HALL);
    // a second, hostile orc after it is never looked at (sic)
    let m2 = spawn(&mut w, 2, HALL);
    monster(&mut w, m2).knownnum = MonsterNo(1);
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}{CLR}Attempting to hide...\r\n")
    );
    // the item number equal to lastmon: hostile, lastmoncnt counted down
    plr(&mut w, a).delay = 0;
    plr(&mut w, a).lastmon = 77;
    plr(&mut w, a).lastmoncnt = 5;
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}Attempting to hide... You don't think you are hidden.\r\n")
    );
    assert_eq!(plr(&mut w, a).lastmoncnt, 4);
}

#[test]
fn a_sneaker_just_arrived_is_not_attacked() {
    let mut w = world();
    let a = aldric(&mut w);
    spawn(&mut w, 1, HALL);
    plr(&mut w, a)
        .status
        .insert(PlayerStatus::SNEAKING | PlayerStatus::JUSTARRIVED);
    assert_eq!(
        run(&mut w, a, "hide"),
        format!("{NORMAL}{CLR}Attempting to hide...\r\n")
    );
}

// ---------------------------------------------------------------------------------------
// TRACK.

#[test]
fn track_alone_shows_the_syntax() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).status2.insert(PlayerStatus2::MEDITATING);
    assert_eq!(
        run(&mut w, a, "track"),
        format!("{NORMAL}\x1b[1;31;40mSyntax: TRACK {{monster}}\r\n{PROMPT}")
    );
    assert!(!plr(&mut w, a).status2.contains(PlayerStatus2::MEDITATING));
    assert_eq!(plr(&mut w, a).delay, 0);
}

/// Bob walked from the hall to the north room.
fn bob_went_north(w: &mut World) -> SessionId {
    let b = enter(w, "Bob", NORTH, MessageLevel::High);
    let p = plr(w, b);
    p.trailrooms[0] = RoomId(NORTH);
    p.trailrooms[1] = RoomId(HALL);
    p.trailmaps[0] = MapId(1);
    p.trailmaps[1] = MapId(1);
    b
}

#[test]
fn track_a_player_through_the_room() {
    let mut w = world();
    let a = aldric(&mut w);
    bob_went_north(&mut w);
    plr(&mut w, a).tracking = 100;
    assert_eq!(
        run(&mut w, a, "track bob"),
        format!("Bob went {NORTH_NAME} from here.\r\n")
    );
    assert_eq!(plr(&mut w, a).delay, 1, "a tick of delay, no prompt");
}

#[test]
fn track_without_skill_fails() {
    let mut w = world();
    let a = aldric(&mut w);
    bob_went_north(&mut w);
    let fail = format!("{CLR}\x1b[1;31mYour tracking skills fail you this time.\r\n");
    assert_eq!(run(&mut w, a, "track bob"), fail);
    plr(&mut w, a).delay = 0;
    assert_eq!(run(&mut w, a, "track nobody"), fail, "no one of that name");
}

#[test]
fn track_rolls_against_the_tracking_skill() {
    for seed in 0..40 {
        let mut w = world();
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        bob_went_north(&mut w);
        plr(&mut w, a).tracking = 50;
        let r = next_rolls(&w, 1);
        let out = run(&mut w, a, "track bob");
        let expect = if r[0] < 50 {
            format!("Bob went {NORTH_NAME} from here.\r\n")
        } else {
            format!("{CLR}\x1b[1;31mYour tracking skills fail you this time.\r\n")
        };
        assert_eq!(out, expect, "seed {seed}");
    }
}

#[test]
fn track_a_monster_by_its_trail() {
    let mut w = world();
    let a = aldric(&mut w);
    plr(&mut w, a).tracking = 100;
    let m = spawn(&mut w, 1, NORTH);
    monster(&mut w, m).trail[0] = RoomId(NORTH);
    monster(&mut w, m).trail[1] = RoomId(HALL);
    // the cache holds the monsters the game has looked up (find_monster_in_memory)
    w.active_monsters.touch(m);
    assert_eq!(
        run(&mut w, a, "track orc"),
        format!("orc went {NORTH_NAME} from here.\r\n")
    );
    // a player in the hall is found first in the room, then not on the trail
    plr(&mut w, a).delay = 0;
    let c = enter(&mut w, "Cora", HALL, MessageLevel::High);
    let _ = c;
    assert_eq!(
        run(&mut w, a, "track cora"),
        format!("{CLR}\x1b[1;31mYour tracking skills fail you this time.\r\n")
    );
}

// ---------------------------------------------------------------------------------------
// DISARM TRAP.

fn trap(kind: ExitType, state: i16) -> Exit {
    Exit {
        target: RoomId(NORTH),
        kind,
        par1: 8,
        par2: state,
        par3: 51,
        par4: 50,
    }
}

fn exit_state(w: &World) -> i16 {
    silvermere_engine::movement::exits::live_exit(
        w,
        RoomKey::new(1, HALL),
        silvermere_types::enums::Direction::North,
    )
    .unwrap()
    .par2
}

#[test]
fn disarm_a_trap() {
    for (state, after) in [(0, 1), (3, 4)] {
        let mut w = world_with(trap(ExitType::Trap, state));
        let a = aldric(&mut w);
        plr(&mut w, a).disarm = 200;
        assert_eq!(
            run(&mut w, a, "disarm trap n"),
            format!("{NORMAL}You successfully disarmed the trap to the {NORTH_NAME}.\r\n{PROMPT}")
        );
        assert_eq!(exit_state(&w), after);
    }
}

#[test]
fn disarm_a_spell_trap_shows_its_message() {
    let mut w = world_with(trap(ExitType::SpellTrap, 0));
    let a = aldric(&mut w);
    let watcher = enter(&mut w, "Cora", HALL, MessageLevel::High);
    let beyond = enter(&mut w, "Dain", NORTH, MessageLevel::High);
    raw(&mut w, watcher);
    raw(&mut w, beyond);
    plr(&mut w, a).disarm = 200;
    assert_eq!(
        run(&mut w, a, "disarm trap north"),
        format!(
            "{NORMAL}You successfully disarmed the trap to the {NORTH_NAME}.\r\n\
             {CLR}\x1b[0;32mThe needle is safe.\r\n{PROMPT}"
        )
    );
    assert_eq!(exit_state(&w), 1);
    let seen = raw(&mut w, watcher);
    assert!(
        seen.starts_with(&format!(
            "{CLR}\x1b[1;33m\x1b[0;32mAldric makes the needle safe.\r\n"
        )),
        "{seen:?}"
    );
    let heard = raw(&mut w, beyond);
    assert!(
        heard.starts_with(&format!(
            "{CLR}\x1b[1;33m\x1b[0;32mAldric works on a lock.\r\n"
        )),
        "{heard:?}"
    );
}

#[test]
fn a_near_miss_on_a_trap_only_fails() {
    for seed in 0..40 {
        let mut w = world_with(trap(ExitType::Trap, 0));
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        let roll = next_rolls(&w, 1)[0];
        plr(&mut w, a).disarm = (roll - 5) as i16;
        assert_eq!(
            run(&mut w, a, "disarm trap n"),
            format!(
                "{NORMAL}{CLR}\x1b[1;31mYou failed to disarm any trap to the {NORTH_NAME}.\r\n{PROMPT}"
            ),
            "seed {seed}"
        );
        assert_eq!(exit_state(&w), 0, "still armed");
        assert_eq!(plr(&mut w, a).hp, 10);
    }
}

#[test]
fn a_bad_miss_sets_the_trap_off() {
    for seed in 0..20 {
        let mut w = world_with(trap(ExitType::Trap, 0));
        w.rng = SeededRng::new(seed);
        let a = aldric(&mut w);
        let watcher = enter(&mut w, "Cora", HALL, MessageLevel::High);
        raw(&mut w, watcher);
        plr(&mut w, a).disarm = -100;
        let mut r = w.rng.clone();
        r.range(0, 100);
        let damage = r.range(4, 9);
        // the room is told from the buffer that still holds the dispatcher's ANSI_NORMAL:
        // it goes to the listeners, not to the player
        let out = run(&mut w, a, "disarm trap n");
        assert_eq!(
            before_prompt(&out),
            format!("{CLR}\x1b[0;32mYou hear a click.\r\n"),
            "seed {seed}"
        );
        assert_eq!(i32::from(plr(&mut w, a).hp), 10 - damage);
        assert!((4..=8).contains(&damage));
        let seen = raw(&mut w, watcher);
        assert!(
            seen.starts_with(&format!(
                "{NORMAL}{CLR}\x1b[1;33m\x1b[0;32mAldric trips a wire!\r\n"
            )),
            "{seen:?}"
        );
    }
}

#[test]
fn a_bad_miss_on_a_state_3_trap_carries_the_player_through() {
    let mut w = world_with(trap(ExitType::Trap, 3));
    let a = aldric(&mut w);
    plr(&mut w, a).disarm = -100;
    let out = run(&mut w, a, "disarm trap n");
    assert!(
        out.starts_with(&format!("{CLR}\x1b[0;32mYou hear a click.\r\n")),
        "{out:?}"
    );
    assert_eq!(plr(&mut w, a).room, RoomId(NORTH));
}

#[test]
fn a_failed_spell_trap_fails_then_goes_off() {
    let mut w = world_with(trap(ExitType::SpellTrap, 0));
    let a = aldric(&mut w);
    plr(&mut w, a).disarm = -100;
    assert_eq!(
        run(&mut w, a, "disarm trap n"),
        format!(
            "{NORMAL}{CLR}\x1b[1;31mYou failed to disarm any trap to the {NORTH_NAME}.\r\n\
             {CLR}\x1b[0;32mYou hear a click.\r\n{PROMPT}"
        )
    );
}

#[test]
fn nothing_to_disarm() {
    let fail = format!(
        "{NORMAL}{CLR}\x1b[1;31mYou failed to disarm any trap to the {NORTH_NAME}.\r\n{PROMPT}"
    );
    // a normal exit, and a trap already disarmed
    for north in [exit(NORTH, ExitType::Normal), trap(ExitType::Trap, 1)] {
        let mut w = world_with(north);
        let a = aldric(&mut w);
        plr(&mut w, a).disarm = 200;
        assert_eq!(run(&mut w, a, "disarm trap n"), fail);
    }
    // a spell trap that is not armed says nothing at all (sic)
    let mut w = world_with(trap(ExitType::SpellTrap, 4));
    let a = aldric(&mut w);
    assert_eq!(run(&mut w, a, "disarm trap n"), format!("{NORMAL}{PROMPT}"));
    // not a direction: nothing
    assert_eq!(
        run(&mut w, a, "disarm trap door"),
        format!("{NORMAL}{PROMPT}")
    );
}

#[test]
fn any_other_disarm_is_unready() {
    let mut w = world();
    let a = aldric(&mut w);
    let unready = run(&mut w, a, "unready");
    plr(&mut w, a).status.insert(PlayerStatus::SNEAKING);
    for line in ["disarm", "disarm trap", "disarm sword now"] {
        assert_eq!(run(&mut w, a, line), unready, "{line}");
    }
    assert!(!plr(&mut w, a).status.contains(PlayerStatus::SNEAKING));
}

// ---------------------------------------------------------------------------------------
// MAP.

fn with_mapfile(w: &mut World, file: &str) {
    let key = RoomKey::new(1, HALL);
    let mut room = w.room(key).unwrap().clone();
    room.mapfile = file.into();
    w.rooms.insert(key, room);
}

#[test]
fn map_without_a_map_file() {
    let mut w = world();
    let a = aldric(&mut w);
    assert_eq!(
        run(&mut w, a, "map"),
        format!("{NORMAL}\x1b[1;31;40mThere is no map available for this room.\r\n{PROMPT}")
    );
}

#[test]
fn map_sends_the_file_then_an_empty_line() {
    let mut w = world();
    let a = aldric(&mut w);
    with_mapfile(&mut w, "hall.ans");
    w.display.assets.insert(
        "HALL.ANS",
        b"\x1b[1;33m+--+\r\n|  |\r\n+--+\r\n\x1aSAUCE00 rest".to_vec(),
    );
    assert_eq!(
        run(&mut w, a, "map"),
        format!("{NORMAL}{CLR}\x1b[1;33m+--+\r\n|  |\r\n+--+\r\n\r\n{PROMPT}")
    );
}

// The stock map of the Sysop Support Chamber (1:164) against the original's bytes for MAP
// there, from the m4/stealth live run (tests/fixtures/map/README.md); the map file is the
// stock distribution's
#[test]
fn map_of_the_stock_town_matches_the_original_byte_for_byte() {
    let mut w = world();
    let a = aldric(&mut w);
    with_mapfile(&mut w, "WCCMAP01.ANS");
    w.display.assets.insert(
        "WCCMAP01.ANS",
        std::fs::read(stock::assets_dir().join("WCCMAP01.ANS")).unwrap(),
    );
    execute_input(&mut w, a, "map", false);
    let prompt = silvermere_engine::tell::prompt_bytes(&w, a);
    let palette = silvermere_engine::tell::palette_of(&w, a);
    let s = w.session_mut(a).unwrap();
    let b = s.output.render(palette, &prompt);
    let cut = b
        .windows(8)
        .rposition(|x| x == b"\x1b[79D\x1b[K")
        .expect("a prompt");
    assert_eq!(
        &b[..cut],
        include_bytes!("fixtures/map/orig-map.bin").as_slice()
    );
}

#[test]
fn map_file_that_cannot_be_read() {
    let mut w = world();
    let a = aldric(&mut w);
    with_mapfile(&mut w, "gone.ans");
    assert_eq!(run(&mut w, a, "map"), format!("{NORMAL}{CLR}\r\n{PROMPT}"));
}

#[test]
fn map_reads_at_most_500_pieces_of_499_characters() {
    let mut w = world();
    let a = aldric(&mut w);
    with_mapfile(&mut w, "big.ans");
    let mut file = vec![b'x'; 600];
    file.push(b'\n');
    for _ in 0..600 {
        file.extend_from_slice(b"y\n");
    }
    w.display.assets.insert("BIG.ANS", file);
    let out = run(&mut w, a, "map");
    let body = out
        .strip_prefix(&format!("{NORMAL}{CLR}"))
        .and_then(|b| b.strip_suffix(&format!("\r\n{PROMPT}")))
        .expect("framed");
    let mut expect = "x".repeat(499);
    expect.push_str(&"x".repeat(101));
    expect.push_str("\r\n");
    expect.push_str(&"y\r\n".repeat(498));
    assert_eq!(body, expect);
}

// ---------------------------------------------------------------------------------------
// FEEDBACK.

#[test]
fn feedback_alone_shows_the_syntax() {
    let mut w = world();
    let a = aldric(&mut w);
    assert_eq!(
        run(&mut w, a, "feedback"),
        format!("{NORMAL}{CLR}\x1b[1;31mSyntax: FEEDBACK {{comments}}\r\n{PROMPT}")
    );
}

#[test]
fn feedback_is_sent_and_thanked() {
    let mut w = world();
    let a = aldric(&mut w);
    assert_eq!(
        run(&mut w, a, "fe the orcs are too strong"),
        format!("{NORMAL}{CLR}Thank you for your input... email sent.\r\n{PROMPT}")
    );
}
