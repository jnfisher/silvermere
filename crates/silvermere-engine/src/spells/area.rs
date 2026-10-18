//! Who an area spell reaches: the one target iteration for the three casters, a player
//! ([`count_valid_targets`] with [`valid_player`] and [`valid_monster`]), a monster
//! ([`monster_count_valid_targets`]) and a room (`medium_update_character` and the other
//! callers of `room_cast_on_user`, which cast on one player at a time).
//!
//! The players an area cast reaches are marked with the record's own flag
//! ([`PlayerStatus2::AREATARGET`]), not kept in a list, so every later step of the cast
//! sees the same marks. The mark's life:
//!
//! - a player's count marks each valid target that does not resist, leaves a valid target
//!   who resists as he was, and clears the mark of everyone else in the game;
//! - a monster's count marks each player in its room who does not resist and clears no mark
//!   at all;
//! - "Remove spell" and "Terminate Spell" take the mark off a player whose spell they
//!   ended;
//! - logging on clears `status2`.
//!
//! So a player marked by an earlier cast who resists this one is still reached by it, and
//! by a monster's cast until a player's count clears him (sic, kept; KB candidate). The
//! number that divides a "Divided" amount is the count of this cast's non-resisting targets
//! ([`AreaTargets::nplayers`]), not the number reached.
//!
//! [`targets`] reads the marks; [`count_valid_targets`] and [`monster_count_valid_targets`]
//! roll the resistances and set them.

use silvermere_rules::spells::apply_monster::player_resists_monster_cast;
use silvermere_rules::spells::resist::{item_negates, player_resists};
use silvermere_rules::spells::{
    MonsterCandidate, PlayerCandidate, TargetKindExt, ValidTarget, is_valid_monster_target,
    is_valid_target,
};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, RoomType, SpellType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId};
use silvermere_types::records::{ActiveMonster, Character, Spell};

use super::cast::{Caster, EVIL_TICKS};
use crate::combat::{
    self, EvilKind, add_evil_points, monster_ability, monster_has, room_of, user_has,
};
use crate::tell::tell_user;
use crate::world::World;

/// One target an area cast reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AreaTarget {
    Player(SessionId),
    Monster(ActiveMonsterId),
}

/// The result of a player's count ([`count_valid_targets`]), with the targets the cast then
/// reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AreaTargets {
    /// The players reached, in session order, and whether hitting each is evil: the valid
    /// and marked players of the caster's room (see the module notes).
    pub players: Vec<(SessionId, bool)>,
    /// The valid monsters of the caster's room, in room order, and whether each is evil.
    pub monsters: Vec<(ActiveMonsterId, bool)>,
    /// The valid players of this count who did not resist.
    pub nplayers: i32,
    /// Some target counted is evil to hit.
    pub evil: bool,
    /// Every evil target counted is hidden from the caster.
    pub only_hidden: bool,
    /// For `cast_no_target`: the cast is an evil act (an attack type by item or script, or
    /// Evil in Combat).
    pub evilspell: bool,
}

impl AreaTargets {
    /// The players and monsters counted, the count's result.
    pub fn ntargets(&self) -> i32 {
        self.nplayers + self.monsters.len() as i32
    }

    /// The players again after the marks changed ("Remove spell").
    pub fn refresh_players(&mut self, world: &World, caster: SessionId, spl: &Spell) {
        self.players = player_hits(world, caster, spl);
    }
}

fn plr(world: &World, id: SessionId) -> Option<&Character> {
    combat::player(world, id)
}

fn arena_safe(world: &World, caster: &Character) -> bool {
    world
        .room(room_of(caster))
        .is_some_and(|r| r.room_type == RoomType::Colliseum && world.display.arena_safe)
}

/// The facts the validity of a player target is decided on.
fn player_candidate(world: &World, caster: SessionId, tid: SessionId) -> Option<PlayerCandidate> {
    let c = plr(world, caster)?;
    let t = plr(world, tid)?;
    let cs = world.session(caster)?;
    let ts = world.session(tid)?;
    Some(PlayerCandidate {
        is_caster: caster == tid,
        same_room: c.room == t.room && c.map == t.map,
        leads_caster: cs.party.leader == Some(tid),
        led_by_caster: ts.party.leader == Some(caster),
        in_casters_group: cs.party.group.contains(&Some(tid)),
        evil: t.evil,
        // `should_give_evil` (T-038)
        attack_gives_evil: crate::evil::timers::should_give_evil(world, caster, tid)
            != crate::evil::timers::Give::None,
        pvp_range_ok: caster == tid || combat::attack::pvp_level_range_ok(world, caster, tid),
        arena_safe: arena_safe(world, c),
        sysop_debug: combat::debug_on(world, caster),
    })
}

