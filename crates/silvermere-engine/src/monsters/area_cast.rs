//! A monster casts a spell on the room.
//!
//! The players it reaches are [`crate::spells::area`]'s with a monster caster: everyone in
//! the room who does not resist, party or not, level or not (the count marks them and
//! never clears a mark). The players' resistance roll is the targeted cast's (half the
//! magic resistance held to 97, an item negating the spell); the strength gets no caster
//! bonus and is divided among the players for every kind but the "Full" ones. Each slot goes
//! through T-067's `apply_monster_cast_area` (the per-player element share, the
//! magic-resistance multiplier of "Inflict Damage on Armour", the kinds that hurt nobody),
//! and the engine shows the cast with [`monster_display_spell_success`] to every player an
//! instant slot reaches while no earlier slot of the cast has acted (`done`), kills and
//! knocks out, and shows each lasting add.
//!
//! Callers: a monster's cast hands over every kind but the single ones, and a cast by
//! spell number the area kinds; both are T-036's and T-052's (`combat::monster_cast` is
//! still a stub on main).

use silvermere_rules::Rng as _;
use silvermere_rules::spells::apply_monster::{
    MonsterCast, MonsterCastState, MonsterCaster, apply_monster_area_remove_pass,
    apply_monster_cast_area, monster_duration, roll_monster_amount,
};
use silvermere_rules::spells::resist::apply_element_resistance;
use silvermere_rules::spells::{
    EffectSlot, Outcome, SpellEffects, TargetKindExt, UserTarget, per_target_share,
};
use silvermere_types::enums::Ability;
use silvermere_types::flags::PlayerStatus2;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MonsterNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{Character, KnownMonster, Spell};

use super::spell_support::monster_display_spell_success;
use crate::combat::{self, user_has};
use crate::spells::area::{AreaTarget, monster_count_valid_targets, targets};
use crate::spells::cast::Caster;
use crate::world::World;

/// What an area cast comes to: nothing was cast (records missing, nobody to hit, or
/// a lasting spell nobody would take), the spell went off (or failed, or the monster was
/// short of energy), or it killed a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaCastResult {
    Nothing,
    Cast,
    Killed,
}

impl AreaCastResult {
    /// The result as a number, in variant order: 0, 1, 2.
    pub fn code(self) -> i32 {
        match self {
            Self::Nothing => 0,
            Self::Cast => 1,
            Self::Killed => 2,
        }
    }
}

/// The marked players of the monster's room, as the effect code sees them: their
/// characters taken out of the sessions for the call, with what the code reads around them.
struct Marked {
    ids: Vec<SessionId>,
    chars: Vec<Character>,
    anti_magic: Vec<bool>,
    poison_immune: Vec<bool>,
    element_pct: Vec<i32>,
}

impl Marked {
    /// The marked players, or only `only` when given (and still marked).
    fn take(
        world: &mut World,
        mon: ActiveMonsterId,
        room: RoomKey,
        spl: &Spell,
        only: Option<SessionId>,
    ) -> Self {
        let ids: Vec<SessionId> = targets(world, room, &Caster::Monster(mon), spl)
            .into_iter()
            .filter_map(|t| match t {
                AreaTarget::Player(id) if only.is_none_or(|o| o == id) => Some(id),
                _ => None,
            })
            .collect();
        let mut m = Self {
            ids: Vec::new(),
            chars: Vec::new(),
            anti_magic: Vec::new(),
            poison_immune: Vec::new(),
            element_pct: Vec::new(),
        };
        for id in ids {
            let Some(p) = combat::player(world, id) else {
                continue;
            };
            let anti = user_has(world, p, Ability::AntiMagic);
            let immune = user_has(world, p, Ability::PoisonImmune);
            let pct = crate::spells::cast::user_element_resist(world, p, spl);
            let Some(c) = world.session_mut(id).and_then(|s| s.character.take()) else {
                continue;
            };
            m.ids.push(id);
            m.chars.push(c);
            m.anti_magic.push(anti);
            m.poison_immune.push(immune);
            m.element_pct.push(pct);
        }
        m
    }

