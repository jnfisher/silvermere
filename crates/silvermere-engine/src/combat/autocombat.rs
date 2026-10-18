//! Auto-combat (T-034): the ring of fighting players and the energy round.
//!
//! A player who attacks is queued; every energy round (`background_energy`, 5 s) the
//! queue is run once up to a round-end marker, and each queued player fights one round
//! through T-033's attack functions, which queue the player again ([`engage`]) while the
//! fight goes on. A player is "in auto-combat" exactly while queued
//! ([`is_inside_autocombat`]); the entry ([`Entry`]) keeps its target and move after the
//! fight ends, for ATTACK or BASH alone ([`restart_autocombat`]).
//!
//! There are no global flags: whether a call is an auto-combat round is T-033's
//! [`Mode`] argument, the move is the entry's, handed to the attack call, and every cast
//! the spell hook makes is an automatic one. The ring grows as needed (the original's 550
//! slots are not carried); its FIFO order and the round-end marker are.
//!
//! What other cards own is reached through [`HOOKS`]: the monsters' round (T-036) and the
//! auto-combat spells (T-050).

use std::collections::{BTreeMap, VecDeque};

use silvermere_rules::Rng;
use silvermere_rules::combat::Move;
use silvermere_text::Msg;
use silvermere_types::enums::WeaponType;
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, SessionId, SpellNo};

use super::{
    AutoTarget, Mode, attack_monster_monster, attack_monster_user, attack_user_monster,
    attack_user_user, monster, monster_mut, player, player_mut, room_of,
};
use crate::items::inventory::matches_any_word;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Most queue entries handled in one pass.
pub const MAX_ROUND_RUN: usize = 1000;

/// The percentage of rounds in which the players strike first.
pub const PLAYERS_FIRST: i32 = 60;

/// Above this encumbrance a player gets no moves.
const HEAVY_LOAD: i16 = 66;

/// The number of a free monster cache slot.
const FREE_MONSTER: i32 = 65535;

/// One player's auto-combat entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The player or monster fought, `None` for neither (a spell that needs no target).
    pub target: Option<AutoTarget>,
    /// The move each round is made with (a plain attack is [`Move::Attack`]).
    pub mv: Move,
    /// Each round casts `spell` instead of striking.
    pub casting: bool,
    /// The spell cast when `casting`.
    pub spell: SpellNo,
}

impl Default for Entry {
    /// The entry of a player who never fought.
    fn default() -> Self {
        Self {
            target: None,
            mv: Move::Attack,
            casting: false,
            spell: SpellNo(0),
        }
    }
}

/// A slot of the ring: a player due to act, or the round-end marker. A killed slot is
/// simply taken out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RingItem {
    Session(SessionId),
    RoundEnd,
}

/// The auto-combat state of the realm: the entries and the ring.
#[derive(Debug, Clone, Default)]
pub struct AutoCombat {
    entries: BTreeMap<SessionId, Entry>,
    ring: VecDeque<RingItem>,
}

impl AutoCombat {
    /// The player's entry; a fresh one when the player never fought.
    pub fn entry(&self, id: SessionId) -> Entry {
        self.entries.get(&id).copied().unwrap_or_default()
    }

    fn entry_mut(&mut self, id: SessionId) -> &mut Entry {
        self.entries.entry(id).or_default()
    }

    /// Whether the player is queued.
    pub fn queued(&self, id: SessionId) -> bool {
        self.ring.contains(&RingItem::Session(id))
    }

    /// The queued players in ring order, round-end markers left out.
    pub fn queue(&self) -> Vec<SessionId> {
        self.ring
            .iter()
            .filter_map(|r| match r {
                RingItem::Session(id) => Some(*id),
                RingItem::RoundEnd => None,
            })
            .collect()
    }
}

