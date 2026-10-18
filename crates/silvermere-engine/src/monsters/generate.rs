//! [`generate_monster`] and [`get_random_name`]: a new active monster in a room, from the
//! room's pool or a named known monster, announced to the room.
//!
//! The refusals are tested in this order: nothing to choose, the room full (unless
//! it is the room's own specific monster), no free slot, the regeneration delay since the
//! room's last kill (arenas, forced spawns and the specific monster exempt), the area
//! limit of the controlling room, a specific monster already alive, an empty pool, guards
//! and angels in tournament play, `maxalive`, and the wait of a killed unique monster.
//! The pool and pick, the delay and the unique wait are `silvermere_rules::spawn`.
//!
//! Not carried: the crash-recovery flag, the developer-mode audit line, the retries when
//! a new record's number clashes (numbers come from the free list and never clash), the
//! internal-error report and its recovery (the free slot is checked first, so the room
//! add cannot fail).

use silvermere_rules::Rng;
use silvermere_rules::spawn;
use silvermere_text::Msg;
use silvermere_text::ansi;
use silvermere_types::enums::{Direction, ExitType, MonsterType, RoomType};
use silvermere_types::ids::{ActiveMonsterId, MonsterNo, RoomKey, SessionId, TextBlockNo};
use silvermere_types::records::{ActiveMonster, Coins, KnownMonster};
use silvermere_types::sizes::NROOMMONS;

use super::active::{add_monster_to_room, save_active_monster, save_known_monster, save_room};
use crate::display::movement_noise::display_entry_movement;
use crate::items::limited::{Holder, acquire};
use crate::movement::dirname;
use crate::movement::exits::{Arg, fmt_msg, tell_room_all};
use crate::world::World;

/// `RNDNAMSIZ`: a random name holds at most 29 characters.
pub const RNDNAMSIZ: usize = 30;
/// `NAMELINES`: lines of a name list looked at.
pub const NAMELINES: usize = 1000;

/// What [`generate_monster`] is asked for: the arguments after the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// `areatype`: the `montype` to pick from, -1 any.
    pub areatype: i16,
    /// `monnum`: a specific known monster, or 0 for the pool.
    pub monnum: MonsterNo,
    /// `minlevel`, `maxlevel`: the genindex range (0 = no limit; a pool pick needs a
    /// non-zero `maxlevel`).
    pub minlevel: i16,
    pub maxlevel: i16,
    /// `owner`: the player it follows, `-1` none.
    pub owner: Option<SessionId>,
    /// `ally`: a friend of the owner.
    pub ally: bool,
    /// `force`: ignore the room's spawn rules.
    pub force: bool,
}

impl Request {
    /// The spawn pass's call for a room: the room's group, specific monster and level
    /// range, no owner, not forced.
    pub fn for_room(world: &World, room: RoomKey) -> Option<Self> {
        let rm = world.room(room)?;
        Some(Self {
            areatype: rm.mongroup,
            monnum: rm.genmonster,
            minlevel: rm.minlevel,
            maxlevel: rm.maxlevel,
            owner: None,
            ally: false,
            force: false,
        })
    }
}

