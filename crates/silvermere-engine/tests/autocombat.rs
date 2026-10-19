//! Auto-combat (T-034) through the engine on the `TestClock`: the energy round every 5 s
//! (`background_energy`), the ring (`do_autocombat`),
//! the restart of a re-typed attack (`kill_autocombat` + `*Combat Off*`, then
//! `*Combat Engaged*`), the endings, the backstab rules of
//! `restart_autocombat`, the experience split (`distribute_experience`)
//! and the monsters' free swing at a player leaving (`give_monsters_a_free_attack`).
//!
//! The records are the stock ones `combat_flow.rs` uses (the vendored stock world
//! fixtures): class 2 Witchunter, race 11
//! Kang, item 100 quarterstaff, known monster 1 giant rat with WCCMSG 27, 8237, 8289,
//! 8294 and 8307. The monsters' own round (`do_monster_combat`) is T-036's; its tests are
//! in `monster_combat.rs`.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::combat::autocombat::{self, background_energy};
use silvermere_engine::combat::{AutoTarget, kill_monster::distribute_experience};
use silvermere_engine::{
    AccountRef, Engine, EngineInput, EngineOutput, SeededRng, SessionId, World,
};
use silvermere_rules::combat::Move;
use silvermere_text::strip_ansi;
use silvermere_types::enums::{
    Ability, Alignment, GroupRole, ItemType, MessageLevel, MonsterAttackType, MonsterType,
    WeaponType, WornLocation,
};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, PlayerStatus};
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId,
    RoomKey, SpellNo,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins, Message};

const ARENA: u32 = 2150;
const ELSEWHERE: u32 = 2151;
const KANG: RaceNo = RaceNo(11);
const WITCHUNTER: ClassNo = ClassNo(2);
const QUARTERSTAFF: ItemNo = ItemNo(100);
const MAUL: ItemNo = ItemNo(101);
const HELM: ItemNo = ItemNo(102);
const CLUB: ItemNo = ItemNo(103);
const RAT: MonsterNo = MonsterNo(1);

/// Blow cost of the test Kang (level 1, combat 7, agility 30, unencumbered):
/// `speed*1000/1560`, then times 75/100 for no encumbrance.
/// 1259 gives 807, then 605: the weapon whose blows alternate, one blow then two.
const SPEED_605: i16 = 1259;

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

