//! Combat orchestration (T-033) through the engine: the attack commands
//! (`cmd_any_attack`), a player's round on a monster (`attack_user_monster`), a monster's
//! round on a player (`attack_monster_user`), PvP (`attack_user_user`), the kills
//! (`check_kill_monster`, `check_kill_user`) and ROB (`rob_user`).
//!
//! The records are the stock ones: class 2 Witchunter (combat 7,
//! Bash, Anti-Magic, Hit Magical 10), race 11 Kang, item 100 quarterstaff (2-12, speed
//! 1200, hit verbs WCCMSG 8237 `smack|whap|beat`, miss verbs 8289 `swipe at`), item 67
//! shortsword, known monsters 1 giant rat, 7 kobold thief and 5 acid slime with their
//! WCCMSG hit/deflect/miss/death records (27, 41, 35, 8294, 8296, 8297, 8307, 8309,
//! 8310). The auto-combat ring is T-034's: a round here is a direct call with
//! [`Mode::Auto`] and a full energy pool, as `do_autocombat_for_user` will make it.

use crate::common;

use common::{enter, test_config, tiny_snapshot};
use silvermere_engine::combat::{
    self, KillOutcome, Mode, attack_monster_user, attack_user_monster, attack_user_user,
    check_kill_user,
};
use silvermere_engine::commands::input::execute_input;
use silvermere_engine::world::{World, blank};
use silvermere_engine::{AccountRef, SessionId};
use silvermere_rules::SeededRng;
use silvermere_rules::combat::{Move, Outcome};
use silvermere_text::strip_ansi;
use silvermere_types::enums::GroupRole;
use silvermere_types::enums::{
    Ability, Alignment, ItemType, MessageLevel, MonsterAttackType, MonsterType, RoomType,
};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, PlayerStatus2, RoomFlags};
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId,
    RoomKey,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins, Message};

const ARENA: u32 = 2150;
const REINC: u32 = 2189;
const KANG: RaceNo = RaceNo(11);
const WITCHUNTER: ClassNo = ClassNo(2);
const QUARTERSTAFF: ItemNo = ItemNo(100);
const SHORTSWORD: ItemNo = ItemNo(67);
const RAT: MonsterNo = MonsterNo(1);
const SLIME: MonsterNo = MonsterNo(5);
const KOBOLD: MonsterNo = MonsterNo(7);

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

/// The stock WCCMSG records the fights read.
fn messages() -> Vec<Message> {
    vec![
        msg(
            27,
            "The %s bites you for %d damage!",
            "The %s bites %s for %s damage!",
            "The giant rat falls to the ground with a tortured squeak.",
        ),
        msg(
            35,
            "The %s whips you with its pseudopod for %d damage!",
            "The %s whips %s with its pseudopod for %s damage!",
            "The acid slime dissolves into a puddle of bluish goo.",
        ),
        msg(
            41,
            "The %s stabs you for %d damage!",
            "The %s stabs %s for %s damage!",
            "The kobold thief falls to the ground with a shrill cry.",
        ),
        msg(
            8237,
            "smack|whap|beat",
            "smacks|whaps|beats",
            "smacks|whaps|beats",
        ),
        msg(8289, "swipe at", "swipes at", "swipes at"),
        msg(
            8228,
            "impale|skewer|slash",
            "impales|skewers|slashes",
            "impales|skewers|slashes",
        ),
        msg(8287, "lunge at", "lunges at", "lunges at"),
        msg(
            8294,
            "The %s %slunges at %s, %sbut %s dodges out of the way!",
            "The %s %slunges at %syou!",
            "The %s %slunges at %s! %s",
        ),
        msg(
            8296,
            "The %s %slashes at %s, %sbut %s dodges out of the way!",
            "The %s %sflails at you!",
            "The %s %sflails at %s!",
        ),
        msg(
            8297,
            "The %s %s you, but your armour deflects the blow!",
            "The %s %s %s, but %s armour deflects the blow!",
            "The %s %s you with their %s, but you dodge!",
        ),
        msg(
            8307,
            "The %s %slunges at you, but your armour deflects the blow!",
            "The %s %slunges at %s, but %s armour deflects the blow!",
            "The %s %slunges at %syou, but you dodge out of the way!",
        ),
        msg(
            8309,
            "The %s %slashes at you, but your armour deflects the blow!",
            "The %s %slashes at %s, but %s armour deflects the blow!",
            "The %s %slashes at %syou, but you dodge out of the way!",
        ),
        msg(
            8310,
            "The %s %s %s with their %s, but %s dodges!",
            "The %s %s you with their %s!",
            "The %s %s %s with their %s!",
        ),
    ]
}

/// One attack slot of a known monster.
// one argument per record field the fights read
#[allow(clippy::too_many_arguments)]
fn attack(
    kind: MonsterAttackType,
    acc: i16,
    pct: u8,
    min: i16,
    max: i16,
    energy: i16,
    hit: i32,
    def: i32,
    miss: i32,
) -> silvermere_types::records::MonsterAttack {
    silvermere_types::records::MonsterAttack {
        kind,
        accuracy: acc,
        pct,
        min,
        max,
        hitmsg: MessageNo(hit),
        defmsg: MessageNo(def),
        missmsg: MessageNo(miss),
        energy,
        hitspell: Default::default(),
    }
}

