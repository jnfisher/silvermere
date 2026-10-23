//! Monsters (T-035): `generate_monster` and the 5-second spawn pass, the known-monster
//! quick reference, `move_monster`, the medium and fast monster passes (wandering and
//! following), the active-monster numbers and their persistence.
//!
//! | Behaviour | Here |
//! |---|---|
//! | generating a monster, random names | [`generate`] |
//! | the spawn pass near players, `generate_monsters_near_users` | [`spawn_pass`] |
//! | the known-monster quick reference | [`quickref`] |
//! | moving a monster, `pick_valid_random_direction` | [`move_monster`] |
//! | the medium and fast monster bodies, confusion | [`following`] |
//! | the WCCACMSR numbers and free list, `remove_active_monster` | [`active`] |
//! | the active-monster cache the background passes walk (T-086) | [`cache`] |
//! | the start-up room pass: specific monsters, lair and shop fill, placed items, exits | [`preload_cycle`] |
//! | the monsters' half of the combat round, T-036 | [`combat_ai`] |
//! | whether a monster sees a player, T-036 | [`visibility`] |
//!
//! `handle_following_monsters` and `give_monsters_a_free_attack` run inside `move_user` and
//! stay with it (`movement::move_user`); the monster they move goes through
//! [`move_monster::move_monster`] by way of [`MONSTER_HOOKS`]. Monster spell rounds are
//! T-052.

pub mod active;
pub mod area_cast;
pub mod cache;
pub mod cast;
pub mod combat_ai;
pub mod following;
pub mod generate;
pub mod move_monster;
pub mod quickref;
pub mod spawn_pass;
pub mod spell_support;
pub mod visibility;

pub use active::{ActiveMonsters, FreeList};

use silvermere_types::enums::{Ability, Direction, ExitType, RoomType};
use silvermere_types::flags::ExitFlags;
use silvermere_types::ids::{ActiveMonsterId, MonsterNo, RoomKey};
use silvermere_types::records::ActiveMonster;
use silvermere_types::sizes::NDIRS;

use crate::movement::exits::{LOCK_LOCKED, live_exits, set_par1, set_par2};
use crate::movement::timed_exits::{add_exit_to_list, blocked_upto};
use crate::movement::{GenerateSpec, MonsterHooks};
use crate::scheduler::Routine;
use crate::world::World;

/// The statics of the spawn pass and the monster passes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnState {
    /// `gennextusr`: the channel pass 1 resumes at.
    pub gennextusr: u16,
    /// `gennextocc`: the occupied-room entry pass 2 resumes at.
    pub gennextocc: usize,
    /// `mapnum` of the last player pass 1 looked at, which pass 2 loads neighbours with
    /// (sic).
    pub last_map: Option<silvermere_types::ids::MapId>,
    /// `mediummoved`: wanderers moved this medium pass.
    pub mediummoved: u32,
    /// `randname`: the last random name built.
    pub randname: String,
}

/// The monster side of `move_user` (`movement::MonsterHooks`), T-035's implementation.
pub struct Monsters;

impl MonsterHooks for Monsters {
    fn generate_monster(&self, world: &mut World, room: RoomKey, spec: GenerateSpec) -> bool {
        let req = generate::Request {
            areatype: spec.group,
            monnum: spec.monster,
            minlevel: spec.minlevel,
            maxlevel: spec.maxlevel,
            owner: None,
            ally: spec.ally,
            force: spec.force,
        };
        generate::generate_monster(world, room, req).is_some()
    }

    fn move_monster(&self, world: &mut World, monster: ActiveMonsterId, dir: Direction) -> bool {
        move_monster::move_monster(world, monster, dir, None, false) == move_monster::Moved::Yes
    }
}

/// The hooks instance `movement::HOOKS` points at.
pub static MONSTER_HOOKS: Monsters = Monsters;