fn world() -> World {
    let mut s = tiny_snapshot();
    for room in [ARENA, ELSEWHERE] {
        let mut r = silvermere_engine::world::blank::clear_room(RoomKey::new(1, room));
        r.name = "Newhaven Arena".into();
        s.rooms.push(r);
    }
    s.messages = vec![
        msg(
            27,
            "The %s bites you for %d damage!",
            "The %s bites %s for %s damage!",
            "The giant rat falls to the ground with a tortured squeak.",
        ),
        msg(
            8237,
            "smack|whap|beat",
            "smacks|whaps|beats",
            "smacks|whaps|beats",
        ),
        msg(8289, "swipe at", "swipes at", "swipes at"),
        msg(
            8294,
            "The %s %slunges at %s, %sbut %s dodges out of the way!",
            "The %s %slunges at %syou!",
            "The %s %slunges at %s! %s",
        ),
        msg(
            8307,
            "The %s %slunges at you, but your armour deflects the blow!",
            "The %s %slunges at %s, but %s armour deflects the blow!",
            "The %s %slunges at %syou, but you dodge out of the way!",
        ),
    ];
    let mut staff = silvermere_engine::world::blank::clear_item(QUARTERSTAFF);
    staff.name = "quarterstaff".into();
    staff.item_type = ItemType::Weapon;
    staff.weight = 100;
    staff.mindmg = 3;
    staff.maxdmg = 4;
    staff.speed = SPEED_605;
    staff.strreq = 30;
    staff.hitmsg = MessageNo(8237);
    staff.missmsg = MessageNo(8289);
    s.items.push(staff.clone());
    let mut maul = staff;
    maul.number = MAUL;
    maul.name = "maul".into();
    maul.wpntype = WeaponType::TwoHandBlunt;
    s.items.push(maul);
    let mut club = silvermere_engine::world::blank::clear_item(CLUB);
    club.name = "club".into();
    club.item_type = ItemType::Weapon;
    club.weight = 100;
    club.mindmg = 3;
    club.maxdmg = 4;
    club.speed = SPEED_605;
    club.strreq = 30;
    club.hitmsg = MessageNo(8237);
    club.missmsg = MessageNo(8289);
    s.items.push(club);
    let mut helm = silvermere_engine::world::blank::clear_item(HELM);
    helm.name = "helm".into();
    helm.item_type = ItemType::Armour;
    helm.worn = WornLocation::Head;
    helm.weight = 40;
    s.items.push(helm);
    let mut k = silvermere_engine::world::blank::clear_known_monster(RAT);
    k.name = "giant rat".into();
    k.level = 1;
    k.dr = 1;
    k.exp = 15;
    k.maxhp = 500;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.montype = MonsterType::Lair;
    k.followpct = 20;
    k.deathmsg = MessageNo(27);
    k.attacks[0] = silvermere_types::records::MonsterAttack {
        kind: MonsterAttackType::Normal,
        accuracy: 10,
        pct: 100,
        min: 2,
        max: 10,
        hitmsg: MessageNo(27),
        defmsg: MessageNo(8307),
        missmsg: MessageNo(8294),
        energy: 1000,
        hitspell: Default::default(),
    };
    s.monsters.push(k);
    let mut witchunter = silvermere_engine::world::blank::clear_class(WITCHUNTER);
    witchunter.name = "Witchunter".into();
    witchunter.exp = 30;
    witchunter.combat = 7;
    witchunter.abilities[0] = (Ability::Bash, 0);
    witchunter.abilities[1] = (Ability::Meditate, 0);
    s.classes.push(witchunter);
    let mut kang = silvermere_engine::world::blank::clear_race(KANG);
    kang.name = "Kang".into();
    kang.exp = 35;
    s.races.push(kang);
    let mut w = World::load(test_config(), 8, s).expect("world loads");
    w.rng = SeededRng::new(7);
    w
}

/// The engine on a `TestClock`, its writes gathered per session.
struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    text: std::collections::HashMap<SessionId, String>,
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
            text: Default::default(),
        }
    }

    fn w(&mut self) -> &mut World {
        &mut self.engine.world
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            if let EngineOutput::Write { id, bytes } = o {
                self.text
                    .entry(id)
                    .or_default()
                    .push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }

    /// A level-1 Kang Witchunter in `room` with the quarterstaff wielded.
    fn kang(&mut self, name: &str, room: u32) -> SessionId {
        let w = self.w();
        let id = enter(w, name, room, MessageLevel::High);
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
        // CANCAST, as every energy round leaves it (no 10% after-casting surcharge)
        c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
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

    /// An active giant rat in `room`, numbered `n`.
    fn rat(&mut self, room: u32, n: u16) -> ActiveMonsterId {
        let w = self.w();
        let k = w.monsters.get(&RAT).unwrap().clone();
        let number = ActiveMonsterId(i32::from(n));
        w.active_monsters.insert(ActiveMonster {
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
        });
        let rs = w.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
        *rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap() = number;
        number
    }

    fn mon_mut(&mut self, n: ActiveMonsterId) -> &mut ActiveMonster {
        self.w()
            .active_monsters
            .values_mut()
            .find(|m| m.number == n)
            .unwrap()
    }

    fn plr(&mut self, id: SessionId) -> &mut silvermere_types::records::Character {
        self.w()
            .session_mut(id)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
    }

    fn line(&mut self, id: SessionId, text: &str) {
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
    }

    fn ticks(&mut self, n: u32) {
        self.clock.advance_ticks(n, &mut self.engine);
        self.pump();
    }

    /// One fast tick, then the screen lines `id` got.
    fn tick(&mut self, id: SessionId) -> Vec<String> {
        self.ticks(1);
        self.lines(id)
    }

    /// The screen lines written to `id` since the last call: ANSI off, prompts and
    /// empty lines dropped.
    fn lines(&mut self, id: SessionId) -> Vec<String> {
        self.pump();
        let mut raw = self.text.remove(&id).unwrap_or_default();
        // what a direct call left in the batch, not flushed yet
        let w = self.w();
        let prompt = silvermere_engine::tell::prompt_bytes(w, id);
        let palette = silvermere_engine::tell::palette_of(w, id);
        if let Some(s) = w.session_mut(id) {
            let b = s.output.render(palette, &prompt);
            s.output.clear();
            raw.push_str(&String::from_utf8_lossy(&b));
        }
        let text = raw.replace("\x1b[79D\x1b[K", "\r\n");
        let plain = String::from_utf8_lossy(&strip_ansi(text.as_bytes())).into_owned();
        plain
            .split(['\r', '\n'])
            .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
            .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
            .collect()
    }

    /// Everything written to `id` since the last call, byte for byte (ANSI included).
    fn raw(&mut self, id: SessionId) -> String {
        self.pump();
        self.text.remove(&id).unwrap_or_default()
    }

    fn inside(&mut self, id: SessionId) -> bool {
        autocombat::is_inside_autocombat(self.w(), id)
    }
}

/// The player's blows in a batch of lines (each names the rat).
fn blows(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| l.starts_with("You") && l.contains("giant rat"))
        .count()
}