// one argument per record field the fights read
#[allow(clippy::too_many_arguments)]
fn known(
    n: MonsterNo,
    name: &str,
    ac: i16,
    dr: i16,
    exp: i32,
    maxhp: i16,
    weapon: i32,
    slot0: silvermere_types::records::MonsterAttack,
    death: i32,
) -> silvermere_types::records::KnownMonster {
    let mut k = blank::clear_known_monster(n);
    k.name = name.into();
    k.level = 1;
    k.ac = ac;
    k.dr = dr;
    k.exp = exp;
    k.maxhp = maxhp;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.montype = MonsterType::Lair;
    k.followpct = 20;
    k.magicres = 30;
    k.weapon = ItemNo(weapon);
    k.deathmsg = MessageNo(death);
    k.attacks[0] = slot0;
    k
}

/// The arena, the reincarnation room, the stock records.
fn world() -> World {
    let mut s = tiny_snapshot();
    let mut arena = blank::clear_room(RoomKey::new(1, ARENA));
    arena.name = "Newhaven Arena".into();
    s.rooms.push(arena);
    s.messages = messages();
    let mut staff = blank::clear_item(QUARTERSTAFF);
    staff.name = "quarterstaff".into();
    staff.item_type = ItemType::Weapon;
    staff.weight = 100;
    staff.mindmg = 2;
    staff.maxdmg = 12;
    staff.speed = 1200;
    staff.strreq = 30;
    staff.hitmsg = MessageNo(8237);
    staff.missmsg = MessageNo(8289);
    staff.robable = 1;
    s.items.push(staff);
    let mut sword = blank::clear_item(SHORTSWORD);
    sword.name = "shortsword".into();
    sword.item_type = ItemType::Weapon;
    sword.weight = 65;
    sword.mindmg = 2;
    sword.maxdmg = 9;
    sword.speed = 1000;
    sword.strreq = 30;
    sword.hitmsg = MessageNo(8228);
    sword.missmsg = MessageNo(8287);
    sword.robable = 1;
    s.items.push(sword);
    s.monsters.push(known(
        RAT,
        "giant rat",
        0,
        1,
        9,
        12,
        0,
        attack(
            MonsterAttackType::Normal,
            10,
            100,
            2,
            10,
            1000,
            27,
            8307,
            8294,
        ),
        27,
    ));
    s.monsters.push(known(
        KOBOLD,
        "kobold thief",
        10,
        1,
        13,
        20,
        67,
        attack(
            MonsterAttackType::Normal,
            15,
            100,
            1,
            8,
            666,
            41,
            8297,
            8310,
        ),
        41,
    ));
    s.monsters.push(known(
        SLIME,
        "acid slime",
        0,
        0,
        16,
        25,
        0,
        attack(
            MonsterAttackType::Normal,
            20,
            100,
            2,
            9,
            1000,
            35,
            8309,
            8296,
        ),
        35,
    ));
    let mut witchunter = blank::clear_class(WITCHUNTER);
    witchunter.name = "Witchunter".into();
    witchunter.exp = 30;
    witchunter.combat = 7;
    witchunter.hplevel = 4;
    witchunter.basehp = 6;
    witchunter.abilities[0] = (Ability::Bash, 0);
    witchunter.abilities[1] = (Ability::AntiMagic, 0);
    witchunter.abilities[2] = (Ability::HitMagical, 10);
    s.classes.push(witchunter);
    let mut kang = blank::clear_race(KANG);
    kang.name = "Kang".into();
    kang.exp = 35;
    kang.abilities[0] = (Ability::PoisonImmune, 100);
    kang.abilities[1] = (Ability::DamageResist, 10);
    kang.abilities[2] = (Ability::Defence, 5);
    s.races.push(kang);
    let mut config = test_config();
    config.death_hp = -10;
    let mut w = World::load(config, 8, s).expect("world loads");
    w.rng = SeededRng::new(7);
    w
}

/// A level-1 Kang Witchunter in the arena with the quarterstaff wielded.
fn kang(w: &mut World, name: &str, msglevel: MessageLevel) -> SessionId {
    let id = enter(w, name, ARENA, msglevel);
    let s = w.session_mut(id).unwrap();
    s.account = Some(AccountRef {
        id: AccountId(i64::from(id.0) + 1),
        username: name.to_ascii_uppercase(),
        sysop: false,
        gameop: false,
        sex: b'M',
    });
    s.created = true;
    let c = s.character.as_mut().unwrap();
    c.settings = PlayerSettings::VALIDATED;
    c.race = KANG;
    c.class = WITCHUNTER;
    c.level = 1;
    (c.str, c.intel, c.wil, c.hea, c.agi, c.cha) = (55, 30, 45, 50, 30, 30);
    (c.hp, c.maxhp) = (48, 48);
    (c.energy, c.maxenergy) = (1000, 1000);
    c.lives = 9;
    c.items[0] = QUARTERSTAFF;
    c.itemuses[0] = -1;
    c.weapon = QUARTERSTAFF;
    id
}

/// An active monster of `known` in `room`, numbered `n`, full of hit points and energy.
fn spawn(w: &mut World, known: MonsterNo, room: u32, n: u16) -> ActiveMonsterId {
    let k = w.monsters.get(&known).unwrap().clone();
    let number = ActiveMonsterId(i32::from(n));
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
    let key = RoomKey::new(1, room);
    let rs = w.room_state.get_mut(&key).unwrap();
    let slot = rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap();
    *slot = number;
    number
}