/// The work of other cards an auto-combat round calls. The defaults do nothing.
pub trait RoundHooks: Sync {
    /// Every monster's round. T-036.
    fn monster_combat(&self, _world: &mut World) {}
    /// The auto-combat spell, at the player or monster target or, without one, at none. The
    /// cast is an automatic one. T-050.
    fn cast(
        &self,
        _world: &mut World,
        _id: SessionId,
        _spell: SpellNo,
        _target: Option<AutoTarget>,
    ) {
    }
    /// CAST alone repeating the entry's spell ([`restart_autocombat`]): an automatic cast
    /// at the player or monster target, made as a typed command makes it. T-050.
    fn restart_cast(
        &self,
        _world: &mut World,
        _id: SessionId,
        _spell: SpellNo,
        _target: Option<AutoTarget>,
    ) {
    }
}

/// The defaults.
pub struct NoRoundHooks;

impl RoundHooks for NoRoundHooks {}

/// The auto-combat spell of T-050 ([`crate::spells::autocombat_cast`]) and the monsters'
/// round of T-036 ([`crate::monsters::combat_ai::do_monster_combat`]).
pub struct SpellRoundHooks;

impl RoundHooks for SpellRoundHooks {
    fn monster_combat(&self, world: &mut World) {
        crate::monsters::combat_ai::do_monster_combat(world);
    }
    fn cast(&self, world: &mut World, id: SessionId, spell: SpellNo, target: Option<AutoTarget>) {
        crate::spells::autocombat_cast(world, id, spell, target);
    }
    fn restart_cast(
        &self,
        world: &mut World,
        id: SessionId,
        spell: SpellNo,
        target: Option<AutoTarget>,
    ) {
        crate::spells::cast::restart_cast(world, id, spell, target);
    }
}

/// The hooks in use.
pub static HOOKS: &dyn RoundHooks = &SpellRoundHooks;

// ---------------------------------------------------------------------------------------
// The entry and the ring

/// The entry's targets go; the move, the spell and the ring stay.
pub fn clear_autocombat_info(world: &mut World, id: SessionId) {
    world.autocombat.entry_mut(id).target = None;
}

/// A backstab entry becomes a plain attack (as moving does).
pub fn clear_backstab_attack_type(world: &mut World, id: SessionId) {
    if let Some(e) = world.autocombat.entries.get_mut(&id)
        && e.mv == Move::Backstab
    {
        e.mv = Move::Attack;
    }
}

/// The session leaves for good (its id may be handed out again): no slot in the ring and a
/// fresh entry, so the next player given the id starts clean.
pub fn forget(world: &mut World, id: SessionId) {
    world
        .autocombat
        .ring
        .retain(|r| *r != RingItem::Session(id));
    world.autocombat.entries.remove(&id);
}

/// The entry takes the target, the move `mv` (the attack in progress) and the spell, and
/// the player goes to the end of the ring for the next round.
pub fn engage(
    world: &mut World,
    id: SessionId,
    target: Option<AutoTarget>,
    mv: Move,
    spell: Option<SpellNo>,
) {
    let ac = &mut world.autocombat;
    *ac.entry_mut(id) = Entry {
        target,
        mv,
        casting: spell.is_some(),
        spell: spell.unwrap_or(SpellNo(0)),
    };
    ac.ring.push_back(RingItem::Session(id));
}

/// Every slot the player holds is freed and energy above one round is cut (every caller
/// passes `reset` 1). Nothing is said.
pub fn kill_autocombat(world: &mut World, id: SessionId) {
    world
        .autocombat
        .ring
        .retain(|r| *r != RingItem::Session(id));
    if let Some(p) = player_mut(world, id)
        && p.energy > p.maxenergy
    {
        p.energy = p.maxenergy;
    }
}

/// Everyone in the game fighting the player stops, untold.
pub fn kill_autocombat_against(world: &mut World, target: SessionId) {
    for id in world.session_ids() {
        if world.is_in_game(id) && is_combatting_user(world, id, target) {
            kill_autocombat(world, id);
        }
    }
}