/// Ticks until the next energy round (the scheduler's 5 s), returning the lines of
/// each tick.
fn to_next_round(h: &mut Harness, id: SessionId) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    loop {
        let before = h.w().scheduler.tick();
        let l = h.tick(id);
        out.push(l);
        if (before + 1).is_multiple_of(5) {
            return out;
        }
    }
}

// One round per energy tick, nothing in between, the stored move (BASH) every round.
#[test]
fn one_round_per_energy_tick_with_the_stored_move() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    h.rat(ARENA, 1);
    h.line(k, "bash rat");
    assert!(h.lines(k).contains(&"*Combat Engaged*".to_owned()));
    assert!(h.inside(k));
    assert_eq!(h.plr(k).energy, 0);
    let mut counts = Vec::new();
    for _ in 0..4 {
        let ticks = to_next_round(&mut h, k);
        let (quiet, round) = ticks.split_at(ticks.len() - 1);
        assert!(quiet.iter().all(|l| blows(l) == 0), "{quiet:?}");
        counts.push(blows(&round[0]));
        assert_eq!(autocombat::get_my_attack_type(h.w(), k), Move::Bash);
        assert!(h.inside(k));
    }
    assert!(counts.iter().all(|&c| c >= 1), "{counts:?}");
}

// Inside auto-combat the remainder carries over, so a 605-cost
// weapon alternates one and two blows. Re-typing the attack mid-fight gives
// `*Combat Off*` then `*Combat Engaged*` and an empty pool: the round that was due two
// blows makes one.
#[test]
fn retyping_the_attack_restarts_and_loses_the_carried_energy() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    h.rat(ARENA, 1);
    h.line(k, "a rat");
    h.lines(k);
    let mut counts = Vec::new();
    for _ in 0..4 {
        let ticks = to_next_round(&mut h, k);
        counts.push(blows(ticks.last().unwrap()));
    }
    assert_eq!(counts, [1, 2, 1, 2]);
    // 370 carried: the next round would have 1370, two blows
    assert_eq!(h.plr(k).energy, 370);
    h.line(k, "a rat");
    let l = h.lines(k);
    let off = l.iter().position(|x| x == "*Combat Off*");
    let on = l.iter().position(|x| x == "*Combat Engaged*");
    assert!(off.is_some() && on.is_some() && off < on, "{l:?}");
    assert_eq!(h.plr(k).energy, 0);
    let ticks = to_next_round(&mut h, k);
    assert_eq!(blows(ticks.last().unwrap()), 1, "the restart costs a blow");
}