/// Everything written to `id` since the last call, ANSI kept.
fn raw(w: &mut World, id: SessionId) -> String {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    String::from_utf8_lossy(&b).into_owned()
}

/// The screen lines written to `id` since the last call: ANSI off, split at line ends,
/// prompts and empty lines dropped.
fn lines(w: &mut World, id: SessionId) -> Vec<String> {
    // ANSI_CLRLINE (ESC[79D ESC[K) wipes the prompt the line was written after
    let text = raw(w, id).replace("\x1b[79D\x1b[K", "\r\n");
    let plain = String::from_utf8_lossy(&strip_ansi(text.as_bytes())).into_owned();
    plain
        .split("\r\n")
        .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

fn hp(w: &World, id: SessionId) -> i16 {
    w.session(id).unwrap().player().unwrap().hp
}

/// One auto-combat round of the player's: a full pool first, as the energy tick gives it.
fn round(w: &mut World, id: SessionId, mon: ActiveMonsterId) -> Option<Outcome> {
    w.session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .energy = 1000;
    attack_user_monster(w, id, mon, Move::Attack, Move::Attack, Mode::Auto)
}

/// One round of the monster's, with its pool full.
fn monster_round(w: &mut World, mon: ActiveMonsterId, id: SessionId) -> Option<Outcome> {
    if let Some(m) = w.active_monsters.values_mut().find(|m| m.number == mon) {
        m.energy = m.maxenergy as u16;
    }
    attack_monster_user(w, mon, id)
}

fn alive(w: &World, mon: ActiveMonsterId) -> bool {
    w.active_monsters.values().any(|m| m.number == mon)
}

// the whole fight.
#[test]
fn kang_witchunter_kills_a_giant_rat() {
    let mut w = world();
    let kasimir = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let rat = spawn(&mut w, RAT, ARENA, 1);
    lines(&mut w, kasimir);
    lines(&mut w, bob);

    // the command: no blow yet
    execute_input(&mut w, kasimir, "a rat", false);
    let me = lines(&mut w, kasimir);
    assert!(me.contains(&"*Combat Engaged*".to_owned()), "{me:?}");
    let them = lines(&mut w, bob);
    assert!(
        them.contains(&"Kasimir moves to attack giant rat.".to_owned()),
        "{them:?}"
    );
    assert_eq!(w.session(kasimir).unwrap().player().unwrap().energy, 0);

    // the rounds, the rat answering, until it dies
    let mut mine = Vec::new();
    let mut seen = Vec::new();
    for _ in 0..20 {
        round(&mut w, kasimir, rat);
        mine.extend(lines(&mut w, kasimir));
        seen.extend(lines(&mut w, bob));
        if !alive(&w, rat) {
            break;
        }
        monster_round(&mut w, rat, kasimir);
        mine.extend(lines(&mut w, kasimir));
        seen.extend(lines(&mut w, bob));
    }
    assert!(!alive(&w, rat), "{mine:#?}");
    let hit = regex::Regex::new(r"^You (smack|whap|beat) giant rat for \d+ damage!$").unwrap();
    assert!(mine.iter().any(|l| hit.is_match(l)), "{mine:#?}");
    let room_hit =
        regex::Regex::new(r"^Kasimir (smacks|whaps|beats) giant rat for \d+ damage!$").unwrap();
    assert!(seen.iter().any(|l| room_hit.is_match(l)), "{seen:#?}");
    assert!(
        mine.contains(&"The giant rat falls to the ground with a tortured squeak.".to_owned()),
        "{mine:#?}"
    );
    assert!(seen.contains(&"The giant rat falls to the ground with a tortured squeak.".to_owned()));
    assert!(
        mine.contains(&"You gain 9 experience.".to_owned()),
        "{mine:#?}"
    );
    insta::assert_snapshot!("rat_fight", mine.join("\n"));
}

fn plr(w: &mut World, id: SessionId) -> &mut silvermere_types::records::Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

fn mon(w: &mut World, n: ActiveMonsterId) -> &mut ActiveMonster {
    w.active_monsters
        .values_mut()
        .find(|m| m.number == n)
        .unwrap()
}

fn run(w: &mut World, id: SessionId, line: &str) -> Vec<String> {
    lines(w, id);
    execute_input(w, id, line, false);
    lines(w, id)
}

#[test]
fn too_afraid() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    plr(&mut w, k).status2.insert(PlayerStatus2::AFRAID);
    assert!(run(&mut w, k, "a rat").contains(&"You are too afraid!".to_owned()));
}

// a protected room
#[test]
fn guilt_in_a_protected_room() {
    let mut w = world();
    w.rooms
        .get_mut(&RoomKey::new(1, ARENA))
        .unwrap()
        .flags
        .insert(RoomFlags::PROTECTED);
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    let out = run(&mut w, k, "a rat");
    assert!(
        out.contains(
            &"You are overcome with a feeling of guilt and break off your attack.".to_owned()
        ),
        "{out:?}"
    );
}