/// Everyone in the game fighting the monster stops, untold.
pub fn kill_autocombat_against_monster(world: &mut World, mon: ActiveMonsterId) {
    for id in world.session_ids() {
        if world.is_in_game(id) && is_combatting_monster(world, id, mon) {
            kill_autocombat(world, id);
        }
    }
}

/// `*Combat Off*` and the prompt; with `announce`, `<Name> breaks off combat.` to everyone
/// else in the room, blind players too, each with their prompt. The top list is brought up
/// to date.
pub fn display_autocombat_broken(world: &mut World, id: SessionId, announce: bool) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.output.push(Msg::CombatOff);
    s.push_prompt();
    world.update_top_list(id);
    if announce && let Some(plr) = player(world, id) {
        let key = room_of(plr);
        let msg = Msg::CombatBreaksOff {
            who: plr.firstname.clone(),
        };
        tell_room(
            world,
            key,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &msg,
        );
    }
}

// ---------------------------------------------------------------------------------------
// Queries

/// Whether the player is queued.
pub fn is_inside_autocombat(world: &World, id: SessionId) -> bool {
    world.autocombat.queued(id)
}

/// Whether the player is queued against that player.
pub fn is_combatting_user(world: &World, id: SessionId, target: SessionId) -> bool {
    world.autocombat.entry(id).target == Some(AutoTarget::Player(target))
        && is_inside_autocombat(world, id)
}

/// Whether the player is queued against that monster.
pub fn is_combatting_monster(world: &World, id: SessionId, mon: ActiveMonsterId) -> bool {
    world.autocombat.entry(id).target == Some(AutoTarget::Monster(mon))
        && is_inside_autocombat(world, id)
}

/// Whether someone in the game fights the player.
pub fn is_being_attacked(world: &World, id: SessionId) -> bool {
    world
        .session_ids()
        .into_iter()
        .any(|i| world.is_in_game(i) && is_combatting_user(world, i, id))
}

/// Whether the player fights a player or is fought by one.
pub fn is_in_pvp_combat(world: &World, id: SessionId) -> bool {
    world.session_ids().into_iter().any(|i| {
        world.is_in_game(i)
            && (is_combatting_user(world, id, i) || is_combatting_user(world, i, id))
    })
}

/// Whether someone fighting the player stands in the player's room (`can_sneak`).
pub fn attacker_is_in_same_room(world: &World, id: SessionId) -> bool {
    let Some(plr) = player(world, id) else {
        return false;
    };
    world.session_ids().into_iter().any(|i| {
        world.is_in_game(i)
            && is_combatting_user(world, i, id)
            && player(world, i).is_some_and(|t| t.room == plr.room && t.map == plr.map)
    })
}

/// The auto-combat target, when `name` matches it. A player target is the only one tried
/// when there is one.
pub fn match_combat_target(world: &World, id: SessionId, name: &str) -> Option<AutoTarget> {
    match world.autocombat.entry(id).target? {
        AutoTarget::Player(t) => player(world, t)
            .filter(|p| matches_any_word(name, &p.firstname))
            .map(|_| AutoTarget::Player(t)),
        AutoTarget::Monster(m) => monster(world, m)
            .filter(|mon| matches_any_word(name, &mon.name))
            .map(|_| AutoTarget::Monster(m)),
    }
}

/// The entry's move, kept after the fight.
pub fn get_my_attack_type(world: &World, id: SessionId) -> Move {
    world.autocombat.entry(id).mv
}

/// The move of the first player found fighting `id`, or `None` when no player is.
pub fn attacker_type(world: &World, id: SessionId) -> Option<Move> {
    world
        .session_ids()
        .into_iter()
        .find(|&i| world.is_in_game(i) && is_combatting_user(world, i, id))
        .map(|i| world.autocombat.entry(i).mv)
}

// ---------------------------------------------------------------------------------------
// Validation and restart

