//! The M2 ("fight") fixture world and an in-process driver for it (T-040).
//!
//! The world is Newhaven Arena 1:2150 and the Narrow Road above it (1:2146), the four
//! special rooms of the default config, and the stock records the fights read: known
//! monsters 1 giant rat, 5 acid slime and 7 kobold thief, items 100 quarterstaff and 67
//! shortsword, class 2 Witchunter, race 11 Kang, and their WCCMSG records. The values are
//! the stock export's (`crates/silvermere-import/tests/fixtures/world/*.jsonl.gz`,
//! MANIFEST of 2026-10-14).
//!
//! [`M2`] runs the real `Engine` on a [`TestClock`]: lines go in as `EngineInput::Line`,
//! time as one `EngineInput::Tick` per simulated second, and every session's bytes are
//! kept so a test can read them as capture-style lines (`raw_to_lines`, the same text
//! preparation the T-005 captures go through). No sleeps.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use silvermere_engine::monsters::generate::{Request, generate_monster};
use silvermere_engine::world::blank;
use silvermere_engine::{
    AccountRef, Caps, Engine, EngineInput, EngineOutput, PlayerSession, SeededRng, SessionId,
    World, WorldSnapshot, clear_player,
};
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{
    Ability, Alignment, ExitType, GroupRole, InGame, ItemType, MessageLevel, MonsterAttackType,
    MonsterType, RoomType, SessionState, StatlineMode, WeaponType,
};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::TextBlockNo;
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId,
    RoomKey, SpellNo,
};
use silvermere_types::records::{
    Character, Exit, KnownMonster, Message, MonsterAttack, Room, TextBlock,
};

use crate::captures::raw_to_lines;
use crate::clock::TestClock;

/// Newhaven Arena (1:2150).
pub const ARENA: u32 = 2150;
/// Newhaven, Narrow Road (1:2146), up the stairs from the arena.
pub const ROAD: u32 = 2146;
/// The arena's exit up and the road's exit down (exit slots 8 and 9: N S E W NE NW SE SW U D).
pub const UP: usize = 8;
pub const DOWN: usize = 9;
/// The stock arena's monster group (`mongroup` 6) and level range 1-3.
pub const ARENA_GROUP: i16 = 6;
/// A monster group no record has: rooms of it never spawn, so a scripted fight has only
/// the monsters the test places.
pub const NO_GROUP: i16 = 99;

pub const KANG: RaceNo = RaceNo(11);
pub const WITCHUNTER: ClassNo = ClassNo(2);
pub const QUARTERSTAFF: ItemNo = ItemNo(100);
pub const SHORTSWORD: ItemNo = ItemNo(67);
pub const RAT: i32 = 1;
pub const SLIME: i32 = 5;
pub const KOBOLD: i32 = 7;

/// `GameConfig::default().reinc_room`, where the dead wake.
pub fn reinc_room() -> u32 {
    GameConfig::default().reinc_room
}

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

/// The stock WCCMSG records of the three monsters and the two weapons.
pub fn messages() -> Vec<Message> {
    vec![
        msg(
            26,
            "A %s creeps into the room from %s.",
            "The %s creeps out of the room to %s.",
            "The giant rat creeps in after you!",
        ),
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
        msg(
            8228,
            "impale|skewer|slash",
            "impales|skewers|slashes",
            "impales|skewers|slashes",
        ),
        msg(8287, "lunge at", "lunges at", "lunges at"),
        msg(
            8237,
            "smack|whap|beat",
            "smacks|whaps|beats",
            "smacks|whaps|beats",
        ),
        msg(8289, "swipe at", "swipes at", "swipes at"),
    ]
}

/// One attack slot of a WCCKNMSR record.
// one argument per record field
#[allow(clippy::too_many_arguments)]
pub fn slot(
    kind: u16,
    accuracy: i16,
    pct: u8,
    min: i16,
    max: i16,
    msgs: (i32, i32, i32),
    energy: i16,
    hitspell: i16,
) -> MonsterAttack {
    MonsterAttack {
        kind: MonsterAttackType::from_code(kind),
        accuracy,
        pct,
        min,
        max,
        hitmsg: MessageNo(msgs.0),
        defmsg: MessageNo(msgs.1),
        missmsg: MessageNo(msgs.2),
        energy,
        hitspell: SpellNo(hitspell),
    }
}