    fn put_back(self, world: &mut World) {
        for (id, c) in self.ids.into_iter().zip(self.chars) {
            if let Some(s) = world.session_mut(id) {
                s.character = Some(c);
            }
        }
    }
}

/// Runs `f` with the marked players (or only `only`) as `UserTarget`s and the casting
/// monster as the `MonsterCaster`, the characters back in their sessions afterwards.
#[allow(clippy::too_many_arguments)]
fn with_targets<R>(
    world: &mut World,
    mon: ActiveMonsterId,
    known: &KnownMonster,
    room: RoomKey,
    spl: &Spell,
    only: Option<SessionId>,
    f: impl FnOnce(
        &mut MonsterCaster<'_>,
        &mut [UserTarget<'_, '_>],
        &mut dyn silvermere_rules::Rng,
    ) -> R,
) -> Option<(Vec<SessionId>, Vec<i32>, R)> {
    let mut marked = Marked::take(world, mon, room, spl, only);
    let deathhp = world
        .config
        .death_hp
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let r = {
        let World {
            active_monsters,
            spells,
            items,
            rng,
            ..
        } = world;
        let monster = active_monsters.values_mut().find(|a| a.number == mon);
        match monster {
            None => None,
            Some(monster) => {
                let spells_f = |n| spells.get(&n);
                let items_f = |n| items.get(&n);
                let mut users: Vec<UserTarget<'_, '_>> = marked
                    .chars
                    .iter_mut()
                    .enumerate()
                    .map(|(i, character)| UserTarget {
                        character,
                        is_caster: false,
                        anti_magic: marked.anti_magic[i],
                        poison_immune: marked.poison_immune[i],
                        element_resist_pct: marked.element_pct[i],
                        deathhp,
                        arena_safe: false,
                        spells: &spells_f,
                        items: &items_f,
                    })
                    .collect();
                let mut caster = MonsterCaster { monster, known };
                Some(f(&mut caster, &mut users, rng))
            }
        }
    };
    let ids = marked.ids.clone();
    let pcts = marked.element_pct.clone();
    marked.put_back(world);
    r.map(|r| (ids, pcts, r))
}

fn firstname(world: &World, id: SessionId) -> String {
    combat::player(world, id).map_or(String::new(), |p| p.firstname.clone())
}

/// A monster's area cast. `slot` is the attack slot that casts (its accuracy the spell, its
/// minimum the cast chance, its energy the cost, its maximum the duration's level), `None`
/// for a cast from outside the attack table (100%, no energy). `level` raises the strength.
pub fn monster_area_cast(
    world: &mut World,
    mon: ActiveMonsterId,
    slot: Option<usize>,
    spellnum: SpellNo,
    level: i32,
) -> AreaCastResult {
    // the cast roll comes first, before anything is looked at
    let roll = world.rng.range(0, 100);
    let Some(m) = combat::monster(world, mon) else {
        return AreaCastResult::Nothing;
    };
    let room = RoomKey {
        map: m.map,
        room: m.room,
    };
    let Some(known) = world.monsters.get(&m.knownnum).cloned() else {
        return AreaCastResult::Nothing;
    };
    if world.room(room).is_none() {
        return AreaCastResult::Nothing;
    }
    let attack = slot.and_then(|s| known.attacks.get(s).copied());
    let (first, castpct, energy) = match attack {
        Some(a) => (a.accuracy, i32::from(a.min), i32::from(a.energy)),
        None => (spellnum.0, 100, 0),
    };
    let Some(spl) = world.spells.get(&SpellNo(first)).cloned() else {
        return AreaCastResult::Nothing;
    };
    let mut killed = false;
    // (unsigned compare)
    if u32::from(m.energy) >= energy as u32 {
        // (a player marked by an earlier cast who resists this one is reached but not
        // counted: the share is divided by the count of this cast's non-resisters)
        let nhit = monster_count_valid_targets(world, mon, &spl);
        if nhit == 0 {
            return AreaCastResult::Nothing;
        }
        // fetched again by `spellnum`
        let Some(spl) = world.spells.get(&spellnum).cloned() else {
            return AreaCastResult::Nothing;
        };
        if roll >= castpct {
            // a failed cast costs half the energy, at least 1
            if energy != 0
                && let Some(m) = combat::monster_mut(world, mon)
            {
                m.energy = m.energy.wrapping_sub((energy / 2).max(1) as u16);
            }
        } else {
            if let Some(m) = combat::monster_mut(world, mon) {
                m.energy = m.energy.wrapping_sub(energy as u16);
            }
            match cast_slots(world, mon, &known, room, &spl, slot, level, nhit) {
                None => return AreaCastResult::Nothing,
                Some(k) => killed = k,
            }
        }
    }
    // every marked player's stats are worked out again, wherever he is
    for id in world.session_ids() {
        if world.is_in_game(id)
            && combat::player(world, id)
                .is_some_and(|p| p.status2.contains(PlayerStatus2::AREATARGET))
        {
            crate::items::equip::recalc(world, id);
        }
    }
    if killed {
        AreaCastResult::Killed
    } else {
        AreaCastResult::Cast
    }
}