// The kill hands out the experience, then `*Combat Off*`.
#[test]
fn the_kill_ends_auto_combat() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let rat = h.rat(ARENA, 1);
    h.mon_mut(rat).hp = 1;
    h.line(k, "a rat");
    h.lines(k);
    let ticks = to_next_round(&mut h, k);
    let l = ticks.last().unwrap();
    let gain = l.iter().position(|x| x.starts_with("You gain "));
    let off = l.iter().position(|x| x == "*Combat Off*");
    assert!(gain.is_some() && off == gain.map(|g| g + 1), "{l:?}");
    assert_eq!(l[gain.unwrap()], "You gain 15 experience.");
    assert!(!h.inside(k));
    // nothing more comes
    let ticks = to_next_round(&mut h, k);
    assert!(ticks.iter().all(|l| l.is_empty()), "{ticks:?}");
}

// Two players on one rat share 15 as 7 and 7; both stop.
#[test]
fn two_attackers_split_the_experience() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let b = h.kang("Bob", ARENA);
    let rat = h.rat(ARENA, 1);
    h.line(k, "a rat");
    h.line(b, "a rat");
    h.lines(k);
    h.lines(b);
    to_next_round(&mut h, k);
    h.lines(b);
    h.mon_mut(rat).hp = 1;
    let mut lk = Vec::new();
    let mut lb = Vec::new();
    for _ in 0..5 {
        h.ticks(1);
        lk.extend(h.lines(k));
        lb.extend(h.lines(b));
    }
    assert!(lk.contains(&"You gain 7 experience.".to_owned()), "{lk:?}");
    assert!(lb.contains(&"You gain 7 experience.".to_owned()), "{lb:?}");
    assert!(lk.contains(&"*Combat Off*".to_owned()));
    assert!(lb.contains(&"*Combat Off*".to_owned()));
    assert!(!h.inside(k) && !h.inside(b));
}

// A no-target caster in the room gets a share and keeps
// going; one in another room gets nothing; the minimum is 1.
#[test]
fn a_no_target_caster_in_the_room_gets_a_share() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let c = h.kang("Cass", ARENA);
    let far = h.kang("Faramond", ELSEWHERE);
    let rat = h.rat(ARENA, 1);
    autocombat::engage(h.w(), c, None, Move::Attack, Some(SpellNo(1)));
    autocombat::engage(h.w(), far, None, Move::Attack, Some(SpellNo(1)));
    let key = RoomKey::new(1, ARENA);
    let shares = autocombat_split(&mut h, k, 15, rat, key);
    assert_eq!(shares, [(k, 7), (c, 7)]);
    assert_eq!(h.lines(k), ["You gain 7 experience.", "*Combat Off*"]);
    assert_eq!(h.lines(c), ["You gain 7 experience."]);
    assert!(h.lines(far).is_empty());
    assert!(h.inside(c), "the caster goes on");
    // killer + fighter + caster: 2 among 3 is 0, so 1 each
    let d = h.kang("Dorn", ARENA);
    autocombat::engage(h.w(), d, Some(AutoTarget::Monster(rat)), Move::Attack, None);
    let shares = autocombat_split(&mut h, k, 2, rat, key);
    assert_eq!(shares, [(k, 1), (c, 1), (d, 1)]);
}

fn autocombat_split(
    h: &mut Harness,
    killer: SessionId,
    exp: u32,
    rat: ActiveMonsterId,
    room: RoomKey,
) -> Vec<(SessionId, u32)> {
    for id in h.w().session_ids() {
        h.lines(id);
    }
    silvermere_engine::combat::experience_split::distribute_experience(
        h.w(),
        Some(killer),
        exp,
        AutoTarget::Monster(rat),
        room,
    )
}

// Through T-033's entry point: no victim, nothing.
#[test]
fn no_victim_no_share() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let key = RoomKey::new(1, ARENA);
    distribute_experience(h.w(), Some(k), 9, None, None, key);
    distribute_experience(h.w(), Some(k), 9, Some(k), Some(ActiveMonsterId(1)), key);
    assert_eq!(h.plr(k).exp, 0);
}