/// A WCCKNMSR record with the fields the fights read; `grouprole` Solo.
// one argument per record field
#[allow(clippy::too_many_arguments)]
fn known(
    n: i32,
    name: &str,
    weapon: ItemNo,
    dr: i16,
    ac: i16,
    exp: i32,
    maxhp: i16,
    abilities: &[Ability],
    followpct: i16,
    arrivemsg: i32,
    deathmsg: i32,
    attacks: &[MonsterAttack],
) -> KnownMonster {
    let mut k = blank::clear_known_monster(MonsterNo(n));
    k.name = name.into();
    k.montype = MonsterType::Group1;
    k.level = 1;
    k.genindex = 1;
    k.weapon = weapon;
    k.dr = dr;
    k.ac = ac;
    k.exp = exp;
    k.maxhp = maxhp;
    k.energy = 1000;
    k.hpregen = 1;
    k.alignment = Alignment::ChaoticEvil;
    k.followpct = followpct;
    k.grouprole = GroupRole::Solo;
    k.arrivemsg = MessageNo(arrivemsg);
    k.deathmsg = MessageNo(deathmsg);
    for (i, a) in abilities.iter().enumerate() {
        k.abilities[i] = (*a, 0);
    }
    for (i, a) in attacks.iter().enumerate() {
        k.attacks[i] = *a;
    }
    k
}

/// The stock name-list numbers (`namelist`): only the rat's list is in the fixture.
fn with_namelist(mut k: KnownMonster, namelist: i32) -> KnownMonster {
    k.namelist = namelist;
    k
}