/// The facts the validity of a monster target is decided on.
fn monster_candidate(
    world: &World,
    caster: &Character,
    spl: &Spell,
    m: &ActiveMonster,
) -> MonsterCandidate {
    let reqtohit = monster_ability(world, m, Ability::ReqToHit);
    MonsterCandidate {
        alignment: m.alignment,
        montype: m.montype,
        follows_caster: m.following.eq_ignore_ascii_case(&caster.firstname),
        charmed: m.flags.contains(ActiveMonsterFlags::CHARMED),
        friendly: m.friendly != 0,
        caster_evil: caster.evil,
        can_hit: !monster_has(world, m, Ability::ReqToHit)
            || silvermere_rules::spells::spell_can_hit_monster(reqtohit, spl),
    }
}

/// Whether a player caster's spell may hit a player, and whether that is evil.
pub fn valid_player(
    world: &World,
    caster: SessionId,
    tid: SessionId,
    spl: &Spell,
) -> Option<ValidTarget> {
    is_valid_target(spl, &player_candidate(world, caster, tid)?)
}

/// Whether a player caster's spell may hit an active monster, and whether that is evil.
pub fn valid_monster(
    world: &World,
    caster: SessionId,
    mon: ActiveMonsterId,
    spl: &Spell,
) -> Option<ValidTarget> {
    let c = plr(world, caster)?;
    let m = combat::monster(world, mon)?;
    is_valid_monster_target(spl, &monster_candidate(world, c, spl, m))
}

fn in_game_sorted(world: &World) -> Vec<SessionId> {
    let mut ids = world.session_ids();
    ids.sort();
    ids.retain(|&id| world.is_in_game(id));
    ids
}

fn marked(world: &World, id: SessionId) -> bool {
    plr(world, id).is_some_and(|p| p.status2.contains(PlayerStatus2::AREATARGET))
}

fn in_room(world: &World, id: SessionId, room: RoomKey) -> bool {
    plr(world, id).is_some_and(|p| room_of(p) == room)
}

/// The player loops of `cast_no_target`: every player in the game that the spell may hit,
/// that is marked, and that stands in the caster's room.
fn player_hits(world: &World, caster: SessionId, spl: &Spell) -> Vec<(SessionId, bool)> {
    let Some(room) = plr(world, caster).map(room_of) else {
        return Vec::new();
    };
    in_game_sorted(world)
        .into_iter()
        .filter(|&tid| marked(world, tid) && in_room(world, tid, room))
        .filter_map(|tid| valid_player(world, caster, tid, spl).map(|v| (tid, v.evil)))
        .collect()
}

/// The monster loops of `cast_no_target`: the valid monsters of the room, for the kinds
/// that reach monsters.
fn monster_hits(world: &World, caster: SessionId, spl: &Spell) -> Vec<(ActiveMonsterId, bool)> {
    if !spl.target.area_reaches_monsters() {
        return Vec::new();
    }
    let Some(room) = plr(world, caster).map(room_of) else {
        return Vec::new();
    };
    let Some(rs) = world.room_state.get(&room) else {
        return Vec::new();
    };
    rs.monsters
        .iter()
        .filter(|n| n.0 != 0)
        .filter_map(|&n| valid_monster(world, caster, n, spl).map(|v| (n, v.evil)))
        .collect()
}

/// Who an area cast by `caster` in `room` reaches now, as the casting engines' loops find
/// them after the count:
///
/// - a player: the players [`player_hits`] finds (valid, marked, in the room) and then the
///   valid monsters of the room for the kinds that reach monsters;
/// - a monster: every marked player in the game who stands in its room, party or not, valid
///   or not (`monster_cast_area`'s loops); never a monster;
/// - a room: every player in the game in the room, each cast on separately
///   (`medium_update_character`); never a monster (nothing in the module calls
///   `room_cast_on_monster`).
pub fn targets(world: &World, room: RoomKey, caster: &Caster, spell: &Spell) -> Vec<AreaTarget> {
    match *caster {
        Caster::Player(id) => {
            if plr(world, id).map(room_of) != Some(room) {
                return Vec::new();
            }
            let players = player_hits(world, id, spell)
                .into_iter()
                .map(|(t, _)| AreaTarget::Player(t));
            let monsters = monster_hits(world, id, spell)
                .into_iter()
                .map(|(m, _)| AreaTarget::Monster(m));
            players.chain(monsters).collect()
        }
        Caster::Monster(mon) => {
            let Some(m) = combat::monster(world, mon) else {
                return Vec::new();
            };
            let here = RoomKey {
                map: m.map,
                room: m.room,
            };
            if here != room {
                return Vec::new();
            }
            in_game_sorted(world)
                .into_iter()
                .filter(|&t| marked(world, t) && in_room(world, t, room))
                .map(AreaTarget::Player)
                .collect()
        }
        Caster::Room(_) => in_game_sorted(world)
            .into_iter()
            .filter(|&t| in_room(world, t, room))
            .map(AreaTarget::Player)
            .collect(),
    }
}