// The monster leaves the room; the next energy round ends the fight.
#[test]
fn the_target_walks_away() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let rat = h.rat(ARENA, 1);
    h.line(k, "a rat");
    h.lines(k);
    to_next_round(&mut h, k);
    h.mon_mut(rat).room = RoomId(ELSEWHERE);
    let ticks = to_next_round(&mut h, k);
    let l = ticks.last().unwrap();
    assert_eq!(l.as_slice(), ["*Combat Off*"]);
    assert!(!h.inside(k));
    // ATTACK alone: the target is not here
    h.line(k, "a");
    assert_eq!(h.lines(k), ["You don't see your target here!"]);
}

// A player knocked out stops with `*Combat Off*`.
#[test]
fn a_knocked_out_player_stops() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    h.rat(ARENA, 1);
    h.line(k, "a rat");
    h.lines(k);
    to_next_round(&mut h, k);
    h.plr(k).hp = 0;
    let ticks = to_next_round(&mut h, k);
    // the Chaotic Evil rat may swing at the downed player first (T-036)
    assert_eq!(
        ticks.last().unwrap().last().map(String::as_str),
        Some("*Combat Off*")
    );
    assert!(!h.inside(k));
}

// BREAK tells the room.
#[test]
fn break_tells_the_room() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let b = h.kang("Bob", ARENA);
    h.rat(ARENA, 1);
    h.line(k, "a rat");
    h.lines(k);
    h.lines(b);
    h.line(k, "break");
    assert_eq!(h.lines(k), ["*Combat Off*"]);
    assert_eq!(h.lines(b), ["Kasimir breaks off combat."]);
    assert!(!h.inside(k));
    h.line(k, "break");
    assert!(h.lines(k).is_empty());
}

// A backstab entry restarts as a backstab only from hiding and not
// sneaking (no "moves to attack" for the room); otherwise a plain attack.
#[test]
fn backstab_restart_from_hiding_or_not() {
    for hidden in [true, false] {
        let mut h = Harness::new();
        let k = h.kang("Kasimir", ARENA);
        let b = h.kang("Bob", ARENA);
        let rat = h.rat(ARENA, 1);
        autocombat::engage(
            h.w(),
            k,
            Some(AutoTarget::Monster(rat)),
            Move::Backstab,
            None,
        );
        autocombat::kill_autocombat(h.w(), k);
        h.plr(k).hidden = u8::from(hidden);
        assert!(autocombat::restart_autocombat(h.w(), k, false));
        assert!(h.lines(k).contains(&"*Combat Engaged*".to_owned()));
        let room = h.lines(b);
        let announced = room.contains(&"Kasimir moves to attack giant rat.".to_owned());
        assert_eq!(announced, !hidden, "hidden {hidden}: {room:?}");
        assert_eq!(
            autocombat::get_my_attack_type(h.w(), k),
            if hidden { Move::Backstab } else { Move::Attack }
        );
    }
}

// Never with a two-handed weapon; the entry falls back for good.
#[test]
fn two_handed_weapons_cannot_backstab() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let rat = h.rat(ARENA, 1);
    h.plr(k).items[0] = MAUL;
    h.plr(k).weapon = MAUL;
    h.plr(k).hidden = 1;
    autocombat::engage(
        h.w(),
        k,
        Some(AutoTarget::Monster(rat)),
        Move::Backstab,
        None,
    );
    autocombat::kill_autocombat(h.w(), k);
    assert!(autocombat::restart_autocombat(h.w(), k, false));
    let l = h.lines(k);
    assert_eq!(l[0], "You cannot backstab with this weapon.");
    assert!(l.contains(&"*Combat Engaged*".to_owned()));
    assert_eq!(autocombat::get_my_attack_type(h.w(), k), Move::Attack);
}