/// The abilities that do not add up for monsters: the best value counts
/// ([`monster_ability_value`]).
fn best_only(ability: Ability) -> bool {
    matches!(
        ability,
        Ability::ColdDefence
            | Ability::HotDefence
            | Ability::AvBless
            | Ability::ResistStone
            | Ability::ResistLightning
            | Ability::Confusion
            | Ability::DamageShield
            | Ability::EnergyUse
            | Ability::ConfuseMsg
            | Ability::AvCurse
            | Ability::AvThird
    )
}

/// A monster's ability value: the spells on the monster (a slot value of 0 takes the
/// monster's rolled `spellvals`, a "Negate ability" for it gives 0), its known monster,
/// every item it carries, its weapon and its worn item; summed, or the best for the
/// non-stacking abilities.
pub fn monster_ability_value(world: &World, mon: &ActiveMonster, ability: Ability) -> i32 {
    let addup = !best_only(ability);
    let mut value: Option<i32> = if addup { Some(0) } else { None };
    let mut add = |v: i32| {
        value = Some(match value {
            None => v,
            Some(cur) if addup => cur + v,
            Some(cur) => cur.max(v),
        });
    };
    for s in &mon.spells {
        if s.spell.0 == 0 {
            continue;
        }
        let Some(spl) = world.spells.get(&s.spell) else {
            continue;
        };
        for (a, v) in &spl.abilities {
            if *a == ability {
                add(if *v != 0 {
                    i32::from(*v)
                } else {
                    i32::from(s.value)
                });
            }
            if *a == Ability::Negate && i32::from(*v) == i32::from(ability.code()) {
                return 0;
            }
        }
    }
    if let Some(k) = world.monsters.get(&mon.knownnum) {
        for (a, v) in &k.abilities {
            if *a == ability {
                add(i32::from(*v));
            }
        }
    }
    let item_value = |n: silvermere_types::ids::ItemNo| -> i32 {
        world.items.get(&n).map_or(0, |itm| {
            itm.abilities
                .iter()
                .filter(|(a, _)| *a == ability)
                .map(|(_, v)| i32::from(*v))
                .sum()
        })
    };
    for (item, _) in &mon.items {
        if item.0 != 0 {
            add(item_value(*item));
        }
    }
    if mon.weapon.0 != 0 {
        add(item_value(mon.weapon));
    }
    if mon.wornitem.0 != 0 {
        add(item_value(mon.wornitem));
    }
    value.unwrap_or(0)
}

/// Whether a monster has the ability: a spell on the monster, its known monster, an item it
/// carries, its weapon or worn item has the ability. A monster numbered 65535 has none.
pub fn monster_has_ability(world: &World, mon: &ActiveMonster, ability: Ability) -> bool {
    if mon.number == active::NOACTNUM {
        return false;
    }
    let spells = mon.spells.iter().filter(|s| s.spell.0 != 0).any(|s| {
        world
            .spells
            .get(&s.spell)
            .is_some_and(|spl| spl.abilities.iter().any(|(a, _)| *a == ability))
    });
    let known = world
        .monsters
        .get(&mon.knownnum)
        .is_some_and(|k| k.abilities.iter().any(|(a, _)| *a == ability));
    let item_has = |n: silvermere_types::ids::ItemNo| {
        n.0 != 0
            && world
                .items
                .get(&n)
                .is_some_and(|itm| itm.abilities.iter().any(|(a, _)| *a == ability))
    };
    spells
        || known
        || mon.items.iter().any(|(n, _)| item_has(*n))
        || item_has(mon.weapon)
        || item_has(mon.wornitem)
}

/// Which start-up cycle runs: a cold start or a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartUp {
    /// The module start: every room is also reset.
    Cold,
    /// A SYSOP restart of the cycle: only the specific monsters and the lair and shop fill
    /// run; the cold-start reset is skipped.
    Reinit,
}