fn exit(target: u32) -> Exit {
    Exit {
        target: RoomId(target),
        kind: ExitType::Normal,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

/// A room of the fixture world.
pub fn room(n: u32, name: &str, kind: RoomType, mongroup: i16, maxmonsters: u8) -> Room {
    let mut r = blank::clear_room(RoomKey::new(1, n));
    r.name = name.into();
    r.room_type = kind;
    r.mongroup = mongroup;
    r.minlevel = 1;
    r.maxlevel = 3;
    r.maxmonsters = maxmonsters;
    r
}

/// The fixture world as a snapshot. `spawns` gives the arena the stock monster group
/// (Arena type, `maxmonsters` 2); without it no room ever spawns.
pub fn snapshot(spawns: bool) -> WorldSnapshot {
    let mut s = WorldSnapshot::default();
    let g = GameConfig::default();
    for (i, n) in [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room]
        .into_iter()
        .enumerate()
    {
        s.rooms.push(room(
            n,
            &format!("Room {}", i + 1),
            RoomType::Normal,
            NO_GROUP,
            0,
        ));
    }
    let group = if spawns { ARENA_GROUP } else { NO_GROUP };
    let mut arena = room(ARENA, "Newhaven, Arena", RoomType::Arena, group, 2);
    arena.exits[UP] = exit(ROAD);
    s.rooms.push(arena);
    let mut road = room(ROAD, "Newhaven, Narrow Road", RoomType::Normal, NO_GROUP, 0);
    road.exits[DOWN] = exit(ARENA);
    s.rooms.push(road);
    s.messages = messages();
    // the rat's name list (WCCTEXT 2001): here only `N:`, its own name, so the arrival
    // line puts the name first; the stock list adds an adjective
    s.text_blocks.push(TextBlock {
        part: 0,
        number: TextBlockNo(2001),
        link: TextBlockNo(0),
        text: "N:".into(),
    });
    let mut staff = blank::clear_item(QUARTERSTAFF);
    staff.name = "quarterstaff".into();
    staff.item_type = ItemType::Weapon;
    staff.weight = 100;
    staff.mindmg = 2;
    staff.maxdmg = 12;
    staff.speed = 1200;
    staff.strreq = 30;
    staff.ac = 10;
    staff.hitmsg = MessageNo(8237);
    staff.missmsg = MessageNo(8289);
    s.items.push(staff);
    let mut sword = blank::clear_item(SHORTSWORD);
    sword.name = "shortsword".into();
    sword.item_type = ItemType::Weapon;
    sword.wpntype = WeaponType::from_code(2);
    sword.mindmg = 2;
    sword.maxdmg = 9;
    sword.speed = 1000;
    sword.hitmsg = MessageNo(8228);
    sword.missmsg = MessageNo(8287);
    sword.abilities[0] = (Ability::BackstabAv, 5);
    s.items.push(sword);
    s.monsters = vec![
        // giant rat: atktypes [1,0,0,229,0], atkpcts [100,20,30,0,0]
        with_namelist(
            known(
                RAT,
                "giant rat",
                ItemNo(0),
                1,
                0,
                9,
                12,
                &[Ability::PoisonImmune, Ability::Animal],
                20,
                26,
                27,
                &[
                    slot(1, 10, 100, 2, 10, (27, 8307, 8294), 1000, 0),
                    slot(0, 70, 20, 4, 20, (27, 8307, 8294), 1000, 0),
                    slot(0, 1, 30, 50, 0, (1, 1, 1), 1000, 0),
                    slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
                ],
            ),
            2001,
        ),
        known(
            SLIME,
            "acid slime",
            ItemNo(0),
            0,
            0,
            16,
            25,
            &[Ability::NonLiving],
            15,
            0,
            35,
            &[
                slot(1, 20, 100, 2, 9, (35, 8309, 8296), 1000, 126),
                slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
            ],
        ),
        known(
            KOBOLD,
            "kobold thief",
            SHORTSWORD,
            1,
            10,
            13,
            20,
            &[],
            20,
            0,
            41,
            &[
                slot(1, 15, 100, 1, 8, (41, 8297, 8310), 666, 0),
                slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
            ],
        ),
    ];
    let mut witchunter = blank::clear_class(WITCHUNTER);
    witchunter.name = "Witchunter".into();
    witchunter.exp = 30;
    witchunter.combat = 7;
    witchunter.basehp = 6;
    witchunter.hplevel = 4;
    witchunter.abilities[0] = (Ability::Bash, 0);
    s.classes.push(witchunter);
    let mut kang = blank::clear_race(KANG);
    kang.name = "Kang".into();
    kang.exp = 35;
    s.races.push(kang);
    s
}

/// The module's 60 s save period, as the engine tests pin it.
pub fn config() -> GameConfig {
    GameConfig {
        sec_oth: 60,
        ..GameConfig::default()
    }
}

/// The real engine over the fixture world on a [`TestClock`], every session's bytes kept.
pub struct M2 {
    pub engine: Engine<TestClock, SeededRng>,
    pub clock: TestClock,
    rx: Receiver<EngineOutput>,
    bytes: HashMap<SessionId, Vec<u8>>,
}

impl M2 {
    /// The fixture world (see [`snapshot`]) with the world's `Rng` seeded with `seed`.
    pub fn new(spawns: bool, seed: u64) -> Self {
        Self::with(snapshot(spawns), config(), seed)
    }

    /// Any snapshot and config.
    pub fn with(snapshot: WorldSnapshot, config: GameConfig, seed: u64) -> Self {
        let mut world = World::load(config, 16, snapshot).expect("the M2 world loads");
        world.rng = SeededRng::new(seed);
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let engine = Engine::new(world, clock.clone(), SeededRng::new(seed), tx);
        Self {
            engine,
            clock,
            rx,
            bytes: HashMap::new(),
        }
    }

    pub fn w(&mut self) -> &mut World {
        &mut self.engine.world
    }

    /// Collects what the engine has written so far.
    pub fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            if let EngineOutput::Write { id, bytes } = o {
                self.bytes.entry(id).or_default().extend_from_slice(&bytes);
            }
        }
    }

    /// A playing character `name` in `room`, `clear_player()` plus the fields given, on a
    /// fresh session with an account.
    pub fn enter(&mut self, name: &str, room: u32) -> SessionId {
        let now = self.clock.now();
        let w = self.w();
        let id = w.ids.alloc().expect("a free session id");
        let mut s = PlayerSession::new(id, Caps::default(), now);
        let mut c = clear_player();
        c.firstname = name.to_string();
        c.userid = name.to_ascii_uppercase();
        c.map = MapId(1);
        c.room = RoomId(room);
        c.msglevel = MessageLevel::High;
        (c.hp, c.maxhp) = (10, 10);
        c.class = WITCHUNTER;
        c.race = KANG;
        c.sex = b'M';
        c.statline = StatlineMode::On;
        c.lives = 9;
        s.account = Some(AccountRef {
            id: AccountId(i64::from(id.0) + 1),
            username: name.to_ascii_uppercase(),
            sysop: false,
            gameop: false,
            sex: b'M',
        });
        s.created = true;
        s.character = Some(c);
        s.state = SessionState::Playing;
        s.ingame = InGame::Realm;
        w.sessions.insert(id, s);
        id
    }

    /// Kasimir's sheet (T-031's `kang-witchunter.json`): a
    /// level-1 Kang Witchunter, Str 79 Int 30 Wil 45 Agi 42 Hea 74 Cha 30, the quarterstaff
    /// wielded, 48 hit points.
    pub fn kang(&mut self, name: &str, room: u32) -> SessionId {
        let id = self.enter(name, room);
        let c = self.plr(id);
        // CANCAST, as every energy round leaves it (no after-casting surcharge)
        c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
        c.level = 1;
        (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (79, 30, 45, 42, 74, 30);
        (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (79, 30, 45, 42, 74, 30);
        // 74/2 + 6 (basehp) + (74-50)/16 + 4 random = 48
        c.randomhp = 4;
        (c.hp, c.maxhp) = (48, 48);
        (c.energy, c.maxenergy) = (1000, 1000);
        c.items[0] = QUARTERSTAFF;
        c.itemuses[0] = -1;
        c.weapon = QUARTERSTAFF;
        id
    }

    pub fn plr(&mut self, id: SessionId) -> &mut Character {
        self.w()
            .session_mut(id)
            .and_then(|s| s.character.as_mut())
            .expect("a character")
    }

    /// A stock monster forced into `room` (`generate_monster(..., force 1)`), Solo.
    pub fn monster(&mut self, known: i32, room: u32) -> ActiveMonsterId {
        let req = Request {
            areatype: -1,
            monnum: MonsterNo(known),
            minlevel: 0,
            maxlevel: 0,
            owner: None,
            ally: false,
            force: true,
        };
        let w = self.w();
        let m = generate_monster(w, RoomKey::new(1, room), req).expect("placed");
        w.active_monsters.by_number_mut(m).unwrap().grouprole = GroupRole::Solo;
        // the arrival line goes out as a spawn's would at the end of its tick
        self.flush();
        m
    }

    /// Flushes what direct calls on the world queued (an input that does nothing).
    pub fn flush(&mut self) {
        self.engine.handle(EngineInput::Custom {
            session: None,
            run: Box::new(|_| {}),
        });
        self.pump();
    }

    /// Makes `id` impossible to knock down in a test that is about something else.
    pub fn sturdy(&mut self, id: SessionId) {
        let c = self.plr(id);
        (c.hp, c.maxhp) = (30_000, 30_000);
    }

    pub fn mon(&mut self, m: ActiveMonsterId) -> &mut silvermere_types::records::ActiveMonster {
        self.w().active_monsters.by_number_mut(m).expect("alive")
    }

    pub fn alive(&mut self, m: ActiveMonsterId) -> bool {
        self.w().active_monsters.by_number(m).is_some()
    }

    /// The monsters in `room`, in slot order.
    pub fn here(&mut self, room: u32) -> Vec<ActiveMonsterId> {
        self.w().room_state[&RoomKey::new(1, room)]
            .monsters
            .iter()
            .copied()
            .filter(|m| m.0 != 0)
            .collect()
    }

    /// One typed line.
    pub fn line(&mut self, id: SessionId, text: &str) {
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
    }

    /// `n` simulated seconds: one fast tick each.
    pub fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.clock.advance(Duration::from_secs(1));
            self.engine.handle(EngineInput::Tick);
        }
        self.pump();
    }

    /// The scheduler's fast-tick count.
    pub fn tick_count(&mut self) -> u64 {
        self.w().scheduler.tick()
    }

    /// Ticks to the end of the next energy round (every fifth tick).
    pub fn next_round(&mut self) {
        loop {
            let before = self.tick_count();
            self.ticks(1);
            if (before + 1).is_multiple_of(5) {
                return;
            }
        }
    }

    /// Everything written to `id` since the last take, raw.
    pub fn take(&mut self, id: SessionId) -> Vec<u8> {
        self.pump();
        self.bytes.remove(&id).unwrap_or_default()
    }

    /// Everything written to `id` since the last take, as capture lines (`raw_to_lines`):
    /// prompts included.
    pub fn lines(&mut self, id: SessionId) -> Vec<String> {
        raw_to_lines(&self.take(id))
    }

    /// [`M2::lines`] without prompts, bells and empty lines.
    pub fn text(&mut self, id: SessionId) -> Vec<String> {
        self.lines(id)
            .into_iter()
            .map(|l| strip_prompt(&l).trim_matches([' ', '\u{7}']).to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }
}

/// A line without a leading `[HP=...]:` prompt.
pub fn strip_prompt(line: &str) -> &str {
    if line.starts_with("[HP=")
        && let Some(i) = line.find("]:")
    {
        return &line[i + 2..];
    }
    line
}

/// The hit points shown by each prompt in `lines` (`[HP=n` or `[HP=n (Resting) ]`).
pub fn prompt_hps(lines: &[String]) -> Vec<i32> {
    lines
        .iter()
        .filter_map(|l| {
            let rest = l.strip_prefix("[HP=")?;
            let end = rest.find(|c: char| c != '-' && !c.is_ascii_digit())?;
            rest[..end].parse().ok()
        })
        .collect()
}