/// A player's area count: each valid player rolls his resistance (a worn item, or half his
/// magic resistance held to 98 when the spell may be resisted); the caster hears `%s
/// resists your spell!` for each who does (buffered); the others are counted and marked, a
/// visible evil one clearing [`AreaTargets::only_hidden`]; any but a General spell ends
/// resting and sneaking for every valid player; an invalid player loses his mark. Then the
/// valid monsters, for the kinds that reach them, are counted and flagged targeted.
pub fn count_valid_targets(world: &mut World, id: SessionId, spl: &Spell) -> AreaTargets {
    let mut t = AreaTargets {
        only_hidden: true,
        ..AreaTargets::default()
    };
    let Some(c) = plr(world, id).cloned() else {
        return t;
    };
    let sees_hidden = c.status.contains(PlayerStatus::SEEHIDDEN);
    for tid in in_game_sorted(world) {
        let Some(tp) = plr(world, tid) else {
            continue;
        };
        let Some(v) = valid_player(world, id, tid, spl) else {
            // not a target: clear any mark from an earlier count
            if let Some(tp) = combat::player_mut(world, tid) {
                tp.status2.remove(PlayerStatus2::AREATARGET);
            }
            continue;
        };
        let worn: Vec<_> = tp.worn.iter().filter_map(|n| world.items.get(n)).collect();
        let negated = item_negates(worn, spl.number);
        let anti_magic = user_has(world, tp, Ability::AntiMagic);
        let (mr, name, hidden) = (i32::from(tp.mr), tp.firstname.clone(), tp.hidden);
        let resisted = player_resists(spl, mr, anti_magic, negated, &mut world.rng);
        if resisted {
            // buffered: it goes out ahead of the caster's next line
            tell_user(world, id, Msg::CastResistsYourSpell { target: name });
        } else {
            // only a visible evil target clears `only_hidden`
            if v.evil {
                t.evil = true;
                if hidden == 0 || sees_hidden {
                    t.only_hidden = false;
                }
            }
            t.nplayers += 1;
            if let Some(tp) = combat::player_mut(world, tid) {
                tp.status2.insert(PlayerStatus2::AREATARGET);
            }
        }
        if spl.spell_type != SpellType::General
            && let Some(tp) = combat::player_mut(world, tid)
        {
            tp.status
                .remove(PlayerStatus::RESTING | PlayerStatus::SNEAKING);
        }
    }
    t.monsters = monster_hits(world, id, spl);
    for &(n, evil) in &t.monsters {
        // a monster is never hidden
        if evil {
            t.evil = true;
            t.only_hidden = false;
        }
        if let Some(m) = combat::monster_mut(world, n) {
            m.flags.insert(ActiveMonsterFlags::TARGETED);
        }
    }
    t.players = player_hits(world, id, spl);
    t
}

/// A monster's area count: every player in the game who stands in the monster's room,
/// whatever his party or his level, resists when a worn item negates the spell or (for a
/// spell that may be resisted) a 1-100 roll comes up at most half his magic resistance held
/// to 97; the others are marked and counted. Nobody's mark is cleared (sic). Returns the
/// number marked.
pub fn monster_count_valid_targets(world: &mut World, mon: ActiveMonsterId, spl: &Spell) -> i32 {
    let Some(room) = combat::monster(world, mon).map(|m| RoomKey {
        map: m.map,
        room: m.room,
    }) else {
        return 0;
    };
    let mut nhit = 0;
    for tid in in_game_sorted(world) {
        if !in_room(world, tid, room) {
            continue;
        }
        let Some(tp) = plr(world, tid) else {
            continue;
        };
        let worn: Vec<_> = tp.worn.iter().filter_map(|n| world.items.get(n)).collect();
        let negated = item_negates(worn, spl.number);
        let anti_magic = user_has(world, tp, Ability::AntiMagic);
        let mr = i32::from(tp.mr);
        if !player_resists_monster_cast(spl, mr, anti_magic, negated, &mut world.rng) {
            nhit += 1;
            if let Some(tp) = combat::player_mut(world, tid) {
                tp.status2.insert(PlayerStatus2::AREATARGET);
            }
        }
    }
    nhit
}

/// The evil the area cast earns, through `add_evil_points` (T-038). `true` when the caster
/// backed off at a warning.
pub fn add_evil_warnings_to_room(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    targets: &AreaTargets,
    mut evil: bool,
) -> bool {
    let pts = i32::from(silvermere_rules::spells::EVIL_SPELL_PTS);
    if evil && add_evil_points(world, id, None, pts, EVIL_TICKS, EvilKind::Attack) {
        return true;
    }
    for &(tid, tevil) in &targets.players {
        // the first evil hit charges the caster, once
        if tevil && !evil {
            if add_evil_points(world, id, None, pts, EVIL_TICKS, EvilKind::Attack) {
                return true;
            }
            evil = true;
        }
        // every area target is recorded as attacked by the caster
        add_evil_points(world, id, Some(tid), 0, EVIL_TICKS, EvilKind::Attack);
    }
    if spl.target.area_reaches_monsters() {
        for &(_, tevil) in &targets.monsters {
            if tevil && !evil {
                return add_evil_points(world, id, None, pts, EVIL_TICKS, EvilKind::Attack);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests;