#[test]
fn the_required_weapon() {
    let mut w = world();
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::ReqToHit, 5);
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    let out = run(&mut w, k, "a rat");
    assert!(
        out.contains(&"You are not using the required weapon to hit this monster!".to_owned()),
        "{out:?}"
    );
}

// Hit Magical 10 (Witchunter) against Magical 15
#[test]
fn hit_magical_against_a_magical_monster() {
    let mut w = world();
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::Magical, 15);
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    let out = run(&mut w, k, "a rat");
    assert!(
        out.contains(&"Your weapon has no effect against this monster!".to_owned()),
        "{out:?}"
    );
    plr(&mut w, k).weapon = ItemNo(0);
    let out = run(&mut w, k, "a rat");
    assert!(
        out.contains(&"Your fists have no effect against this monster!".to_owned()),
        "{out:?}"
    );
    // Magical 10 is covered by Hit Magical alone
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::Magical, 10);
    let out = run(&mut w, k, "a rat");
    assert!(out.contains(&"*Combat Engaged*".to_owned()), "{out:?}");
}

#[test]
fn a_guard_moves_to_protect() {
    let mut w = world();
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::Guarded, KOBOLD.0 as i16);
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    let kobold = spawn(&mut w, KOBOLD, ARENA, 2);
    let out = run(&mut w, k, "a rat");
    assert_eq!(out[0], "kobold thief moves to protect giant rat", "{out:?}");
    assert!(out.contains(&"*Combat Engaged*".to_owned()));
    assert_eq!(mon(&mut w, kobold).following, "Kasimir");
}

#[test]
fn bash_smash_and_killblow_gates() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    spawn(&mut w, RAT, ARENA, 1);
    plr(&mut w, k).weapon = ItemNo(0);
    assert_eq!(
        run(&mut w, k, "bash rat"),
        ["You need a weapon to bash with!"]
    );
    plr(&mut w, k).weapon = QUARTERSTAFF;
    assert!(run(&mut w, k, "bash rat").contains(&"*Combat Engaged*".to_owned()));
    assert_eq!(
        run(&mut w, k, "smash rat"),
        ["You don't know the first thing about smashing!"]
    );
    w.classes.get_mut(&WITCHUNTER).unwrap().abilities[0] = (Ability::None, 0);
    assert_eq!(
        run(&mut w, k, "bash rat"),
        ["You don't know the first thing about bashing!"]
    );
    // KILLBLOW is no game command: whatever the parser says next is not combat
    let out = run(&mut w, k, "killblow rat");
    assert!(!out.contains(&"*Combat Engaged*".to_owned()), "{out:?}");
}

// the room sees misses only at the high message level
#[test]
fn misses_by_message_level() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let high = kang(&mut w, "Hilda", MessageLevel::High);
    let medium = kang(&mut w, "Mona", MessageLevel::Medium);
    let rat = spawn(&mut w, RAT, ARENA, 1);
    mon(&mut w, rat).ac = 2000; // DV 2000: every blow but one in ten misses
    for _ in 0..5 {
        round(&mut w, k, rat);
    }
    let mine = lines(&mut w, k);
    assert!(
        mine.iter().any(|l| l == "You swipe at giant rat!"),
        "{mine:?}"
    );
    let h = lines(&mut w, high);
    assert!(
        h.iter().any(|l| l == "Kasimir swipes at giant rat!"),
        "{h:?}"
    );
    let m = lines(&mut w, medium);
    assert!(!m.iter().any(|l| l.contains("swipes at")), "{m:?}");
}

// the armour takes it all
#[test]
fn a_blow_glances_off() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let rat = spawn(&mut w, RAT, ARENA, 1);
    mon(&mut w, rat).dr = 100; // 10 points off every blow
    for _ in 0..4 {
        round(&mut w, k, rat);
    }
    let mine = lines(&mut w, k);
    assert!(
        mine.iter()
            .any(|l| l == "Your swipe at giant rat hits, but glances off its armour."),
        "{mine:?}"
    );
    let seen = lines(&mut w, bob);
    assert!(
        seen.iter()
            .any(|l| l == "Kasimir's swipe at giant rat hits, but glances off its armour."),
        "{seen:?}"
    );
}

#[test]
fn smash_knocks_a_monster_down_and_it_rises() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let rat = spawn(&mut w, RAT, ARENA, 1);
    w.config.smash_rounds = 2;
    plr(&mut w, k).str = 200; // 200+1 > 1+30+30
    mon(&mut w, rat).hp = 30000;
    mon(&mut w, rat).maxhp = 30000;
    let mut all = Vec::new();
    // one SMASH round, then plain rounds: the count goes down at the start of each
    // (a knockdown sets it again, so it only runs out without one)
    for mv in [Move::Smash, Move::Attack, Move::Attack, Move::Attack] {
        w.session_mut(k).unwrap().character.as_mut().unwrap().energy = 1000;
        attack_user_monster(&mut w, k, rat, mv, mv, Mode::Auto);
        all.extend(lines(&mut w, k));
    }
    assert!(
        all.contains(&"The giant rat is smashed to the floor defenseless!".to_owned()),
        "{all:?}"
    );
    assert!(
        all.contains(&"Slightly dazed the giant rat rises from the floor.".to_owned()),
        "{all:?}"
    );
}