/// `MAXMISSROOMS`: missing room numbers that end a map's scan.
pub const MAXMISSROOMS: u32 = 50;
/// `MAXMISSMAPS`: empty maps that end the scan.
pub const MAXMISSMAPS: u32 = 100;
/// `initrate` as shipped (`INITRATE`, WCCMMUD.MSG:369, `{Rate to perform initialization?
/// 195}`): the records a slice of the start-up cycle handles.
pub const INITRATE: u32 = 195;
/// `MAXACTMONS`: the highest active monster number the cycle looks at.
pub const MAXACTMONS: i32 = 40_000;
/// Lair and shop fill: generated monsters past the first (the fill stops after 15 more).
const LAIRFILL: u32 = 15;

/// The cold start of the start-up cycle, state INIT_ROOMS: see [`preload_cycle`]. Its
/// writes go out once per row (T-113): the pass saves a room each time a monster is
/// generated in or next to it and a known monster each time one of its kind is, about
/// 22,000 saves of 11,000 rows on the first start after a stock import; only the last of
/// each is kept ([`active::coalesce_saves`]).
pub fn preload(world: &mut World) {
    let from = world.outbox.len();
    preload_cycle(world, StartUp::Cold);
    active::coalesce_saves(&mut world.outbox, from);
}

/// The room pass of the start-up cycle (state INIT_ROOMS), all of it at once, not in timed
/// slices. The rooms come in [`room_walk`]
/// order and each gets, in this order:
///
/// 1. a negative area count set to 0;
/// 2. its specific monster (`specmon`) made sure of: RF_SPECALIVE is cleared
///    when the room has none, or when no active monster numbered up to `maxactive` is
///    that known monster generated in this room; a room whose specific monster is then
///    not alive gets it generated (type 0, any level, nobody's, not forced);
/// 3. a Lair or Shop room its monsters, from the room's group, specific monster and level
///    range, until generation fails or 16 are made;
/// 4. on a cold start only: no recent kill, no hidden item found, the placed items put
///    back ([`preload_placed_items`]) and the exits reset ([`preload_exits`]).
///
/// Every RNG draw (the monsters' pool pick, coins, items, names and arrival direction)
/// therefore comes room by room in this order. The recovery mode (WCCRECOV.FLG after an
/// unclean shutdown: duplicate specific monsters killed, room counts rebuilt) is not
/// carried: silvermere's writes are transactional.
pub fn preload_cycle(world: &mut World, kind: StartUp) {
    // the highest active monster number at the start of a slice, at most
    // MAXACTMONS, looked up again at the start of every slice while it is 0. A slice ends
    // when its count reaches `initrate`, and every room number the pass tries
    // counts, missing ones too; the slice that ends resumes at the same number, counted
    // again. Started on an empty WCCACMSR, the value is the highest number in use at the
    // first slice boundary after the first monster is generated, and stays so: the room
    // pass's specific-monster searches and INIT_ACTIVE use it. There is no look-up between
    // the room pass and INIT_ACTIVE (the same slice).
    // Not carried: the low-rate yield on more than MAXPENDING write-backs in a slice
    // (live, as INITRATE 195 is under SLOWRATE 200). While maxactive is 0 it would need
    // more than 10 changed rooms or known monsters evicted within one slice (the caches
    // probe without wrapping, so they evict long before they are full), and it can end
    // the slice of the first monster early. silvermere keeps neither cache. Once the value
    // is not 0 the slices change nothing here, so the specific-monster searches' own
    // counts are not kept.
    let mut maxactive = highest_active(world);
    let mut count = 0;
    for key in room_tries(world) {
        if maxactive == 0 {
            count += 1;
            if count >= INITRATE {
                // a new slice, at this room number
                count = 1;
                maxactive = highest_active(world);
            }
        }
        if let Some(key) = key {
            preload_room(world, key, kind, maxactive);
        }
    }
    preload_active_cache(world, maxactive);
}

/// The highest active monster number in use, at most MAXACTMONS.
fn highest_active(world: &World) -> i32 {
    world
        .active_monsters
        .numbers_in_use()
        .last()
        .map_or(0, |n| n.0.min(MAXACTMONS))
}