/// Each energy round: a player target out of the room marks the attacker's target as gone
/// (and resets `pvpcnt`), back in the room clears it; a monster target gone or elsewhere
/// ends auto-combat with `*Combat Off*`.
pub fn validate_auto_combat(world: &mut World, id: SessionId) {
    if !is_inside_autocombat(world, id) {
        return;
    }
    let Some(plr) = player(world, id) else {
        return;
    };
    let here = room_of(plr);
    match world.autocombat.entry(id).target {
        Some(AutoTarget::Player(t)) => {
            let with = player(world, t).is_some_and(|tp| room_of(tp) == here);
            if let Some(p) = player_mut(world, id) {
                if !with {
                    if !p.status.contains(PlayerStatus::TARGETGONE) {
                        p.status.insert(PlayerStatus::TARGETGONE);
                        p.pvpcnt = 0;
                    }
                } else {
                    p.status.remove(PlayerStatus::TARGETGONE);
                }
            }
        }
        Some(AutoTarget::Monster(m)) => {
            let with =
                monster(world, m).is_some_and(|mon| mon.room == here.room && mon.map == here.map);
            if !with {
                kill_autocombat(world, id);
                display_autocombat_broken(world, id, false);
            }
        }
        None => {}
    }
}

/// ATTACK or BASH alone (`spell` false) or CAST alone (`spell` true) repeats the entry's
/// attack or spell at once, as a typed command ([`Mode::Command`]: `*Combat Engaged*`
/// again). Only when the player is not queued and the entry is of the kind typed. A
/// backstab entry: never with a two-handed weapon (the entry falls back to a plain attack
/// for good), and only from hiding without sneaking (otherwise this blow is a plain one).
/// The target must still be here. Returns whether the attack or spell was made.
pub fn restart_autocombat(world: &mut World, id: SessionId, spell: bool) -> bool {
    if is_inside_autocombat(world, id) {
        return false;
    }
    let e = world.autocombat.entry(id);
    if e.casting != spell {
        return false;
    }
    let mut atktype = e.mv;
    let mut atkmove = e.mv;
    if !spell && atkmove == Move::Backstab {
        let Some(plr) = player(world, id).cloned() else {
            return false;
        };
        if plr.weapon.0 != 0
            && world.items.get(&plr.weapon).is_some_and(|itm| {
                matches!(
                    itm.wpntype,
                    WeaponType::TwoHandBlunt | WeaponType::TwoHandSharp
                )
            })
        {
            tell_user(world, id, Msg::CombatNoBackstabWeapon);
            atktype = Move::Attack;
            atkmove = Move::Attack;
            world.autocombat.entry_mut(id).mv = Move::Attack;
        }
        if plr.hidden == 0 || plr.status.contains(PlayerStatus::SNEAKING) {
            atkmove = Move::Attack;
        }
    }
    match e.target {
        Some(AutoTarget::Player(t)) => {
            if !world.is_in_game(t) {
                return false;
            }
            if let (Some(tp), Some(p)) = (player(world, t), player(world, id))
                && (room_of(tp) != room_of(p)
                    || (tp.hidden != 0 && !p.status.contains(PlayerStatus::SEEHIDDEN)))
            {
                let who = tp.firstname.clone();
                tell_user(world, id, Msg::CombatNotHere { who });
                return false;
            }
            if e.casting {
                if !world.spells.contains_key(&e.spell) {
                    return false;
                }
                HOOKS.restart_cast(world, id, e.spell, e.target);
                return true;
            }
            attack_user_user(world, id, t, atkmove, atktype, true, Mode::Command);
            true
        }
        Some(AutoTarget::Monster(m)) => {
            // (a monster that no longer loads does not stop it)
            if let (Some(mon), Some(p)) = (monster(world, m), player(world, id))
                && (mon.room != p.room || mon.map != p.map)
            {
                tell_user(world, id, Msg::CombatTargetGone);
                return false;
            }
            if e.casting {
                if !world.spells.contains_key(&e.spell) {
                    return false;
                }
                HOOKS.restart_cast(world, id, e.spell, e.target);
                return true;
            }
            attack_user_monster(world, id, m, atkmove, atktype, Mode::Command);
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------------------
// The round

/// A round-end marker goes in, then the ring is run up to it, so a player queued again
/// during the round waits for the next one. At most [`MAX_ROUND_RUN`] entries.
pub fn do_autocombat(world: &mut World) {
    world.autocombat.ring.push_back(RingItem::RoundEnd);
    let mut cnt = 0;
    while let Some(item) = world.autocombat.ring.pop_front() {
        let RingItem::Session(id) = item else {
            break;
        };
        if cnt >= MAX_ROUND_RUN {
            // this one is already taken out and is not put back: it drops out of the ring
            break;
        }
        cnt += 1;
        do_autocombat_for_user(world, id);
    }
}

/// One round for one player. Out of hit points or with the target player gone from the
/// game, `*Combat Off*` and no round. Otherwise the entry's spell or a [`Mode::Auto`]
/// attack with the entry's move; the attack queues the player again when the fight goes on,
/// so a round that does not is the last. Returns whether the round was made.
pub fn do_autocombat_for_user(world: &mut World, id: SessionId) -> bool {
    if !world.is_in_game(id) {
        return false;
    }
    let Some(plr) = player(world, id) else {
        return false;
    };
    if plr.hp <= 0 {
        display_autocombat_broken(world, id, false);
        return false;
    }
    // the stored move, every round
    let e = world.autocombat.entry(id);
    match e.target {
        Some(AutoTarget::Player(t)) => {
            if !world.is_in_game(t) {
                display_autocombat_broken(world, id, false);
                return false;
            }
            if e.casting {
                if world.spells.contains_key(&e.spell) {
                    HOOKS.cast(world, id, e.spell, e.target);
                }
            } else {
                attack_user_user(world, id, t, e.mv, e.mv, true, Mode::Auto);
            }
        }
        Some(AutoTarget::Monster(m)) => {
            if e.casting {
                if world.spells.contains_key(&e.spell) {
                    HOOKS.cast(world, id, e.spell, e.target);
                }
            } else {
                attack_user_monster(world, id, m, e.mv, e.mv, Mode::Auto);
            }
        }
        None => {
            if e.casting && world.spells.contains_key(&e.spell) {
                HOOKS.cast(world, id, e.spell, None);
            }
        }
    }
    true
}

/// The round, only for a melee backstab.
fn do_backstab_for_user(world: &mut World, id: SessionId) -> bool {
    let e = world.autocombat.entry(id);
    e.mv == Move::Backstab && !e.casting && do_autocombat_for_user(world, id)
}

/// One pass up to a new round-end marker before the round; backstabs strike now, every
/// other entry (and a backstab that did not strike) goes back behind the marker for
/// [`do_autocombat`].
pub fn do_backstab_autocombat(world: &mut World) {
    world.autocombat.ring.push_back(RingItem::RoundEnd);
    let mut cnt = 0;
    while let Some(item) = world.autocombat.ring.pop_front() {
        let RingItem::Session(id) = item else {
            break;
        };
        if cnt >= MAX_ROUND_RUN {
            break;
        }
        cnt += 1;
        if !do_backstab_for_user(world, id) {
            world.autocombat.ring.push_back(RingItem::Session(id));
        }
    }
}

/// For T-036's monster round: a monster charmed by `master` (who is in auto-combat) attacks
/// the master's target; when the master attacks the monster itself the charm breaks (its
/// Enslave spells end; their ending texts are T-050's).
pub fn do_charmed_monster_combat(world: &mut World, master: SessionId, mon: ActiveMonsterId) {
    match world.autocombat.entry(master).target {
        Some(AutoTarget::Player(t)) => {
            attack_monster_user(world, mon, t);
        }
        Some(AutoTarget::Monster(m)) if m == mon => {
            let enslaving: Vec<bool> = match monster(world, mon) {
                Some(mn) => mn
                    .spells
                    .iter()
                    .map(|c| {
                        c.spell != SpellNo(0)
                            && world.spells.get(&c.spell).is_none_or(|s| {
                                s.abilities
                                    .iter()
                                    .any(|(a, _)| *a == silvermere_types::enums::Ability::Enslave)
                            })
                    })
                    .collect(),
                None => return,
            };
            if let Some(mn) = monster_mut(world, mon) {
                mn.flags.remove(ActiveMonsterFlags::CHARMED);
                for (slot, clear) in mn.spells.iter_mut().zip(enslaving) {
                    if clear {
                        slot.spell = SpellNo(0);
                        slot.value = 0;
                        slot.rounds = 0;
                    }
                }
            }
        }
        Some(AutoTarget::Monster(m)) => {
            attack_monster_monster(world, mon, m);
        }
        None => {}
    }
}

/// A player's energy: no longer just arrived; a round of energy on top of what is left, cut
/// to one round outside auto-combat (so a fighter carries the remainder over), none while
/// the player target is gone and the pool full; no moves above 66% encumbrance; then
/// [`validate_auto_combat`].
pub fn energy_update_character(world: &mut World, id: SessionId) {
    let inside = is_inside_autocombat(world, id);
    let Some(p) = player_mut(world, id) else {
        return;
    };
    p.status.remove(PlayerStatus::JUSTARRIVED);
    if p.energy < p.maxenergy || !p.status.contains(PlayerStatus::TARGETGONE) {
        p.energy = p.energy.saturating_add(p.maxenergy);
        if p.energy > p.maxenergy && !inside {
            p.energy = p.maxenergy;
        }
    }
    if p.encumpct > HEAVY_LOAD {
        p.moves = 0;
    }
    validate_auto_combat(world, id);
}

/// The monsters' energy: every monster in the cache, slots in order (T-086): below one
/// round a round is added, above it the energy is cut to one round; the target is
/// forgotten. A monster not in the cache keeps its energy.
pub fn energy_update_monsters(world: &mut World) {
    let cache = world.active_monsters.cache();
    let numbers: Vec<_> = (0..cache.monsbuf())
        .map(|slot| cache.number_at(slot))
        .collect();
    drop(cache);
    for n in numbers {
        if n == 0 || n == FREE_MONSTER as u32 {
            continue;
        }
        let Some(m) = world
            .active_monsters
            .peek_mut(silvermere_types::ids::ActiveMonsterId(n as i32))
        else {
            continue;
        };
        let max = i32::from(m.maxenergy);
        let energy = i32::from(m.energy);
        if energy < max {
            m.energy = (energy + max).clamp(0, i32::from(u16::MAX)) as u16;
        } else if energy > max {
            m.energy = max.max(0) as u16;
        }
        m.target = -1;
    }
}

/// The energy round: energy for every player then every monster; the backstabs; then with a
/// 60% roll the players' auto-combat before the monsters' round, else after; then everyone
/// may cast again and a player attacked in the round is spotted for the next one. Returns
/// whether the players went first. The re-arm is the scheduler's.
pub fn background_energy(world: &mut World) -> bool {
    let ids: Vec<SessionId> = world
        .session_ids()
        .into_iter()
        .filter(|&id| world.is_in_game(id))
        .collect();
    for &id in &ids {
        energy_update_character(world, id);
    }
    energy_update_monsters(world);
    do_backstab_autocombat(world);
    let players_first = world.rng.range(0, 100) < PLAYERS_FIRST;
    if players_first {
        do_autocombat(world);
        HOOKS.monster_combat(world);
    } else {
        HOOKS.monster_combat(world);
        do_autocombat(world);
    }
    for id in world.session_ids() {
        if !world.is_in_game(id) {
            continue;
        }
        if let Some(p) = player_mut(world, id) {
            p.settings.insert(PlayerSettings::CANCAST);
            if p.status2.contains(PlayerStatus2::ATTACKED) {
                p.status.insert(PlayerStatus::SPOTTED);
                p.status2.remove(PlayerStatus2::ATTACKED);
            } else {
                p.status.remove(PlayerStatus::SPOTTED);
            }
        }
    }
    players_first
}