// the coins, one line a currency; the kobold's shortsword verbs
#[test]
fn a_kobold_thief_drops_its_coins() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let kob = spawn(&mut w, KOBOLD, ARENA, 1);
    mon(&mut w, kob).cash = Coins {
        silver: 7,
        copper: 20,
        ..Coins::default()
    };
    mon(&mut w, kob).hp = 1;
    w.rng = SeededRng::new(1);
    let mut out = Vec::new();
    for _ in 0..10 {
        round(&mut w, k, kob);
        out.extend(lines(&mut w, k));
        if !alive(&w, kob) {
            break;
        }
    }
    let at = out
        .iter()
        .position(|l| l == "7 silver drop to the ground.")
        .expect("silver line");
    assert_eq!(out[at + 1], "20 copper drop to the ground.");
    assert_eq!(
        out[at + 2],
        "The kobold thief falls to the ground with a shrill cry."
    );
    let room = w.room_state.get(&RoomKey::new(1, ARENA)).unwrap();
    assert_eq!((room.coins.silver, room.coins.copper), (7, 20));
    assert!(room.monsters.iter().all(|m| m.0 == 0));
}

/// The regex `template_regex()` builds for a WCCMSG text (patterns.json
/// `template_rules`): `%s` lazy, `%d` a number, the first letter either case, a trailing
/// run of `%s` an optional tail.
fn template_regex(t: &str) -> regex::Regex {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = t;
    let mut trailing = 0;
    while let Some(i) = rest.find('%') {
        parts.push(regex::escape(&rest[..i]));
        let conv = &rest[i + 1..i + 2];
        parts.push(match conv {
            "s" => "(.*?)".into(),
            "d" => r"-?\d+".into(),
            _ => "%".into(),
        });
        rest = &rest[i + 2..];
    }
    parts.push(regex::escape(rest));
    // the screen line is trimmed: so is the text before a dropped trailing %s
    while parts.len() >= 2
        && parts.last().is_some_and(|p| p.is_empty())
        && parts[parts.len() - 2] == "(.*?)"
    {
        parts.pop();
        parts.pop();
        trailing += 1;
        if let Some(last) = parts.last_mut() {
            *last = last.trim_end().to_owned();
        }
    }
    let mut rx = parts.concat();
    if trailing > 0 {
        rx.push_str(r"(?:\s.*)?");
    }
    let first = rx.chars().next().unwrap();
    if first.is_ascii_alphabetic() {
        rx = format!(
            "[{}{}]{}",
            first.to_ascii_uppercase(),
            first.to_ascii_lowercase(),
            &rx[1..]
        );
    }
    regex::Regex::new(&format!("^{rx}$")).unwrap()
}

// the monster lines of the giant rat, kobold thief and acid slime match the generated
// WCCMSG families (mon_hits_you/mon_hits_other/mon_miss_*/mon_dodge_*/mon_deflect_*/
// monster_death).
#[test]
fn monster_lines_match_their_wccmsg_families() {
    // (the kobold thief, AV 15 against DV 0, never misses: one kind is all it shows)
    let mut kinds = std::collections::BTreeSet::new();
    for (known, seed) in [(RAT, 3), (KOBOLD, 5), (SLIME, 9)] {
        let mut w = world();
        let k = kang(&mut w, "Kasimir", MessageLevel::High);
        let bob = kang(&mut w, "Bob", MessageLevel::High);
        let m = spawn(&mut w, known, ARENA, 1);
        w.rng = SeededRng::new(seed);
        let km = w.monsters.get(&known).unwrap().clone();
        let texts = |n: MessageNo| w.messages.get(&n).map(|m| m.msgs.clone()).unwrap();
        let hit = texts(km.attacks[0].hitmsg);
        let def = texts(km.attacks[0].defmsg);
        let miss = texts(km.attacks[0].missmsg);
        // the player's view: hit text 1, deflect text 1, dodge: deflect text 3, miss text 2
        let you = [&hit[0], &def[0], &def[2], &miss[1]].map(|t| template_regex(t));
        // the room's: hit text 2, deflect text 2, dodge: miss text 1, miss text 3
        let room = [&hit[1], &def[1], &miss[0], &miss[2]].map(|t| template_regex(t));
        for _ in 0..30 {
            plr(&mut w, k).hp = 48;
            monster_round(&mut w, m, k);
            for l in lines(&mut w, k) {
                let i = you.iter().position(|r| r.is_match(&l));
                assert!(i.is_some(), "{}: {l:?} matches no family", km.name);
                kinds.insert((km.name.clone(), i));
            }
            for l in lines(&mut w, bob) {
                assert!(
                    room.iter().any(|r| r.is_match(&l)),
                    "{}: room {l:?} matches no family",
                    km.name
                );
            }
        }
    }
    assert!(kinds.len() >= 4, "only {kinds:?}");
}

/// A giant rat that always hits for exactly `dmg`.
fn hard_rat(w: &mut World, dmg: i16) -> ActiveMonsterId {
    let k = w.monsters.get_mut(&RAT).unwrap();
    k.attacks[0].accuracy = 1000;
    k.attacks[0].min = dmg;
    k.attacks[0].max = dmg;
    spawn(w, RAT, ARENA, 1)
}