/// The successful cast. `None` when a lasting spell nobody took ended it (the cast then counts
/// as not made), else whether a player died.
#[allow(clippy::too_many_arguments)]
fn cast_slots(
    world: &mut World,
    mon: ActiveMonsterId,
    known: &KnownMonster,
    room: RoomKey,
    spl: &Spell,
    slot: Option<usize>,
    level: i32,
    nhit: i32,
) -> Option<bool> {
    let baseroll = roll_monster_amount(spl, level, &mut world.rng);
    let value = per_target_share(baseroll, spl.target, nhit);
    let duration = monster_duration(
        spl,
        slot.and_then(|s| known.attacks.get(s))
            .map(|a| i32::from(a.max)),
    );
    let effects = SpellEffects::from_record(spl);
    // Remove spell and Terminate Spell, one slot at a time
    for rs in effects.iter() {
        if !matches!(rs.ability, Ability::RemoveSpell | Ability::TermSpell) || rs.value == 0 {
            continue;
        }
        remove_pass(world, mon, known, room, spl, rs, baseroll);
    }
    let cast = MonsterCast {
        spell: spl,
        power: baseroll,
        share: value,
        duration,
    };
    let mut state = MonsterCastState::new();
    let mut killed = false;
    for s in effects.iter() {
        let done = state.added;
        let amount = s.amount(baseroll);
        let hurts = matches!(
            s.ability,
            Ability::DamageNoArmour | Ability::DamageArmour | Ability::StealHp
        );
        // An instant area effect on the players works one player at a time: his change,
        // his display, his kill or knock-out, then the next player, so each prompt shows the
        // figures of its own moment (T-100). Everything else goes through in one call.
        let one_by_one = spl.target.is_area()
            && !cast.base_lasting()
            && matches!(
                s.ability,
                Ability::DamageNoArmour
                    | Ability::DamageArmour
                    | Ability::StealHp
                    | Ability::Energy
                    | Ability::Heal
                    | Ability::HealMana
            );
        let order: Vec<Option<SessionId>> = if one_by_one {
            let Some((ids, _, ())) = with_targets(world, mon, known, room, spl, None, |_, _, _| ())
            else {
                return Some(killed);
            };
            if ids.is_empty() {
                vec![None]
            } else {
                ids.into_iter().map(Some).collect()
            }
        } else {
            vec![None]
        };
        for only in order {
            let r = with_targets(world, mon, known, room, spl, only, |caster, users, _| {
                apply_monster_cast_area(s, &cast, caster, users, &mut state)
            });
            let Some((ids, pcts, out)) = r else {
                return Some(killed);
            };
            match out.caster {
                Outcome::ClearItem(n) => {
                    crate::scripts::actions::erase_item_from_room(world, room, ItemNo(n));
                }
                // to nobody as "everyone", without setting done
                Outcome::Summon(n, _) => {
                    if !done {
                        monster_display_spell_success(world, None, spl, mon, "everyone", amount);
                    }
                    summon(world, room, n);
                }
                // the healed amount, or (sic) the share
                Outcome::Applied { amount: shown, .. } if !done => {
                    let name =
                        combat::monster(world, mon).map_or(String::new(), |m| m.name.clone());
                    monster_display_spell_success(world, None, spl, mon, &name, shown);
                }
                _ => {}
            }
            for (i, o) in out.players.iter().enumerate() {
                let Some(&id) = ids.get(i) else {
                    continue;
                };
                match *o {
                    Outcome::Applied {
                        amount: shown,
                        knocked_out,
                    } => {
                        if !done {
                            let name = firstname(world, id);
                            monster_display_spell_success(world, Some(id), spl, mon, &name, shown);
                        }
                        if hurts {
                            killed |= after_damage(world, id, room, knocked_out);
                        }
                    }
                    Outcome::StolenHp { amount: shown } => {
                        if !done {
                            let name = firstname(world, id);
                            monster_display_spell_success(world, Some(id), spl, mon, &name, shown);
                        }
                        killed |= after_damage(world, id, room, false);
                    }
                    // each add that took is shown, as `monster_add_cast_spell_to_user` does
                    Outcome::Lasting(so) if so.is_success() => {
                        let dose =
                            apply_element_resistance(value, pcts.get(i).copied().unwrap_or(0));
                        let name = firstname(world, id);
                        monster_display_spell_success(world, Some(id), spl, mon, &name, dose);
                    }
                    _ => {}
                }
            }
        }
        if state.refused {
            // the cast counts as not made here, with no stats pass
            return None;
        }
    }
    Some(killed)
}