// Backstab entries strike before everyone else in the round, even
// one queued behind a plain attacker.
#[test]
fn backstabs_strike_first() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let s = h.kang("Shade", ARENA);
    let o = h.kang("Olaf", ARENA);
    let rat = h.rat(ARENA, 1);
    h.line(k, "a rat");
    autocombat::engage(
        h.w(),
        s,
        Some(AutoTarget::Monster(rat)),
        Move::Backstab,
        None,
    );
    h.plr(s).energy = 0;
    assert_eq!(h.w().autocombat.queue(), [k, s]);
    h.lines(o);
    let ticks = to_next_round(&mut h, o);
    let l = ticks.last().unwrap();
    let first_shade = l.iter().position(|x| x.starts_with("Shade"));
    let first_kasimir = l.iter().position(|x| x.starts_with("Kasimir"));
    assert!(
        first_shade.is_some() && first_kasimir.is_some() && first_shade < first_kasimir,
        "{l:?}"
    );
    // the backstab was the first blow only
    assert_eq!(autocombat::get_my_attack_type(h.w(), s), Move::Attack);
}

// The players go first in 60% of rounds (a roll of 0..=100 below 60).
#[test]
fn players_first_sixty_forty() {
    let mut w = world();
    let mut first = 0;
    for _ in 0..1000 {
        if background_energy(&mut w) {
            first += 1;
        }
    }
    assert!((560..=640).contains(&first), "{first} of 1000");
}

// attack_monster_user: each bite takes the hit points
// and then sends its line with the prompt in one send to the player, so in one round
// each prompt shows the hit points after its own line (T-099; the flush used to print the
// end-of-round figure after every line).
#[test]
fn each_combat_line_carries_the_hit_points_of_its_moment() {
    let mut h = Harness::new();
    // a rat that lands every bite, two or three a round
    let rat = h.w().monsters.get_mut(&RAT).unwrap();
    rat.attacks[0].accuracy = 250;
    rat.attacks[0].energy = 400;
    let k = h.kang("Kasimir", ARENA);
    h.rat(ARENA, 1);
    h.line(k, "attack rat");
    to_next_round_raw(&mut h);
    h.pump();
    h.text.clear();
    // the second round, one write: Kasimir's two blows, then three bites
    to_next_round_raw(&mut h);
    h.pump();
    const CL: &str = "\x1b[79D\x1b[K";
    let line = |colour: &str, text: &str| format!("{CL}{colour}{text}\r\n");
    // the figure changes colour as it falls (get_hp_colour)
    let prompt = |hp: &str| format!("{CL}\x1b[0;37m[HP={hp}\x1b[0;37m]:");
    let blow = "\x1b[1;31m";
    let bite = "\x1b[1m\x1b[31m";
    let want = [
        line(blow, "You beat giant rat for 2 damage!"),
        prompt("39"),
        line(blow, "You smack giant rat for 3 damage!"),
        prompt("39"),
        line(bite, "The giant rat bites you for 9 damage!"),
        prompt("30"),
        line(bite, "The giant rat bites you for 9 damage!"),
        prompt("\x1b[1;37m21"),
        line(bite, "The giant rat bites you for 4 damage!"),
        prompt("\x1b[1;37m17"),
    ]
    .concat();
    assert_eq!(h.text.remove(&k).unwrap_or_default(), want);
}

// May cast again; attacked last round -> spotted.
#[test]
fn end_of_round_flags() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    h.plr(k).settings.remove(PlayerSettings::CANCAST);
    h.plr(k)
        .status2
        .insert(silvermere_types::flags::PlayerStatus2::ATTACKED);
    background_energy(h.w());
    let p = h.plr(k).clone();
    assert!(p.settings.contains(PlayerSettings::CANCAST));
    assert!(p.status.contains(PlayerStatus::SPOTTED));
    background_energy(h.w());
    assert!(!h.plr(k).status.contains(PlayerStatus::SPOTTED));
}