// knocked out, bell and all
#[test]
fn knocked_out_with_the_bell() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let rat = hard_rat(&mut w, 5);
    plr(&mut w, k).hp = 3;
    lines(&mut w, bob);
    monster_round(&mut w, rat, k);
    assert_eq!(hp(&w, k), -2);
    let mine = raw(&mut w, k);
    assert!(
        mine.contains("\x1b[0;37m\x1b[41mKasimir drops to the ground!\x07\x1b[40m\r\n"),
        "{mine:?}"
    );
    assert!(lines(&mut w, bob).contains(&"Kasimir drops to the ground!".to_owned()));
}

// between 0 and deathhp only unconscious; the monsters go on hitting
#[test]
fn the_unconscious_window_follows_deathhp() {
    for (deathhp, dies_at_minus_15) in [(-10, true), (-200, false)] {
        let mut w = world();
        w.config.death_hp = deathhp;
        let k = kang(&mut w, "Kasimir", MessageLevel::High);
        let rat = hard_rat(&mut w, 1);
        plr(&mut w, k).hp = -5;
        assert_eq!(check_kill_user(&mut w, k), KillOutcome::Unconscious);
        // a chaotic evil monster goes on hitting a player who is down
        monster_round(&mut w, rat, k);
        assert!(hp(&w, k) < -5);
        lines(&mut w, k);
        plr(&mut w, k).hp = -15;
        let out = check_kill_user(&mut w, k);
        assert_eq!(out.died(), dies_at_minus_15, "deathhp {deathhp}: {out:?}");
    }
}

// death, the pile, the lost life, the rebirth
#[test]
fn death_loses_a_life_and_wakes_in_the_reincarnation_room() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let waiting = enter(&mut w, "Wendy", REINC, MessageLevel::High);
    plr(&mut w, k).coins = Coins {
        gold: 3,
        ..Coins::default()
    };
    plr(&mut w, k).hp = -10;
    lines(&mut w, bob);
    lines(&mut w, waiting);
    assert_eq!(check_kill_user(&mut w, k), KillOutcome::Reborn);
    assert_eq!(
        lines(&mut w, k),
        [
            "You have been killed!",
            "But, due to a miracle, you have been saved.",
            "You have 8 lives left."
        ]
    );
    assert_eq!(lines(&mut w, bob), ["Kasimir is dead."]);
    assert_eq!(
        lines(&mut w, waiting),
        ["Kasimir appeared on the floor in the middle of the room."]
    );
    let c = w.session(k).unwrap().player().unwrap().clone();
    // hp = maxhp before the secondary-stat recalculation
    assert_eq!((c.lives, c.hp, c.room), (8, 48, RoomId(REINC)));
    assert_eq!(c.weapon, ItemNo(0));
    assert!(c.items.iter().all(|n| n.0 == 0));
    // the death pile: the staff and the gold lie where he died
    let arena = w.room_state.get(&RoomKey::new(1, ARENA)).unwrap();
    assert!(arena.items.iter().any(|i| i.item == QUARTERSTAFF));
    assert_eq!(arena.coins.gold, 3);
}

// the room's own death room
#[test]
fn a_room_with_its_own_death_room() {
    let mut w = world();
    w.rooms.get_mut(&RoomKey::new(1, ARENA)).unwrap().deathroom = RoomId(special_room(3));
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    plr(&mut w, k).hp = -50;
    check_kill_user(&mut w, k);
    assert_eq!(
        w.session(k).unwrap().player().unwrap().room,
        RoomId(special_room(3))
    );
}

fn special_room(i: usize) -> u32 {
    common::special_rooms()[i]
}

// a safe colliseum keeps everything
#[test]
fn a_colliseum_saves() {
    let mut w = world();
    w.rooms.get_mut(&RoomKey::new(1, ARENA)).unwrap().room_type = RoomType::Colliseum;
    w.display.arena_safe = true;
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    plr(&mut w, k).arenahp = 40;
    plr(&mut w, k).hp = 0;
    assert_eq!(check_kill_user(&mut w, k), KillOutcome::Arena);
    assert_eq!(
        lines(&mut w, k),
        [
            "You have been killed!",
            "But, because you were in a colliseum, you have been saved.",
            "You have 9 lives left."
        ]
    );
    let c = w.session(k).unwrap().player().unwrap();
    assert_eq!((c.lives, c.hp, c.weapon), (9, 40, QUARTERSTAFF));
}

// no lives left
#[test]
fn the_last_life_deletes_the_character() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    plr(&mut w, k).lives = 1;
    plr(&mut w, k).hp = -10;
    assert_eq!(check_kill_user(&mut w, k), KillOutcome::Deleted);
    let out = lines(&mut w, k);
    assert_eq!(out[0], "You have been killed!");
    assert!(
        out.contains(&"You have no lives remaining!".to_owned()),
        "{out:?}"
    );
    let s = w.session(k).unwrap();
    assert!(s.character.is_none());
    assert_eq!(s.state, silvermere_types::enums::SessionState::MainMenu);
}