/// The cache side of state INIT_ACTIVE: every active monster number from 1 to `maxactive`
/// is looked up by number, (sic) cut to 16 bits and sign-extended, so the cache ends the
/// start-up holding what that walk left in it (T-086). The rest of INIT_ACTIVE (the record
/// repairs and known-monster refresh) and the look-ups of INIT_KNOWN and of the
/// specific-monster search are not carried: INIT_ACTIVE's walk comes last and reads every
/// one of those numbers again.
fn preload_active_cache(world: &mut World, maxactive: i32) {
    for cur in 1..=maxactive {
        let monnum = i32::from(cur as u16 as i16) as u32;
        world.active_monsters.touch_raw(monnum);
    }
}

/// The order the room pass visits the rooms in: there is no list of rooms, so map 1 room 1
/// on are tried in turn. (sic) `missrooms` is reset only at the start of a map and
/// `missmaps` only at the start of the scan, never when a room is found, so a map's scan
/// ends at its MAXMISSROOMS-th missing room number in all (not in a row) and the scan at
/// the MAXMISSMAPS-th map without a room. Rooms past those points are not visited.
pub fn room_walk(world: &World) -> Vec<RoomKey> {
    room_tries(world).into_iter().flatten().collect()
}

/// Every room number [`room_walk`] tries, in order: the room, or `None` for a missing one.
/// Each counts one record of the start-up slice.
pub fn room_tries(world: &World) -> Vec<Option<RoomKey>> {
    let mut out = Vec::new();
    let mut missmaps = 0;
    let mut map = 1u32;
    while missmaps < MAXMISSMAPS {
        let mut missrooms = 0;
        let mut roomfound = false;
        let mut mapmissed = false;
        let mut room = 1u32;
        while missrooms < MAXMISSROOMS {
            let key = RoomKey::new(map, room);
            if world.rooms.contains_key(&key) {
                roomfound = true;
                out.push(Some(key));
            } else {
                out.push(None);
                // the first miss of a map with no room yet counts it empty
                missrooms += 1;
                if !roomfound && !mapmissed {
                    missmaps += 1;
                    mapmissed = true;
                }
            }
            room += 1;
        }
        map += 1;
    }
    out
}

/// One room of the room pass; see [`preload_cycle`].
fn preload_room(world: &mut World, key: RoomKey, kind: StartUp, maxactive: i32) {
    let Some(rm) = world.room(key) else {
        return;
    };
    let (specmon, room_type) = (rm.specmon, rm.room_type);
    let mut changed = false;
    let Some(s) = world.room_state.get_mut(&key) else {
        return;
    };
    // the area monster count cannot be negative
    if s.areacount < 0 {
        s.areacount = 0;
        changed = true;
    }
    // no specific monster: it cannot be alive
    if s.specalive && specmon.0 == 0 {
        s.specalive = false;
        changed = true;
    }
    changed |= specific_monster(world, key, specmon, maxactive).0;
    if matches!(room_type, RoomType::Lair | RoomType::Shop) {
        fill_room(world, key);
    }
    if kind == StartUp::Cold {
        changed |= clear_markers(world, key);
    }
    if changed {
        active::save_room(world, key);
    }
    if kind == StartUp::Cold {
        preload_placed_items(world, key);
        preload_exits(world, key);
    }
}

/// Step 2 of [`preload_cycle`] for one room: its specific monster made sure of, generated
/// when it is not alive. Returns whether the room changed and whether a monster was made.
fn specific_monster(
    world: &mut World,
    key: RoomKey,
    specmon: MonsterNo,
    maxactive: i32,
) -> (bool, bool) {
    let mut changed = false;
    if specmon.0 == 0 {
        return (false, false);
    }
    if world.room_state.get(&key).is_some_and(|s| s.specalive) {
        // look for it among the active monsters 1..=maxactive (that known monster,
        // generated in this room of this map); the search stops at the first copy.
        // (sic) The number is cut to 16 bits, so a monster numbered above 32767 is
        // never found.
        let found = world.active_monsters.values().any(|m| {
            m.number.0 >= 1
                && m.number.0 <= maxactive
                && m.number.0 <= i32::from(i16::MAX)
                && m.knownnum == specmon
                && m.map == key.map
                && m.genroom == key.room
        });
        // not found: it is not alive after all
        if !found && let Some(s) = world.room_state.get_mut(&key) {
            s.specalive = false;
            changed = true;
        }
    }
    // not alive: generate it (type 0, any level 0..0x7FFF, nobody's, not an
    // ally, spawn rules not forced)
    let mut made = false;
    if world.room_state.get(&key).is_some_and(|s| !s.specalive) {
        let req = generate::Request {
            areatype: 0,
            monnum: specmon,
            minlevel: 0,
            maxlevel: 0x7FFF,
            owner: None,
            ally: false,
            force: false,
        };
        made = generate::generate_monster(world, key, req).is_some();
    }
    (changed, made)
}