/// A new active monster in `room`. Returns the new monster's number, or `None` when no
/// monster was made.
pub fn generate_monster(world: &mut World, room: RoomKey, req: Request) -> Option<ActiveMonsterId> {
    let Request {
        areatype,
        monnum,
        minlevel,
        maxlevel,
        owner,
        ally,
        force,
    } = req;
    // nothing to choose from
    if maxlevel == 0 && monnum.0 == 0 {
        return None;
    }
    // the room exists and has room, unless it is the room's own monster. (A
    // forced spawn into a missing room crashed the original; here it stops.)
    let rm = world.room(room)?.clone();
    let state = world.room_state.get(&room)?.clone();
    let own_specmon = monnum.0 != 0 && rm.specmon == monnum;
    if state.nummonsters >= rm.maxmonsters && !own_specmon && !force {
        return None;
    }
    // a free monster slot
    if !state.monsters.iter().take(NROOMMONS).any(|m| m.0 == 0) {
        return None;
    }
    // the regeneration delay
    if rm.room_type != RoomType::Arena && !force && state.lastkill != 0 && !own_specmon {
        let now = world.minute_stamp();
        let regenmins = u16::try_from(world.config.regen_mins).unwrap_or(u16::MAX);
        if spawn::regen_blocked(state.lastkill, now, rm.regendelay, regenmins) {
            return None;
        }
    }
    // the area limit of the controlling room. A controlling room that does not
    // exist is ignored (the room record keeps its `ctlroom`).
    let mut ctl = None;
    if rm.ctlroom.0 != 0 && !own_specmon {
        let c = RoomKey {
            map: room.map,
            room: rm.ctlroom,
        };
        if let (Some(crm), Some(cs)) = (world.room(c), world.room_state.get(&c)) {
            if cs.areacount >= crm.areamax && !force {
                return None;
            }
            ctl = Some(c);
        }
    }
    // the known monster
    let known_no = if monnum.0 != 0 {
        if rm.specmon == monnum && state.specalive {
            return None;
        }
        monnum
    } else {
        let pool = spawn::pool(&world.monqref, areatype, minlevel, maxlevel);
        spawn::pick(&pool, &mut world.rng)?
    };
    let kmon = world.monsters.get(&known_no)?.clone();
    // tournament play: no guards, no angels
    if world.config.tournament_play
        && matches!(kmon.montype, MonsterType::Guard | MonsterType::Angel)
    {
        return None;
    }
    // how many may be alive; a killed unique monster's wait
    if kmon.maxalive > 0 {
        if kmon.curalive >= kmon.maxalive {
            return None;
        }
        if kmon.maxalive == 1 {
            let now = world.wall;
            if !spawn::unique_ready(kmon.killed_at, now, kmon.regenhrs, &mut world.rng) {
                return None;
            }
        }
        if let Some(k) = world.monsters.get_mut(&known_no) {
            k.curalive += 1;
        }
        save_known_monster(world, known_no);
    }
    let number = world.active_monsters.numbers.take();
    let mut newmon = new_record(world, &kmon, number, room);
    // carried items: each by its chance; a limited monster never killed carries
    // them all
    for (j, slot) in kmon.items.iter().enumerate() {
        let gets = slot.item.0 != 0
            && ((kmon.maxalive != 0 && kmon.killed_at == 0)
                || world.rng.range(1, 100) <= i32::from(slot.pct));
        if gets
            && world.items.contains_key(&slot.item)
            && acquire(world, slot.item, Holder::Monster(number))
        {
            newmon.items[j] = (slot.item, slot.uses);
        }
    }
    // summoned for a player: it follows the player
    if let Some(o) = owner
        && let Some(name) = world
            .session(o)
            .and_then(|s| s.player())
            .map(|p| p.firstname.clone())
    {
        newmon.following = name;
        newmon.friendly = u8::from(ally);
    }
    // the name
    newmon.name = if kmon.namelist != 0 {
        let n = get_random_name(world, TextBlockNo(kmon.namelist), &kmon.name);
        n.chars().take(RNDNAMSIZ - 1).collect()
    } else {
        kmon.name.clone()
    };
    let name = newmon.name.clone();
    world.active_monsters.insert(newmon);
    // into the room, the counters
    if add_monster_to_room(world, room, number).is_none() {
        // unreachable after the slot check above; undo rather than leave a stray record
        super::active::remove_active_monster(world, number, false);
        return None;
    }
    if let Some(s) = world.room_state.get_mut(&room) {
        if rm.specmon == kmon.number {
            s.specalive = true;
        } else {
            s.nummonsters = s.nummonsters.saturating_add(1);
        }
    }
    if rm.specmon != kmon.number
        && let Some(c) = ctl
    {
        if let Some(cs) = world.room_state.get_mut(&c) {
            cs.areacount += 1;
        }
        save_room(world, c);
    }
    save_room(world, room);
    save_active_monster(world, number);
    // the direction it seems to come from
    let usable: [bool; 8] =
        std::array::from_fn(|k| rm.exits[k].target.0 != 0 && rm.exits[k].kind == ExitType::Normal);
    let dir = spawn::arrival_direction(usable, &mut world.rng).map(|k| Direction::COMPASS[k]);
    let msg = arrival_text(world, &kmon, &name, dir);
    tell_room_all(world, room, [None, None], msg);
    // every neighbouring room hears it (exit 11, NOEXIT)
    display_entry_movement(world, None, room);
    // the creation spell, at level 1, at nobody
    if kmon.createspell.0 != 0 {
        // the new monster comes into the cache only here: it was written straight to
        // WCCACMSR (T-086)
        world.active_monsters.touch(number);
        super::cast::cast_by_number(world, number, kmon.createspell, 1, None);
    }
    Some(number)
}