/// One "Remove spell" or "Terminate Spell" slot of the pre-pass:
/// T-067's pass on the marked players; the first player who loses the spell is shown the
/// cast with the undivided roll (`done` is cleared after each slot); each ending is shown
/// and its chain cast, his stats worked out, and his mark taken off.
fn remove_pass(
    world: &mut World,
    mon: ActiveMonsterId,
    known: &KnownMonster,
    room: RoomKey,
    spl: &Spell,
    rs: &EffectSlot,
    roll: i32,
) {
    if !spl.target.is_area() {
        return;
    }
    let one = SpellEffects { slots: vec![*rs] };
    let r = with_targets(world, mon, known, room, spl, None, |_, users, _| {
        apply_monster_area_remove_pass(&one, spl, users)
    });
    let Some((ids, _, out)) = r else {
        return;
    };
    let mut done = false;
    for (i, o) in out.players.iter().enumerate() {
        let Some(&id) = ids.get(i) else {
            continue;
        };
        let Outcome::Ended { spells, poison } = o else {
            continue;
        };
        if spells.is_empty() {
            continue;
        }
        if !done {
            let name = firstname(world, id);
            monster_display_spell_success(world, Some(id), spl, mon, &name, roll);
        }
        done = true;
        crate::spells::cast::report_ended(world, id, spells, *poison);
    }
    for i in out.unmarked {
        if let Some(&id) = ids.get(i)
            && let Some(p) = combat::player_mut(world, id)
        {
            p.status2.remove(PlayerStatus2::AREATARGET);
        }
    }
}

/// The kill test of the damage slots: a death is shared out as a
/// kill by nobody of no monster, worth nothing; else a player just dropped falls.
fn after_damage(world: &mut World, id: SessionId, room: RoomKey, knocked_out: bool) -> bool {
    if combat::check_kill_user(world, id).died() {
        combat::kill_monster::distribute_experience(world, None, 0, Some(id), None, room);
        return true;
    }
    if knocked_out {
        combat::knock_out_user(world, id);
    }
    false
}

/// The summoned monster is generated forced, any generator, any level, and nothing is set
/// on it.
fn summon(world: &mut World, room: RoomKey, n: i32) {
    let req = super::generate::Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0x7FFF,
        owner: None,
        ally: false,
        force: true,
    };
    super::generate::generate_monster(world, room, req);
}

#[cfg(test)]
mod tests;