/// Step 3 of [`preload_cycle`] for one Lair or Shop room: until generation fails, at most
/// 16 (the call comes before the count test). A room lists at most NROOMMONS (15)
/// monsters, so in practice the fill ends at a full list or the room's `maxmonsters`.
/// Returns the monsters made.
fn fill_room(world: &mut World, key: RoomKey) -> u32 {
    let Some(rm) = world.room(key) else {
        return 0;
    };
    let req = generate::Request {
        areatype: rm.mongroup,
        monnum: rm.genmonster,
        minlevel: rm.minlevel,
        maxlevel: rm.maxlevel,
        owner: None,
        ally: false,
        force: false,
    };
    let mut made = 0;
    let mut tries = 0;
    while generate::generate_monster(world, key, req).is_some() {
        made += 1;
        if tries >= LAIRFILL {
            break;
        }
        tries += 1;
    }
    made
}

/// What [`refill_shops`] did, all Shop rooms together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShopFill {
    /// Monsters placed: specific monsters and the fill.
    pub monsters: u32,
}

/// Steps 2 and 3 of [`preload_cycle`] for every Shop room, in [`room_walk`] order: the
/// specific monster made sure of, then the fill from the room's pool and level range up to
/// its cap. A monster still alive in the room counts against the cap. The daily cleanup
/// runs it after [`cold_reset`], as the original's nightly restart refilled them; on a
/// running server only Lairs are refilled (on entry and by the spawn pass), so Lairs are
/// not included. Players in a room see each arrival as at start-up.
pub fn refill_shops(world: &mut World) -> ShopFill {
    let mut done = ShopFill::default();
    let maxactive = highest_active(world);
    for key in room_walk(world) {
        let Some(rm) = world.room(key) else {
            continue;
        };
        if rm.room_type != RoomType::Shop {
            continue;
        }
        let specmon = rm.specmon;
        let (changed, made) = specific_monster(world, key, specmon, maxactive);
        if changed {
            active::save_room(world, key);
        }
        done.monsters += u32::from(made);
        done.monsters += fill_room(world, key);
    }
    done
}

/// What [`cold_reset`] did, all rooms together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColdReset {
    /// Placed items put back.
    pub placed: u32,
    /// Exits locked, re-armed or hidden again.
    pub exits: u32,
}

/// The cold-start room reset of the start-up cycle (step 4 of [`preload_cycle`]) on its own,
/// for every room in [`room_walk`] order: no recent kill, no hidden item found, the placed
/// items put back and the exits reset. The daily cleanup runs it after its passes, as the
/// original's nightly restart did. Nobody in a room is told.
pub fn cold_reset(world: &mut World) -> ColdReset {
    let mut done = ColdReset::default();
    for key in room_walk(world) {
        if clear_markers(world, key) {
            active::save_room(world, key);
        }
        done.placed += preload_placed_items(world, key);
        done.exits += preload_exits(world, key);
    }
    done
}

/// No recent kill (`lastkill`, a minute stamp relative to the start-up day, starts at 0),
/// no hidden item found yet. True when the room changed.
fn clear_markers(world: &mut World, key: RoomKey) -> bool {
    let Some(s) = world.room_state.get_mut(&key) else {
        return false;
    };
    let mut changed = false;
    if s.lastkill != 0 {
        s.lastkill = 0;
        changed = true;
    }
    if s.foundmask != 0 || s.searchers != 0 {
        s.foundmask = 0;
        s.searchers = 0;
        changed = true;
    }
    changed
}