// pvp_level_range_ok and the engagement
#[test]
fn pvp_engagement_and_the_level_range() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let eve = kang(&mut w, "Eve", MessageLevel::High);
    assert_eq!(
        run(&mut w, k, "a bob"),
        ["Such an attack would result in a very unbalanced combat round."]
    );
    for id in [k, bob] {
        plr(&mut w, id).level = 5;
    }
    lines(&mut w, bob);
    lines(&mut w, eve);
    // T-038: an attack on a neutral player is evil
    assert_eq!(
        run(&mut w, k, "a bob"),
        ["A dark cloud passes over you", "*Combat Engaged*"]
    );
    assert_eq!(lines(&mut w, bob), ["Kasimir moves to attack you!"]);
    assert_eq!(lines(&mut w, eve), ["Kasimir moves to attack Bob!"]);
    // a hidden victim, outside auto-combat
    plr(&mut w, bob).hidden = 1;
    attack_user_user(
        &mut w,
        k,
        bob,
        Move::Attack,
        Move::Attack,
        true,
        Mode::Command,
    );
    assert_eq!(lines(&mut w, k), ["You do not see Bob here!"]);
}

// the three points of view of a PvP round
#[test]
fn a_pvp_round_shows_three_points_of_view() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    let eve = kang(&mut w, "Eve", MessageLevel::High);
    for id in [k, bob] {
        plr(&mut w, id).level = 5;
        plr(&mut w, id).maxhp = 500;
        plr(&mut w, id).hp = 500;
    }
    let (mut mine, mut his, mut seen) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..10 {
        plr(&mut w, k).energy = 1000;
        attack_user_user(&mut w, k, bob, Move::Attack, Move::Attack, true, Mode::Auto);
        mine.extend(lines(&mut w, k));
        his.extend(lines(&mut w, bob));
        seen.extend(lines(&mut w, eve));
    }
    let you = regex::Regex::new(r"^You (smack|whap|beat) Bob for \d+ damage!$").unwrap();
    let victim = regex::Regex::new(r"^Kasimir (smacks|whaps|beats) you for \d+ damage!$").unwrap();
    let room = regex::Regex::new(r"^Kasimir (smacks|whaps|beats) Bob for \d+ damage!$").unwrap();
    assert!(mine.iter().any(|l| you.is_match(l)), "{mine:?}");
    assert!(his.iter().any(|l| victim.is_match(l)), "{his:?}");
    assert!(seen.iter().any(|l| room.is_match(l)), "{seen:?}");
}

#[test]
fn rob_refusals_and_the_bump() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let bob = kang(&mut w, "Bob", MessageLevel::High);
    assert_eq!(run(&mut w, k, "rob"), ["Syntax: ROB {user/monster}"]);
    assert_eq!(
        run(&mut w, k, "rob dragon"),
        ["You don't see that anywhere!"]
    );
    plr(&mut w, k).settings.insert(PlayerSettings::EVILWARN);
    assert_eq!(
        run(&mut w, k, "rob bob"),
        ["You have chosen a way of life which prevents this action."]
    );
    plr(&mut w, k).settings.remove(PlayerSettings::EVILWARN);
    lines(&mut w, k);
    combat::rob::rob_user(&mut w, k, k, None);
    assert_eq!(lines(&mut w, k), ["Why would you want to rob yourself?"]);
    plr(&mut w, k).delay = 0; // ROB's command delay
    assert_eq!(
        run(&mut w, k, "rob bob"),
        ["Such an action would result in a very unbalanced game."]
    );
    plr(&mut w, k).delay = 0;
    for id in [k, bob] {
        plr(&mut w, id).level = 5;
    }
    w.rooms
        .get_mut(&RoomKey::new(1, ARENA))
        .unwrap()
        .flags
        .insert(RoomFlags::PROTECTED);
    assert_eq!(
        run(&mut w, k, "rob bob"),
        ["You are overcome with a feeling of guilt and return your hands to your own pockets"]
    );
    w.rooms
        .get_mut(&RoomKey::new(1, ARENA))
        .unwrap()
        .flags
        .remove(RoomFlags::PROTECTED);
    plr(&mut w, k).delay = 0;
    // no thievery at all: a bump the victim feels
    lines(&mut w, bob);
    let mut bumped = false;
    for _ in 0..20 {
        plr(&mut w, k).delay = 0;
        let out = run(&mut w, k, "rob bob");
        // T-038: each robbery of a neutral player is evil
        if out
            == [
                "A dark cloud passes over you",
                "You bump Bob as you try to rob him.",
            ]
        {
            bumped = true;
            assert_eq!(
                lines(&mut w, bob),
                ["Kasimir bumps you as he tries to rob you!"]
            );
            break;
        }
    }
    assert!(bumped);
}

/// A capture line reduced as its README's mask hints say: damage, experience and coin
/// figures, the character's name and the random weapon verb (`smack|whap|beat`).
fn mask(l: &str) -> String {
    let names = regex::Regex::new(r"\b(Aldric|Corvin|Kasimir)\b").unwrap();
    let digits = regex::Regex::new(r"-?\d+").unwrap();
    let verbs = regex::Regex::new(r"^You (smack|whap|beat) ").unwrap();
    let l = names.replace_all(l, "@");
    let l = digits.replace_all(&l, "#");
    verbs.replace(&l, "You V ").into_owned()
}