/// The record copied from the known monster.
fn new_record(
    world: &mut World,
    kmon: &KnownMonster,
    number: ActiveMonsterId,
    room: RoomKey,
) -> ActiveMonster {
    let mut m = crate::world::blank::clear_active_monster(number);
    m.number = number;
    m.knownnum = kmon.number;
    m.level = kmon.level;
    m.map = room.map;
    m.room = room.room;
    m.genroom = room.room;
    m.poison = 0;
    m.energy = kmon.energy;
    m.hp = kmon.maxhp;
    m.hpregen = kmon.hpregen;
    m.hunting = ActiveMonsterId(0);
    m.exp = kmon.exp;
    m.ac = kmon.ac;
    m.dr = kmon.dr;
    m.maxhp = kmon.maxhp;
    m.alignment = kmon.alignment;
    m.followpct = kmon.followpct;
    m.maxenergy = kmon.energy as i16;
    m.montype = kmon.montype;
    m.grouprole = kmon.grouprole;
    m.created = world.wall;
    m.target = -1;
    // coins: runic first, each up to the known maximum
    let roll = |rng: &mut crate::scheduler::SeededRng, max: u32| -> u32 {
        if max > 0 {
            rng.long(0, i64::from(max) + 1) as u32
        } else {
            0
        }
    };
    let k = kmon.cash;
    let runic = roll(&mut world.rng, k.runic);
    let platinum = roll(&mut world.rng, k.platinum);
    let gold = roll(&mut world.rng, k.gold);
    let silver = roll(&mut world.rng, k.silver);
    let copper = roll(&mut world.rng, k.copper);
    m.cash = Coins {
        copper,
        silver,
        gold,
        platinum,
        runic,
    };
    m.wornitem = kmon.wornitem;
    m.weapon = kmon.weapon;
    m
}

/// The arrival line, yellow with the name and green after it: the known monster's WCCMSG
/// `arrivemsg` text, or the built-in lines.
fn arrival_text(world: &World, kmon: &KnownMonster, name: &str, dir: Option<Direction>) -> Msg {
    let dirword = dir.map(|d| dirname(world, d));
    if kmon.arrivemsg.0 != 0 {
        return match world.messages.get(&kmon.arrivemsg) {
            Some(msg) if !msg.msgs[0].is_empty() => {
                let from = dirword
                    .as_ref()
                    .map_or_else(|| "nowhere".to_string(), |d| format!("the {d}"));
                let green = std::str::from_utf8(ansi::ANSI_GREEN).unwrap_or("");
                let named = format!("{name}{green}");
                // with a random name the name goes first, else the direction
                let text = if kmon.namelist != 0 {
                    fmt_msg(&msg.msgs[0], &[Arg::S(&named), Arg::S(&from)])
                } else {
                    fmt_msg(&msg.msgs[0], &[Arg::S(&from), Arg::S(name)])
                };
                Msg::MonsterArrivalText { text }
            }
            // a record with an empty first line: only the colour prefix goes out
            Some(_) => Msg::MonsterArrivalBlank,
            // no such record
            None => match dirword {
                None => Msg::MonsterMovesInNowhere { mon: name.into() },
                Some(d) => Msg::MonsterArrivesDir {
                    mon: name.into(),
                    dir: d,
                },
            },
        };
    }
    match dirword {
        None => Msg::MonsterJustArrivedNowhere { mon: name.into() },
        Some(d) => Msg::MonsterJustArrivedDir {
            mon: name.into(),
            dir: d,
        },
    }
}

/// A random name: the lines of the name list (WCCTEXT part 0), each replacing the name so
/// far (`N:` the monster's own name, `F:text` the text, `B:text` the text before the name,
/// `A:text` after it, any other line as written), with a 10% chance (a roll of 0-100 below
/// 10) to stop after each; the last line read stands. No block: the monster's name. A block
/// with no text keeps the previous call's name (sic, the static buffer), the monster's name
/// when that is empty.
pub fn get_random_name(world: &mut World, block: TextBlockNo, defname: &str) -> String {
    let Some(text) = world.text_blocks.get(&(0, block)).map(|b| b.text.clone()) else {
        world.spawn.randname = defname.to_string();
        return world.spawn.randname.clone();
    };
    let cut = |s: &str, n: usize| -> String { s.chars().take(n).collect() };
    let mut rest = text.as_str();
    let mut cnt = 0;
    while !rest.is_empty() && cnt < NAMELINES {
        cnt += 1;
        let (line, tail) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = tail;
        let bytes = line.as_bytes();
        let name = if bytes.len() >= 2 && bytes[1] == b':' {
            let body = &line[2..];
            match bytes[0] {
                b'N' => cut(defname, RNDNAMSIZ - 1),
                b'F' => cut(body, RNDNAMSIZ - 1),
                b'B' => cut(
                    &format!("{} {defname}", cut(body, RNDNAMSIZ - 2)),
                    RNDNAMSIZ - 1,
                ),
                b'A' => cut(&format!("{defname} {body}"), RNDNAMSIZ - 1),
                _ => cut(line, RNDNAMSIZ - 1),
            }
        } else {
            cut(line, RNDNAMSIZ - 1)
        };
        world.spawn.randname = name;
        if world.rng.range(0, 100) < 10 {
            break;
        }
    }
    if world.spawn.randname.is_empty() {
        world.spawn.randname = defname.to_string();
    }
    world.spawn.randname.clone()
}