// One roll for the room; the first monster whose follow chance is
// at least the roll strikes once; no other monster strikes.
#[test]
fn the_free_attack_on_leaving() {
    use silvermere_engine::movement::move_user::give_monsters_a_free_attack;
    for (pct, struck) in [(100, true), (0, false)] {
        let mut h = Harness::new();
        let k = h.kang("Kasimir", ARENA);
        let a = h.rat(ARENA, 1);
        let b = h.rat(ARENA, 2);
        h.mon_mut(a).followpct = pct;
        h.mon_mut(b).followpct = pct;
        h.lines(k);
        give_monsters_a_free_attack(h.w(), k);
        let l = h.lines(k);
        let bites = l.iter().filter(|x| x.contains("giant rat")).count();
        assert_eq!(bites > 0, struck, "followpct {pct}: {l:?}");
        assert_eq!(h.plr(k).attackers, i32::from(struck));
    }
}

/// The registry row `name` (tests/fixtures/patterns.json), anchored.
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

// The auto-combat notices as sent (ANSI kept), each line matched against its pattern row.
#[test]
fn the_notices_match_the_patterns_and_the_snapshot() {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let b = h.kang("Bob", ARENA);
    let rat = h.rat(ARENA, 1);
    let mut shown = Vec::new();
    let raw_of = |h: &mut Harness, who: SessionId| -> String {
        h.pump();
        let mut raw = h.text.remove(&who).unwrap_or_default();
        let w = h.w();
        let prompt = silvermere_engine::tell::prompt_bytes(w, who);
        let palette = silvermere_engine::tell::palette_of(w, who);
        if let Some(s) = w.session_mut(who) {
            raw.push_str(&String::from_utf8_lossy(&s.output.render(palette, &prompt)));
            s.output.clear();
        }
        raw
    };
    raw_of(&mut h, k);
    raw_of(&mut h, b);
    let mut check = |label: &str, raw: String, name: &str, line: &str| {
        let plain = String::from_utf8_lossy(&strip_ansi(
            raw.replace("\x1b[79D\x1b[K", "\r\n").as_bytes(),
        ))
        .into_owned();
        assert!(
            plain.split(['\r', '\n']).any(|l| l.trim() == line),
            "{label}: {plain:?}"
        );
        assert!(
            pattern(name).is_match(line),
            "{name} does not match {line:?}"
        );
        let entry = format!("{label}: {raw:?}");
        if !shown.contains(&entry) {
            shown.push(entry);
        }
    };
    h.line(k, "a rat");
    check(
        "engaged",
        raw_of(&mut h, k),
        "combat_engaged",
        "*Combat Engaged*",
    );
    raw_of(&mut h, b);
    h.line(k, "break");
    check("off", raw_of(&mut h, k), "combat_off", "*Combat Off*");
    check(
        "breaks off",
        raw_of(&mut h, b),
        "breaks_off",
        "Kasimir breaks off combat.",
    );
    h.mon_mut(rat).room = RoomId(ELSEWHERE);
    h.line(k, "a");
    check(
        "target gone",
        raw_of(&mut h, k),
        "target_missing",
        "You don't see your target here!",
    );
    h.mon_mut(rat).room = RoomId(ARENA);
    autocombat::engage(
        h.w(),
        k,
        Some(AutoTarget::Monster(rat)),
        Move::Backstab,
        None,
    );
    autocombat::kill_autocombat(h.w(), k);
    h.plr(k).weapon = MAUL;
    h.plr(k).items[0] = MAUL;
    autocombat::restart_autocombat(h.w(), k, false);
    check(
        "no backstab",
        raw_of(&mut h, k),
        "no_backstab",
        "You cannot backstab with this weapon.",
    );
    autocombat::kill_autocombat(h.w(), k);
    autocombat::engage(h.w(), k, Some(AutoTarget::Player(b)), Move::Attack, None);
    autocombat::kill_autocombat(h.w(), k);
    h.plr(b).room = RoomId(ELSEWHERE);
    autocombat::restart_autocombat(h.w(), k, false);
    check(
        "not here",
        raw_of(&mut h, k),
        "target_missing",
        "You do not see Bob here!",
    );
    h.plr(b).room = RoomId(ARENA);
    h.mon_mut(rat).hp = 1;
    h.plr(k).weapon = QUARTERSTAFF;
    h.plr(k).items[0] = QUARTERSTAFF;
    h.line(k, "a rat");
    raw_of(&mut h, k);
    to_next_round_raw(&mut h);
    let raw = raw_of(&mut h, k);
    check("kill", raw.clone(), "exp_gain", "You gain 15 experience.");
    check("kill", raw, "combat_off", "*Combat Off*");
    insta::assert_snapshot!("notices", shown.join("\n"));
}