/// The kind of a combat line, so only kinds the captures show are compared.
fn kind(l: &str) -> Option<&'static str> {
    let kinds: [(&str, &str); 7] = [
        ("hit", r"^You \S+ .+ for # damage!$"),
        ("glance", r"^Your .+ hits, but glances off its armour\.$"),
        ("miss", r"^You swing at .+!$"),
        ("monster", r"^The .+ (lunges|bites|whips)"),
        ("death", r"^The .+ falls to the ground"),
        ("exp", r"^You gain # experience\.$"),
        ("engaged", r"^\*Combat Engaged\*$"),
    ];
    kinds
        .iter()
        .find(|(_, rx)| regex::Regex::new(rx).unwrap().is_match(l))
        .map(|(k, _)| *k)
}

fn capture_lines() -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
    for text in [
        include_str!("../../../tests/captures/fight-arena/plain.txt"),
        include_str!("../../../tests/captures/death-rebirth/plain.txt"),
    ] {
        for l in text.lines() {
            let l = l.trim();
            let l = l.split("]:").last().unwrap_or(l).trim();
            set.insert(mask(l));
        }
    }
    set
}

/// A fight of `rounds` rounds each way against an active monster renamed `name`.
fn capture_fight(known: MonsterNo, name: &str, weapon: bool, seed: u64) -> Vec<String> {
    let mut w = world();
    w.rng = SeededRng::new(seed);
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    if !weapon {
        plr(&mut w, k).weapon = ItemNo(0);
    }
    let m = spawn(&mut w, known, ARENA, 1);
    mon(&mut w, m).name = name.into();
    let mut out = run(
        &mut w,
        k,
        &format!("a {}", name.rsplit(' ').next().unwrap()),
    );
    for _ in 0..30 {
        plr(&mut w, k).hp = 48;
        round(&mut w, k, m);
        out.extend(lines(&mut w, k));
        if !alive(&w, m) {
            break;
        }
        monster_round(&mut w, m, k);
        out.extend(lines(&mut w, k));
    }
    out
}

// The T-005 captures `fight-arena` and `death-rebirth` (the original under MBBSEmu): every
// line of ours of a kind they show is one of theirs once the RNG figures are masked.
#[test]
fn fight_lines_match_the_captures() {
    let cap = capture_lines();
    let mut checked = 0;
    for (known, name, weapon, seed) in [
        (RAT, "large giant rat", true, 11),
        (RAT, "fat giant rat", false, 12),
        (RAT, "thin giant rat", false, 13),
        (SLIME, "acid slime", false, 14),
    ] {
        for l in capture_fight(known, name, weapon, seed) {
            let m = mask(&l);
            if kind(&m).is_none() {
                continue;
            }
            assert!(
                cap.contains(&m),
                "{name}: {l:?} (masked {m:?}) is not in the captures"
            );
            checked += 1;
        }
    }
    assert!(checked >= 30, "only {checked} lines checked");
}

// no experience to share, or no victim, gives nothing
#[test]
fn distribute_experience_returns_early_without_exp_or_victim() {
    use silvermere_engine::combat::kill_monster::distribute_experience;
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let key = RoomKey::new(1, ARENA);
    lines(&mut w, k);
    let before = plr(&mut w, k).exp;
    distribute_experience(&mut w, Some(k), 0, None, Some(ActiveMonsterId(1)), key);
    distribute_experience(&mut w, Some(k), 9, None, None, key);
    assert!(lines(&mut w, k).is_empty());
    assert_eq!(plr(&mut w, k).exp, before);
    distribute_experience(&mut w, Some(k), 9, None, Some(ActiveMonsterId(1)), key);
    // the killer stops fighting
    assert_eq!(lines(&mut w, k), ["You gain 9 experience.", "*Combat Off*"]);
}

// an item's death text block runs as a whole after rebirth
#[test]
fn the_death_text_block_runs_after_rebirth() {
    use silvermere_types::ids::TextBlockNo;
    use silvermere_types::records::TextBlock;
    let mut w = world();
    w.items.get_mut(&QUARTERSTAFF).unwrap().abilities[0] = (Ability::DeathText, 950);
    w.messages.insert(
        MessageNo(950),
        msg(950, "A chill passes through you.", "", ""),
    );
    w.text_blocks.insert(
        (0, TextBlockNo(950)),
        TextBlock {
            part: 0,
            number: TextBlockNo(950),
            link: TextBlockNo(0),
            text: "message 950\n".into(),
        },
    );
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    plr(&mut w, k).hp = -10;
    assert_eq!(check_kill_user(&mut w, k), KillOutcome::Reborn);
    let out = lines(&mut w, k);
    assert_eq!(
        out.last().map(String::as_str),
        Some("A chill passes through you."),
        "{out:?}"
    );
}

// one fewer monster generated by the room, once per kill
#[test]
fn a_kill_counts_down_the_generating_room_once() {
    let mut w = world();
    let k = kang(&mut w, "Kasimir", MessageLevel::High);
    let rat = spawn(&mut w, RAT, ARENA, 1);
    let key = RoomKey::new(1, ARENA);
    w.room_state.get_mut(&key).unwrap().nummonsters = 2;
    w.minute = 600;
    mon(&mut w, rat).hp = 0;
    assert!(combat::check_kill_monster(&mut w, rat, Some(k)));
    let rs = w.room_state.get(&key).unwrap();
    assert_eq!((rs.nummonsters, rs.lastkill), (1, 600));
    assert!(
        !combat::check_kill_monster(&mut w, rat, Some(k)),
        "already gone"
    );
    assert_eq!(w.room_state.get(&key).unwrap().nummonsters, 1);
}