/// The placed items of the start-up cycle, for one room: each placed item no longer lying
/// in the room, visible or hidden, gets a new copy with the item's own charges, in view
/// when it has "Visible Placed Item" or can be picked up, hidden otherwise. (The stock
/// Large Tombs place the class weapons this way, e.g. the mithril cutlass, item 342, in
/// room 1:2236.) Returns the copies put back.
pub fn preload_placed_items(world: &mut World, key: RoomKey) -> u32 {
    let Some(placed) = world.room(key).map(|r| r.placed) else {
        return 0;
    };
    let mut put = 0;
    for item in placed.into_iter().filter(|n| n.0 != 0) {
        let lying = world.room_state.get(&key).is_some_and(|s| {
            s.items.iter().any(|i| i.item == item) || s.hiditems.iter().any(|i| i.item == item)
        });
        if lying {
            continue;
        }
        let Some(itm) = world.items.get(&item) else {
            continue;
        };
        let hidden = !(crate::combat::item_has(itm, Ability::VisiblePlaced) || itm.gettable != 0);
        let charges = itm.charges;
        if crate::items::room_items::add_item_to_room(world, key, item, hidden, charges).is_some() {
            put += 1;
        }
    }
    put
}

/// The exit reset of the start-up cycle, for one room, from the exits as they stand: a Key
/// exit locks (`par2` 2), a disarmed trap re-arms (1 to 0, 4 to 3), a Timed exit's first
/// swap is armed after `par2` steps ([`add_exit_to_list`]), a found hidden exit hides again
/// (or a visible-but-blockable one gets all its blocks back), a Door or Gate locks (`par1`
/// 2). The changes are laid over the content as exit overrides. A change already waiting
/// for one of the room's exits is dropped first, as a restart dropped them all, so a Timed
/// exit keeps one timer. Returns the exits locked, re-armed or hidden again (not the Timed
/// ones).
pub fn preload_exits(world: &mut World, key: RoomKey) -> u32 {
    let Some(exits) = live_exits(world, key) else {
        return 0;
    };
    let mut reset = 0;
    for (i, exit) in exits.iter().enumerate().take(NDIRS) {
        if exit.target.0 == 0 {
            continue;
        }
        let dir = Direction::COMPASS[i];
        world.scheduler.disarm(Routine::ExitRevert {
            room: key,
            dir: dir.code() as u8,
        });
        let par1 = |v: u32| (exit.par1 != v).then_some(v);
        let (p1, p2) = match exit.kind {
            ExitType::Key if exit.par2 != 2 => (None, Some(2)),
            ExitType::Trap | ExitType::SpellTrap => match exit.par2 {
                1 => (None, Some(0)),
                4 => (None, Some(3)),
                _ => (None, None),
            },
            ExitType::Timed => {
                add_exit_to_list(world, key, dir, i32::from(exit.par2));
                (None, None)
            }
            ExitType::Hidden => {
                let f = ExitFlags::from_bits_retain(exit.par1);
                if f.contains(ExitFlags::VISSEARCHED) {
                    (par1(ExitFlags::NEEDSEARCH.bits()), None)
                } else if f.contains(ExitFlags::VISBLOCKED) {
                    let n = exit.par2.unsigned_abs();
                    if (1..=10).contains(&n) {
                        (par1(blocked_upto(u32::from(n))), None)
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            }
            ExitType::Door | ExitType::Gate if exit.par1 != LOCK_LOCKED => {
                (Some(LOCK_LOCKED), None)
            }
            _ => (None, None),
        };
        if let Some(v) = p1 {
            set_par1(world, key, dir, v);
        }
        if let Some(v) = p2 {
            set_par2(world, key, dir, v);
        }
        if p1.is_some() || p2.is_some() {
            reset += 1;
        }
    }
    reset
}