/// Ticks to the next energy round without reading anyone's output.
fn to_next_round_raw(h: &mut Harness) {
    loop {
        let before = h.w().scheduler.tick();
        h.clock.advance_ticks(1, &mut h.engine);
        if (before + 1).is_multiple_of(5) {
            return;
        }
    }
}

/// `cmd` typed by Kasimir in auto-combat with Bob in the room: what each of them got,
/// byte for byte.
fn engaged(cmd: &str, worn_helm: bool) -> (String, String) {
    let mut h = Harness::new();
    let k = h.kang("Kasimir", ARENA);
    let b = h.kang("Bob", ARENA);
    h.rat(ARENA, 1);
    h.plr(k).items[1] = HELM;
    h.plr(k).itemuses[1] = -1;
    h.plr(k).items[2] = CLUB;
    h.plr(k).itemuses[2] = -1;
    if worn_helm {
        h.plr(k).worn[0] = HELM;
    }
    h.plr(k).mana = 0;
    h.plr(k).maxmana = 10;
    h.line(k, "a rat");
    assert!(h.inside(k));
    h.raw(k);
    h.raw(b);
    h.line(k, cmd);
    assert!(!h.inside(k), "{cmd} left the auto-combat running");
    (h.raw(k), h.raw(b))
}

// remove_armour: `You have removed` goes out with the break-off's flush, then `*Combat Off*`.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn remove_helm_breaks_off_autocombat_first() {
    let (me, room) = engaged("remove helm", true);
    assert_eq!(
        me,
        "\x1b[79D\x1b[KYou have removed helm.\r\n\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[K\x1b[0;33mKasimir removes helm!\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;33mKasimir breaks off combat.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}

// ready_weapon: the break-off comes before the old weapon is put away.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn wield_another_weapon_breaks_off_autocombat_first() {
    let (me, room) = engaged("wield club", false);
    assert_eq!(
        me,
        "\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:\x1b[79D\x1b[KYou are now holding club.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[K\x1b[0;33mKasimir breaks off combat.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;33mKasimir removes quarterstaff.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;33mKasimir wields club!\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}

// ready_weapon, UNREADY: `You now have no weapon readied.` is flushed first, then the break-off.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn unready_breaks_off_autocombat_first() {
    let (me, room) = engaged("unready", false);
    assert_eq!(
        me,
        "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[0;33mYou now have no weapon readied.\r\n\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[K\x1b[0;33mKasimir removes quarterstaff.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;33mKasimir breaks off combat.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}

// wear_armour: the break-off comes before the item goes on.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn wear_armour_breaks_off_autocombat_first() {
    let (me, room) = engaged("wear helm", false);
    assert_eq!(
        me,
        "\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:\x1b[79D\x1b[KYou are now wearing helm.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[K\x1b[0;33mKasimir breaks off combat.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[0;33mKasimir wears helm!\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}

// CMD_REST: the break-off and its prompt come before `You are now resting.`.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn rest_breaks_off_autocombat_first() {
    let (me, room) = engaged("rest", false);
    assert_eq!(
        me,
        "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:You are now resting.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]: (Resting) "
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[KKasimir stops to rest.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}

// CMD_MEDITATE: the break-off and its prompt come before `You are now meditating.`.
// Bob hears `breaks off combat` at the equipment sites (toroom 1), not for REST/MEDITATE.
#[test]
fn meditate_breaks_off_autocombat_first() {
    let (me, room) = engaged("meditate", false);
    assert_eq!(
        me,
        "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]:You are now meditating.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m/MA=0\x1b[0;37m]: (Meditating) "
    );
    assert_eq!(
        room,
        "\x1b[79D\x1b[KKasimir kneels to meditate.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
}
