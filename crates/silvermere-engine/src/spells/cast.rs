//! The one casting path: [`cast`] with the target as a [`CastTarget`]. The four engines
//! ([`cast_no_target`], [`cast_user_target`], [`cast_monster_target`] and
//! [`cast_item_target`]) are its match arms; each keeps the original's cost, messages in
//! their order and auto-combat side effects, and they share the pieces (the success
//! roll, the charge, the failure lines, the shortage lines, the slot effects through
//! T-049's `rules::spells::apply_effect`, the messages through
//! [`super::messages::display_spell_success`]).
//!
//! Messages go straight into the sessions' output batches, as everywhere in the engine;
//! the [`CastResult`] says how the cast ended and what it asked of auto-combat.
//!
//! What a cast depends on comes in as arguments: whether the cast is auto-combat's is
//! [`CastOpts::autocombat`], the mode and manual flags are [`CastMode`], the "caster told"
//! and "room told" flags are a [`Told`] per cast (per attempt in the looping engines), the
//! teleport destination and the chain spell live in T-049's `CastState`. No copies of the
//! spell records and no current-user global.
//!
//! Where T-049's arithmetic differs from what the original shows, the engine
//! follows the original (each place says so): the healing amount shown is the roll, not
//! the gain; an instant spell with a Component, Guarded or unknown slot is silly; a lasting
//! area spell goes to every target through [`add_duration_spell_to_room`]; Steal Hit
//! Points pays the caster even when the target goes down.

use silvermere_rules::Rng as _;
use silvermere_rules::abilities::spell_has_ability;
use silvermere_rules::spells::apply::{
    Cast, CastPath, CastState, EndedSpell, PoisonCut, gain_stolen_hp, is_good_aligned,
};
use silvermere_rules::spells::resist::{
    apply_element_resistance, item_negates, monster_magic_resistance, monster_resists_area,
    monster_resists_single, spell_immune,
};
use silvermere_rules::spells::{
    CasterStats, CharacterSlots, DIFF_NOFAIL, EVIL_SPELL_PTS, EffectSlot, ItemTarget, MAXCHANCE,
    MonsterTarget, Outcome, SlotOutcome, SpellEffect, SpellEffects, Target, TargetKindExt,
    UserTarget, add_spell_to_character, add_spell_to_monster, apply_effect, is_attack_type,
    per_target_share, roll_amount,
};
use silvermere_text::Msg;
use silvermere_types::enums::{
    Ability, Alignment, Element, Magery, MonsterType, RoomType, SpellTarget, SpellType,
};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerSettings, PlayerStatus, PlayerStatus2, RoomFlags,
};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{Character, Spell};
use silvermere_types::sizes::NDIRS;

use super::active::end_player_spell;
use super::area::{AreaTargets, add_evil_warnings_to_room, count_valid_targets};
use super::messages::{Told, display_spell_success};
use crate::combat::{
    self, AutoTarget, EvilKind, add_evil_points, check_kill_monster, check_kill_user,
    display_autocombat_broken, kill_autocombat, kill_autocombat_against_monster, monster_ability,
    monster_has, room_of, user_ability, user_has,
};
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Most attempts of one cast in the looping engines.
pub const MAXTRIES: u32 = 20;
/// The evil timer an evil cast sets on its targets.
pub const EVIL_TICKS: i32 = 11;

/// What a spell is aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastTarget {
    /// Nothing named: the caster himself or the area (`cast_no_target`).
    None,
    /// A player (`cast_user_target`).
    User(SessionId),
    /// An active monster (`cast_monster_target`).
    Monster(ActiveMonsterId),
    /// A carried item (`cast_item_target`).
    Item(ItemNo),
}

/// Who casts. Only players cast through here so far; monsters (T-052) and rooms (T-051)
/// reach their own engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caster {
    Player(SessionId),
    Monster(ActiveMonsterId),
    Room(RoomKey),
}

/// How the spell came to be cast: the mode (`cast_no_target`) and the manual flag (the
/// other engines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CastMode {
    /// The CAST command and auto-combat (not manual).
    #[default]
    Cast,
    /// An item or a script (manual): no success roll, its own magery test.
    Item,
    /// A "Cast on ending" spell; sneaking and resting are not touched.
    Chain,
}

/// How a cast is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CastOpts {
    pub mode: CastMode,
    /// The cast is auto-combat's round.
    pub autocombat: bool,
    /// `cast_user_target`'s `flag`: the target and the room are told when combat starts.
    pub announce: bool,
}

/// How a cast ended (the engines' return values, with the reason kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastOutcome {
    /// Refused before anything was spent, or short of energy or mana (0).
    Refused,
    /// The spell was cast (or a failed roll or a resist was charged; 1).
    Cast,
    /// An instant attack spell cast by hand started auto-combat (1).
    Engaged,
    /// The target died (`cast_user_target`'s 2).
    KilledTarget,
}

impl CastOutcome {
    /// The result as a number: 0 refused, 1 cast, 2 the target killed.
    pub fn code(self) -> i32 {
        match self {
            Self::Refused => 0,
            Self::Cast | Self::Engaged => 1,
            Self::KilledTarget => 2,
        }
    }
}

/// What auto-combat was asked to take on: the caster fights on with the spell each round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Engage {
    pub target: Option<AutoTarget>,
    pub spell: SpellNo,
}

/// The result of [`cast`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastResult {
    pub outcome: CastOutcome,
    /// The last auto-combat engage the cast made, if any.
    pub engaged: Option<Engage>,
}

impl CastResult {
    fn new(outcome: CastOutcome) -> Self {
        Self {
            outcome,
            engaged: None,
        }
    }
}

/// The one casting entry point. A player casts `spell` at `target`; monster and room
/// casters are T-052's and T-051's (refused here).
pub fn cast(
    world: &mut World,
    caster: Caster,
    spell: SpellNo,
    target: CastTarget,
    opts: CastOpts,
) -> CastResult {
    let Caster::Player(id) = caster else {
        // TODO(T-051, T-052): monster and room casters.
        return CastResult::new(CastOutcome::Refused);
    };
    match target {
        CastTarget::None => cast_no_target(world, id, spell, opts),
        CastTarget::User(t) => cast_user_target(world, id, spell, t, opts),
        CastTarget::Monster(m) => cast_monster_target(world, id, spell, m, opts),
        CastTarget::Item(i) => cast_item_target(world, id, spell, i, opts),
    }
}

/// The auto-combat round's cast (T-034's `RoundHooks::cast`): at the player or monster
/// target, or with no target, as auto-combat's round.
pub fn autocombat_cast(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    target: Option<AutoTarget>,
) -> CastResult {
    let opts = CastOpts {
        mode: CastMode::Cast,
        autocombat: true,
        announce: true,
    };
    let target = match target {
        None => CastTarget::None,
        Some(AutoTarget::Player(t)) => CastTarget::User(t),
        Some(AutoTarget::Monster(m)) => CastTarget::Monster(m),
    };
    let r = cast(world, Caster::Player(id), spell, target, opts);
    apply_engage(world, id, &r);
    r
}

/// CAST alone repeating the auto-combat spell: the by-hand cast (not auto-combat's), so an
/// instant attack spell prints `*Combat Engaged*` and starts the fight again rather than
/// casting at once.
pub fn restart_cast(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    target: Option<AutoTarget>,
) -> CastResult {
    let opts = CastOpts {
        mode: CastMode::Cast,
        autocombat: false,
        announce: true,
    };
    let target = match target {
        None => CastTarget::None,
        Some(AutoTarget::Player(t)) => CastTarget::User(t),
        Some(AutoTarget::Monster(m)) => CastTarget::Monster(m),
    };
    let r = cast(world, Caster::Player(id), spell, target, opts);
    apply_engage(world, id, &r);
    r
}

/// Hands an engage the cast asked for to auto-combat (`engage_autocombat`, T-034).
pub fn apply_engage(world: &mut World, id: SessionId, r: &CastResult) {
    if let Some(e) = r.engaged {
        combat::engage_autocombat_spell(world, id, e.target, e.spell);
    }
}

// ---------------------------------------------------------------------------------------
// Shared pieces

fn plr(world: &World, id: SessionId) -> Option<&Character> {
    combat::player(world, id)
}

fn plr_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    combat::player_mut(world, id)
}

fn is_kai(world: &World, plr: &Character) -> bool {
    world
        .classes
        .get(&plr.class)
        .is_some_and(|c| c.magery == Magery::Kai)
}

/// The cast chance is `spellcast` plus the spell's `diff`, at most [`MAXCHANCE`]; the roll
/// succeeds when `diff` is at least [`DIFF_NOFAIL`] or the chance is above it.
pub fn roll_succeeds(plr: &Character, spl: &Spell, roll: i32) -> bool {
    let chance = (i32::from(plr.spellcast) + i32::from(spl.diff)).min(MAXCHANCE);
    spl.diff >= DIFF_NOFAIL || chance > roll
}

fn charge(world: &mut World, id: SessionId, energy: i32, mana: i32) {
    if let Some(p) = plr_mut(world, id) {
        p.energy = (i32::from(p.energy) - energy).clamp(0, i32::from(u16::MAX)) as u16;
        p.mana = (i32::from(p.mana) - mana) as i16;
    }
}

/// The full energy and half the mana of a failed or resisted cast.
fn charge_failure(world: &mut World, id: SessionId, spl: &Spell) {
    charge(
        world,
        id,
        i32::from(spl.energy),
        (i32::from(spl.mana) / 2).max(0),
    );
}

/// `You attempt to cast %s, but fail.` with the prompt, and the room's line.
fn fail_lines(world: &mut World, id: SessionId, spl: &Spell) {
    let Some(p) = plr(world, id) else {
        return;
    };
    let who = p.firstname.clone();
    let room = room_of(p);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::CastFail {
            spell: spl.name.clone(),
        });
        s.push_prompt();
    }
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Medium,
        TellBlind::All,
        true,
        None,
        &Msg::CastFailRoom {
            who,
            spell: spl.name.clone(),
        },
    );
}

fn round_word(kai: bool) -> String {
    if kai {
        "invoked a power"
    } else {
        "cast a spell"
    }
    .to_owned()
}

/// The shortage lines of a cast by hand: energy first (a cast this round), then mana (or
/// kai), then level when `level` is checked.
fn short_lines(world: &mut World, id: SessionId, spl: &Spell, level: bool) {
    let Some(p) = plr(world, id) else {
        return;
    };
    let kai = is_kai(world, p);
    let msg = if p.energy < spl.energy {
        Some(Msg::CastShortEnergy {
            what: round_word(kai),
        })
    } else if p.mana < spl.mana {
        Some(Msg::CastShortMana {
            what: if kai {
                "kai to invoke that power"
            } else {
                "mana to cast that spell"
            }
            .to_owned(),
        })
    } else if level && p.level < spl.level {
        Some(Msg::CastShortLevel)
    } else {
        None
    };
    if let Some(m) = msg {
        tell_user(world, id, m);
    }
}

/// Auto-combat's shortage: the energy is spent when there is enough, and emptied when the
/// mana is short.
fn autocombat_short(world: &mut World, id: SessionId, spl: &Spell) {
    if let Some(p) = plr_mut(world, id) {
        if p.energy >= spl.energy {
            p.energy -= spl.energy;
        }
        if p.mana < spl.mana {
            p.energy = 0;
        }
    }
}

/// The caster is told the spell makes no sense; the internal error goes to the log.
fn silly(world: &mut World, id: SessionId, spl: &Spell, kind: &str) {
    tell_user(world, id, Msg::CastSilly);
    tracing::warn!(spell = %spl.name, kind, "Silly Spell Cast");
}

/// The protected-room refusal: the room's own reason or the feeling of guilt, after a
/// cleared line.
fn guilt(world: &mut World, id: SessionId, rm: &silvermere_types::records::Room) {
    tell_user(world, id, Msg::CombatGuiltClr);
    if rm.protmsg.0 != 0 {
        crate::display::long_text::display_long_text(world, id, rm.protmsg, None);
    } else {
        tell_user(world, id, Msg::CombatGuilt);
    }
}

fn protected_refuses(world: &World, id: SessionId, rm: &silvermere_types::records::Room) -> bool {
    !world.config.tournament_play
        && rm.flags.contains(RoomFlags::PROTECTED)
        && !combat::debug_on(world, id)
}

fn is_attack_spell(spl: &Spell) -> bool {
    is_attack_type(spl.spell_type)
}

fn arena_safe_room(world: &World, rm: &silvermere_types::records::Room) -> bool {
    rm.room_type == RoomType::Colliseum && world.display.arena_safe
}

/// One charge of the first carried copy of the item; without one the cast is charged in
/// full and fails with the caster's and the room's lines.
fn use_spell_component(world: &mut World, id: SessionId, spl: &Spell, item: ItemNo) -> bool {
    let Some(p) = plr(world, id) else {
        return false;
    };
    let found = p
        .items
        .iter()
        .zip(p.itemuses.iter())
        .find(|(n, uses)| **n == item && **uses != 0 && world.items.contains_key(n))
        .map(|(_, uses)| *uses);
    if let Some(uses) = found {
        crate::items::inventory::deduct_item_charge(world, id, item, uses);
        return true;
    }
    charge(world, id, i32::from(spl.energy), i32::from(spl.mana).max(0));
    let Some(p) = plr(world, id) else {
        return false;
    };
    let who = p.firstname.clone();
    let room = room_of(p);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::CastNoComponent);
        s.push_prompt();
    }
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Medium,
        TellBlind::All,
        true,
        None,
        &Msg::CastFailRoom {
            who,
            spell: spl.name.clone(),
        },
    );
    false
}

/// What the effect code needs to know about the caster, owned so the caster's record can be
/// borrowed as a target.
struct CasterInfo {
    name: String,
    level: u16,
    spelldamage_pct: i32,
    spellduration_pct: i32,
    spellcast: i16,
}

impl CasterInfo {
    fn of(world: &World, p: &Character) -> Self {
        Self {
            name: p.firstname.clone(),
            level: p.level,
            spelldamage_pct: user_ability(world, p, Ability::SpellDamage),
            spellduration_pct: user_ability(world, p, Ability::SpellDuration),
            spellcast: p.spellcast,
        }
    }

    fn stats(&self) -> CasterStats<'_> {
        CasterStats {
            firstname: &self.name,
            level: self.level,
            spelldamage_pct: self.spelldamage_pct,
            spellduration_pct: self.spellduration_pct,
            spellcast: self.spellcast,
        }
    }
}

/// The element resistance of a player to an attack spell.
pub(crate) fn user_element_resist(world: &World, p: &Character, spl: &Spell) -> i32 {
    match element_ability(spl) {
        Some(a) => user_ability(world, p, a),
        None => 0,
    }
}

fn element_ability(spl: &Spell) -> Option<Ability> {
    if !is_attack_type(spl.spell_type) {
        return None;
    }
    Some(match spl.element {
        Element::Cold => Ability::ColdDefence,
        Element::Hot => Ability::HotDefence,
        Element::Stone => Ability::ResistStone,
        Element::Poison => Ability::PoisonImmune,
        Element::Water => Ability::ResistWater,
        Element::Lightning => Ability::ResistLightning,
        _ => return None,
    })
}

pub(super) fn monster_element_resist(
    world: &World,
    m: &silvermere_types::records::ActiveMonster,
    spl: &Spell,
) -> i32 {
    match element_ability(spl) {
        Some(a) => monster_ability(world, m, a),
        None => 0,
    }
}

/// Applies one slot to a player target with T-049's `apply_effect`.
#[allow(clippy::too_many_arguments)]
fn apply_to_user(
    world: &mut World,
    tid: SessionId,
    caster: &CasterInfo,
    cast_id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    path: CastPath,
    power: i32,
    state: &mut CastState,
) -> Outcome {
    let Some(t) = plr(world, tid) else {
        return Outcome::NoEffect;
    };
    let anti_magic = user_has(world, t, Ability::AntiMagic);
    let poison_immune = user_has(world, t, Ability::PoisonImmune);
    let element_resist_pct = user_element_resist(world, t, spl);
    let arena_safe = plr(world, cast_id)
        .and_then(|c| world.room(room_of(c)))
        .is_some_and(|rm| arena_safe_room(world, rm));
    let deathhp = world
        .config
        .death_hp
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let stats = caster.stats();
    let cast = Cast {
        spell: spl,
        caster: Some(&stats),
        path,
        power,
    };
    let World {
        sessions,
        spells,
        items,
        rng,
        ..
    } = world;
    let Some(character) = sessions.get_mut(&tid).and_then(|s| s.character.as_mut()) else {
        return Outcome::NoEffect;
    };
    let spells_f = |n| spells.get(&n);
    let items_f = |n| items.get(&n);
    let mut target = Target::User(UserTarget {
        character,
        is_caster: tid == cast_id,
        anti_magic,
        poison_immune,
        element_resist_pct,
        deathhp,
        arena_safe,
        spells: &spells_f,
        items: &items_f,
    });
    apply_effect(slot, &cast, &mut target, state, rng)
}

/// Applies one slot to a monster target with T-049's `apply_effect`.
#[allow(clippy::too_many_arguments)]
fn apply_to_monster(
    world: &mut World,
    mon: ActiveMonsterId,
    caster: &CasterInfo,
    spl: &Spell,
    slot: &EffectSlot,
    path: CastPath,
    power: i32,
    magic_resistance: i32,
    state: &mut CastState,
) -> Outcome {
    let Some(m) = combat::monster(world, mon) else {
        return Outcome::NoEffect;
    };
    let anti_magic = monster_has(world, m, Ability::AntiMagic);
    let poison_immune = monster_has(world, m, Ability::PoisonImmune);
    let element_resist_pct = monster_element_resist(world, m, spl);
    let knownnum = m.knownnum;
    let stats = caster.stats();
    let cast = Cast {
        spell: spl,
        caster: Some(&stats),
        path,
        power,
    };
    // the monster into the cache (T-086)
    world.active_monsters.touch(mon);
    let World {
        active_monsters,
        monsters,
        rng,
        ..
    } = world;
    let Some(known) = monsters.get(&knownnum) else {
        return Outcome::NoEffect;
    };
    let Some(monster) = active_monsters.values_mut().find(|a| a.number == mon) else {
        return Outcome::NoEffect;
    };
    let mut target = Target::Monster(MonsterTarget {
        monster,
        known,
        magic_resistance,
        anti_magic,
        poison_immune,
        element_resist_pct,
    });
    apply_effect(slot, &cast, &mut target, state, rng)
}

/// The effects that come out as any unlisted ability (silly when instant, a slot entry
/// when lasting), as in the original: T-049 maps Component, Guarded and unknown abilities
/// to "no effect".
fn takes_default_branch(effect: SpellEffect) -> bool {
    matches!(
        effect,
        SpellEffect::Component | SpellEffect::Guarded | SpellEffect::Unknown(_)
    )
}

/// A lasting add on a player for the default branch of the effects T-049 does not add (see
/// [`takes_default_branch`]).
fn add_lasting_user(
    world: &mut World,
    tid: SessionId,
    caster: &CasterInfo,
    spl: &Spell,
    power: i32,
) -> SlotOutcome {
    let stats = caster.stats();
    let rounds = {
        let World { rng, .. } = world;
        silvermere_rules::spells::duration(
            spl,
            Some(silvermere_rules::spells::CasterLevel {
                level: stats.level,
                spellduration_pct: stats.spellduration_pct,
            }),
            rng,
        )
    };
    let Some(c) = plr_mut(world, tid) else {
        return SlotOutcome::Full;
    };
    add_spell_to_character(
        CharacterSlots {
            spells: &mut c.spells,
            spellvals: &mut c.spellvals,
            spellrnds: &mut c.spellrnds,
        },
        spl.number,
        rounds,
        power,
        true,
    )
}

/// The messages of a lasting add on a player: the success lines unless the target is
/// hidden; the failure line once per cast.
fn lasting_user_lines(
    world: &mut World,
    told: &mut Told,
    caster: SessionId,
    tid: SessionId,
    spl: &Spell,
    power: i32,
    out: SlotOutcome,
) {
    if out.is_success() {
        let Some(t) = plr(world, tid) else {
            return;
        };
        if t.hidden == 0 {
            let name = t.firstname.clone();
            display_spell_success(world, told, caster, Some(tid), spl, &name, power);
        }
    } else if !told.caster
        && let Some(s) = world.session_mut(caster)
    {
        s.output.push(Msg::CastFail {
            spell: spl.name.clone(),
        });
        s.push_prompt();
    }
}

/// The lines of a lasting add on a monster.
fn lasting_monster_lines(
    world: &mut World,
    told: &mut Told,
    caster: SessionId,
    name: &str,
    spl: &Spell,
    power: i32,
    out: SlotOutcome,
) {
    if out.is_success() {
        display_spell_success(world, told, caster, None, spl, name, power);
    } else if !told.caster
        && let Some(s) = world.session_mut(caster)
    {
        s.output.push(Msg::CastFail {
            spell: spl.name.clone(),
        });
        s.push_prompt();
    }
}

/// The endings of spells a Freedom, Dispel Magic, Remove spell or Cure Poison took off a
/// player, one after the other: each runs [`end_player_spell`] (its ending line and prompt,
/// the undo, its chain cast, the stats), so each prompt shows
/// the figures before that spell's undo (T-100). Then the stats once more.
pub(crate) fn report_ended(
    world: &mut World,
    tid: SessionId,
    spells: &[EndedSpell],
    poison: Option<PoisonCut>,
) {
    for e in spells {
        if let Some(spl) = world.spells.get(&e.number).cloned() {
            end_player_spell(world, tid, &spl, e.power, e.chain);
        }
    }
    // "Cure Poison": the level falls after the poison spell's ending
    if let Some(cut) = poison
        && let Some(p) = plr_mut(world, tid)
    {
        silvermere_rules::spells::cut_poison(p, cut);
    }
    crate::items::equip::recalc(world, tid);
}

/// The kill test after a player was hurt by a spell: `check_kill_user` (dead: player kills
/// pay no experience here) or the knock-out.
fn after_user_damage(world: &mut World, tid: SessionId, knocked_out: bool) -> bool {
    if check_kill_user(world, tid).died() {
        return true;
    }
    if knocked_out {
        combat::knock_out_user(world, tid);
    }
    false
}

/// The kill test after a monster was hurt by a spell: the kill pays `exp*level` and ends
/// everyone's auto-combat against it.
fn after_monster_damage(world: &mut World, id: SessionId, mon: ActiveMonsterId) -> bool {
    let Some(m) = combat::monster(world, mon) else {
        return false;
    };
    let exp = world
        .monsters
        .get(&m.knownnum)
        .map_or(1, |k| (k.exp.max(0) as u32).saturating_mul(k.level));
    let key = RoomKey {
        map: m.map,
        room: m.room,
    };
    if check_kill_monster(world, mon, Some(id)) {
        combat::kill_monster::distribute_experience(world, Some(id), exp, None, Some(mon), key);
        kill_autocombat_against_monster(world, mon);
        return true;
    }
    false
}

/// `remove_armour` for the worn slots a Remove Curse took off.
pub(crate) fn strip_worn(world: &mut World, tid: SessionId, slots: &[usize]) {
    for &i in slots {
        let Some(name) = plr(world, tid)
            .and_then(|p| p.worn.get(i).copied())
            .and_then(|n| world.items.get(&n))
            .map(|itm| itm.name.clone())
        else {
            continue;
        };
        crate::items::equip::remove_armour(world, tid, &name, true);
    }
}

/// Each door or gate opens by force with `chance` plus `exitpar2s` in 100 (the chance held
/// to 110), else `Your spell fails to bash one of the exits!`. Returns the doors opened.
fn bash_room_exits(world: &mut World, id: SessionId, key: RoomKey, chance: i32) -> u32 {
    let chance = chance.min(110);
    let Some(rm) = world.room(key).cloned() else {
        return 0;
    };
    let mut opened = 0;
    for (i, exit) in rm.exits.iter().enumerate().take(NDIRS) {
        use silvermere_types::enums::ExitType;
        if exit.target.0 == 0 || !matches!(exit.kind, ExitType::Door | ExitType::Gate) {
            continue;
        }
        let need = chance + i32::from(exit.par2);
        if world.rng.range(0, 100) < need {
            opened += 1;
            if let Ok(dir) = silvermere_types::enums::Direction::COMPASS
                .get(i)
                .copied()
                .ok_or(())
            {
                crate::movement::doors::open_door_by_force(world, key, dir, id);
            }
        } else {
            tell_user(world, id, Msg::CastBashFail);
        }
    }
    opened
}

/// The teleport after the slots: Full Party takes the caster's party along; anything else
/// moves the caster alone, whose followers stop following him.
pub(crate) fn teleport(world: &mut World, mover: SessionId, dest: RoomKey, party: bool) {
    if world.room(dest).is_none() {
        // a map that is not loaded is not activated
        return;
    }
    if let Some(s) = world.session_mut(mover) {
        s.party.dragging = None;
    }
    if party {
        crate::movement::move_user::set_user_location(world, mover, dest, false);
        let (leader, members) = world
            .session(mover)
            .map(|s| (s.party.leader, s.party.members().collect::<Vec<_>>()))
            .unwrap_or_default();
        for m in members {
            if !world.is_in_game(m) {
                continue;
            }
            let mleader = world.session(m).and_then(|s| s.party.leader);
            let together = mleader == leader || mleader == Some(mover) || leader == Some(m);
            if together && !(leader.is_none() && mleader.is_none()) {
                crate::movement::move_user::set_user_location(world, m, dest, false);
            }
        }
    } else {
        // he stops leading: his followers stop, his list is cleared
        crate::party::stop_following_all(world, mover);
        crate::movement::move_user::set_user_location(world, mover, dest, false);
    }
}

fn chain_spell(world: &mut World, state: &CastState) -> Option<SpellNo> {
    let next = state.chain?;
    if next == SpellNo(0) {
        return None;
    }
    (world.rng.range(0, 100) < state.chain_chance).then_some(next)
}

// ---------------------------------------------------------------------------------------
// cast_no_target

/// The lasting area spell goes on every marked player (the share less his element
/// resistance, renewed if active) and, for the kinds that reach them, every valid monster
/// (less its element defence).
fn add_duration_spell_to_room(
    world: &mut World,
    told: &mut Told,
    id: SessionId,
    caster: &CasterInfo,
    spl: &Spell,
    power: i32,
    targets: &AreaTargets,
) {
    for &(tid, _) in &targets.players {
        let Some(tp) = plr(world, tid) else {
            continue;
        };
        let dose = apply_element_resistance(power, user_element_resist(world, tp, spl));
        if combat::debug_on(world, id) {
            tracing::debug!(target = %tp.firstname, "Adding spell");
        }
        let out = add_lasting_user(world, tid, caster, spl, dose);
        lasting_user_lines(world, told, id, tid, spl, dose, out);
    }
    if spl.target.area_reaches_monsters() {
        for &(mon, _) in &targets.monsters {
            let Some(m) = combat::monster(world, mon) else {
                continue;
            };
            let dose = apply_element_resistance(power, monster_element_resist(world, m, spl));
            let name = m.name.clone();
            let rounds = silvermere_rules::spells::duration(
                spl,
                Some(silvermere_rules::spells::CasterLevel {
                    level: caster.level,
                    spellduration_pct: caster.spellduration_pct,
                }),
                &mut world.rng,
            );
            let Some(m) = combat::monster_mut(world, mon) else {
                continue;
            };
            let out = add_spell_to_monster(&mut m.spells, spl.number, rounds, dose);
            lasting_monster_lines(world, told, id, &name, spl, dose, out);
        }
    }
}

/// A spell on the caster himself (Self, Self or Single, Any) or on the area (the Divided
/// and Full kinds).
pub fn cast_no_target(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    opts: CastOpts,
) -> CastResult {
    let mut res = CastResult::new(CastOutcome::Refused);
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return res;
    };
    if plr(world, id).is_none() {
        return res;
    }
    let by_hand = opts.mode == CastMode::Cast;
    let engage_none = |res: &mut CastResult| {
        res.engaged = Some(Engage {
            target: None,
            spell: spl.number,
        });
    };
    // confusion
    if by_hand && super::active::check_confusion(world, id) {
        if opts.autocombat {
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            engage_none(&mut res);
        }
        return res;
    }
    let Some(p) = plr(world, id) else {
        return res;
    };
    // knocked out or afraid
    if by_hand && (p.hp <= 0 || p.status2.contains(PlayerStatus2::AFRAID)) {
        if opts.autocombat {
            display_autocombat_broken(world, id);
        }
        return res;
    }
    let mut told = Told::default();
    // Magebind
    if by_hand && user_has(world, p, Ability::Magebind) {
        tell_user(world, id, Msg::CastSpellFails);
        if opts.autocombat {
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            engage_none(&mut res);
        }
        return res;
    }
    let Some(p) = plr(world, id) else {
        return res;
    };
    let Some(cls) = world.classes.get(&p.class).cloned() else {
        return res;
    };
    let kai = cls.magery == Magery::Kai;
    // Kai Bind
    if kai && by_hand && user_has(world, p, Ability::KaiBind) {
        tell_user(world, id, Msg::CastPowerFails);
        if opts.autocombat {
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            engage_none(&mut res);
        }
        return res;
    }
    let key = room_of(p);
    let Some(rm) = world.room(key).cloned() else {
        return res;
    };
    // an attack in a protected room
    if (is_attack_spell(&spl) || spell_has_ability(&spl, Ability::EvilInCombat))
        && protected_refuses(world, id, &rm)
    {
        guilt(world, id, &rm);
        if let Some(p) = plr_mut(world, id)
            && p.energy >= spl.energy
        {
            p.energy -= spl.energy;
        }
        return res;
    }
    // only the self and area kinds
    let kind = spl.target;
    if !(kind.is_self() || kind.is_area()) {
        tell_user(world, id, Msg::CastNeedTarget);
        return res;
    }
    if opts.mode != CastMode::Chain
        && let Some(p) = plr_mut(world, id)
    {
        p.status
            .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
        p.hidden = 0;
    }
    let mut targets = count_valid_targets(world, id, &spl);
    let nplayers = targets.nplayers;
    let ntargets = targets.ntargets();
    if ntargets == 0 {
        tell_user(world, id, Msg::CastNoEffectRoom);
        return res;
    }
    let arena = arena_safe_room(world, &rm);
    let evil_area = !arena && targets.evil && !targets.only_hidden;
    let mut success = false;
    if opts.mode != CastMode::Cast {
        // an item, a script or a chain: the magery test, no roll
        if spl.magery != Magery::None && (cls.magery != spl.magery || cls.magelvl < spl.magelvl) {
            return res;
        }
        success = true;
        if is_attack_spell(&spl) && evil_area {
            targets.evilspell = true;
            if add_evil_warnings_to_room(world, id, &spl, &targets, true) {
                return res;
            }
        }
    } else if is_attack_spell(&spl) {
        let engage_now = !opts.autocombat && spl.duration == 0;
        if evil_area && add_evil_warnings_to_room(world, id, &spl, &targets, engage_now) {
            return res;
        }
        if engage_now {
            let who = plr(world, id)
                .map(|p| p.firstname.clone())
                .unwrap_or_default();
            tell_user(world, id, Msg::CombatEngaged);
            tell_room(
                world,
                key,
                [Some(id), None],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &Msg::CastAttacksRoom { who },
            );
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            res.outcome = CastOutcome::Engaged;
            engage_none(&mut res);
            return res;
        }
    }
    if kai {
        success = true;
    }
    // energy, mana and level
    let Some(p) = plr(world, id) else {
        return res;
    };
    if !(p.energy >= spl.energy && p.mana >= spl.mana && p.level >= spl.level) {
        if opts.autocombat {
            autocombat_short(world, id, &spl);
            engage_none(&mut res);
        } else {
            short_lines(world, id, &spl, true);
        }
        return res;
    }
    let roll = world.rng.range(0, 100);
    // pass 1: components and Evil in Combat
    for (ability, value) in spl.abilities.iter().copied() {
        match ability {
            Ability::Component
                if !use_spell_component(world, id, &spl, ItemNo(i32::from(value))) =>
            {
                return res;
            }
            Ability::EvilInCombat => {
                targets.evilspell = true;
                if evil_area && add_evil_warnings_to_room(world, id, &spl, &targets, true) {
                    return res;
                }
            }
            _ => {}
        }
    }
    // the roll
    if by_hand {
        let Some(p) = plr_mut(world, id) else {
            return res;
        };
        if spl.spell_type == SpellType::General {
            if p.settings.contains(PlayerSettings::CANCAST) {
                p.settings.remove(PlayerSettings::CANCAST);
                let p = p.clone();
                if roll_succeeds(&p, &spl, roll) {
                    success = true;
                }
            } else {
                tell_user(
                    world,
                    id,
                    Msg::CastAlreadyThisRound {
                        what: round_word(kai),
                    },
                );
                return res;
            }
        } else if roll_succeeds(&p.clone(), &spl, roll) {
            success = true;
        }
    }
    res.outcome = CastOutcome::Cast;
    let mut state = CastState::new();
    let caster = match plr(world, id) {
        Some(p) => CasterInfo::of(world, p),
        None => return res,
    };
    if !success {
        fail_lines(world, id, &spl);
        charge_failure(world, id, &spl);
    } else {
        charge(world, id, i32::from(spl.energy), i32::from(spl.mana).max(0));
        let amount = roll_amount(&spl, caster.level, &mut world.rng);
        let peramt = per_target_share(amount, kind, ntargets);
        if combat::debug_on(world, id) {
            tracing::debug!(amount, peramt, "cast_no_target roll");
        }
        let effects = SpellEffects::from_record(&spl);
        // pass 1: Remove spell and Terminate Spell
        if remove_spells_pass(world, id, &spl, &effects, amount, &mut targets, &mut told) {
            return res;
        }
        targets.refresh_players(world, id, &spl);
        // pass 2
        no_target_slots(
            world, id, &spl, &effects, amount, peramt, nplayers, &targets, &caster, &mut told,
            &mut state, &rm,
        );
        // the teleport
        if let Some((room, map)) = state.teleport() {
            if !told.caster {
                let name = caster.name.clone();
                display_spell_success(world, &mut told, id, Some(id), &spl, &name, amount);
            }
            let dest = RoomKey::new(map as u32, room as u32);
            teleport(world, id, dest, kind == SpellTarget::FullParty);
        }
    }
    // the room's players' stats again
    for &(tid, _) in &targets.players {
        crate::items::equip::recalc(world, tid);
    }
    // the chain
    if let Some(next) = chain_spell(world, &state) {
        cast_no_target(world, id, next, opts);
    }
    crate::items::equip::recalc(world, id);
    if opts.autocombat {
        if let Some(p) = plr_mut(world, id)
            && p.mana < spl.mana
        {
            p.energy = 0;
        }
        engage_none(&mut res);
    }
    res
}

/// The Remove spell / Terminate Spell first pass of `cast_no_target`. `true` when the self
/// form found its spell: the cast ends there (sic).
fn remove_spells_pass(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    effects: &SpellEffects,
    amount: i32,
    targets: &mut AreaTargets,
    told: &mut Told,
) -> bool {
    let mut local_told = false;
    for slot in effects.iter() {
        let SpellEffect::RemoveSpell { chain } = slot.effect else {
            continue;
        };
        let remnum = SpellNo(slot.value);
        if remnum == SpellNo(0) {
            continue;
        }
        let who: Vec<SessionId> = if spl.target.is_area() {
            // the marks as they are now: an earlier slot may have taken some off
            targets.refresh_players(world, id, spl);
            targets.players.iter().map(|t| t.0).collect()
        } else {
            vec![id]
        };
        for tid in who {
            while let Some(tp) = plr(world, tid)
                && let Some(k) = tp.spells.iter().position(|&s| s == remnum)
            {
                let val = i32::from(tp.spellvals[k]);
                let name = tp.firstname.clone();
                if let Some(t) = plr_mut(world, tid) {
                    t.spells[k] = SpellNo(0);
                    t.spellvals[k] = 0;
                    t.spellrnds[k] = 0;
                }
                let Some(spl2) = world.spells.get(&remnum).cloned() else {
                    continue;
                };
                if !local_told {
                    // (sic) the caster is passed as the target
                    display_spell_success(world, told, id, Some(id), spl, &name, amount);
                }
                end_player_spell(world, tid, &spl2, val, chain);
                if !spl.target.is_area() {
                    return true;
                }
                // the slots after this pass leave him alone
                if let Some(t) = plr_mut(world, tid) {
                    t.status2.remove(PlayerStatus2::AREATARGET);
                }
                local_told = true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn no_target_slots(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    effects: &SpellEffects,
    roll: i32,
    rollper: i32,
    nplayers: i32,
    targets: &AreaTargets,
    caster: &CasterInfo,
    told: &mut Told,
    state: &mut CastState,
    rm: &silvermere_types::records::Room,
) {
    let kind = spl.target;
    let lasting = spl.duration != 0;
    let self_kind = kind.is_self();
    for slot in effects.iter() {
        let (amount, peramt) = if slot.value != 0 {
            (i32::from(slot.value), i32::from(slot.value))
        } else {
            (roll, rollper)
        };
        let effect = slot.effect;
        match effect {
            SpellEffect::Marker(_)
            | SpellEffect::KillDead
            | SpellEffect::DetectMagic
            | SpellEffect::RemoveSpell { .. } => continue,
            SpellEffect::TextBlock => {
                crate::scripts::special::run_text_block(
                    world,
                    id,
                    silvermere_types::ids::TextBlockNo(amount),
                );
                if !state.told {
                    let name = caster.name.clone();
                    display_spell_success(world, told, id, Some(id), spl, &name, amount);
                }
                state.told = true;
                continue;
            }
            SpellEffect::ClearItem => {
                crate::scripts::actions::erase_item_from_room(
                    world,
                    room_of_id(world, id),
                    ItemNo(amount),
                );
                continue;
            }
            SpellEffect::Teleport(_) | SpellEffect::EndCast | SpellEffect::EndCastChance => {
                let mut dummy = Target::None;
                let stats = caster.stats();
                let cast = Cast {
                    spell: spl,
                    caster: Some(&stats),
                    path: CastPath::OnSelf,
                    power: amount,
                };
                apply_effect(slot, &cast, &mut dummy, state, &mut world.rng);
                continue;
            }
            SpellEffect::OpenDoor => {
                if lasting {
                    silly(world, id, spl, "None");
                    continue;
                }
                let chance = (i32::from(caster.spellcast) - i32::from(spl.diff)) / 2;
                let opened = bash_room_exits(world, id, rm.key, chance);
                if opened > 0 && !state.told {
                    let name = caster.name.clone();
                    display_spell_success(world, told, id, Some(id), spl, &name, amount);
                }
                state.told = true;
                continue;
            }
            _ => {}
        }
        if self_kind || (kind == SpellTarget::FullParty && effect == SpellEffect::DispelMagic) {
            self_slot(world, id, spl, slot, amount, roll, caster, told, state);
            continue;
        }
        // The area kinds.
        area_slot(
            world, id, spl, slot, amount, peramt, nplayers, targets, caster, told, state,
        );
    }
}

fn room_of_id(world: &World, id: SessionId) -> RoomKey {
    plr(world, id).map(room_of).unwrap_or(RoomKey::new(0, 0))
}

/// One slot of `cast_no_target` on the caster himself (the self columns of the census).
#[allow(clippy::too_many_arguments)]
fn self_slot(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    amount: i32,
    roll: i32,
    caster: &CasterInfo,
    told: &mut Told,
    state: &mut CastState,
) {
    let lasting = spl.duration != 0;
    let effect = slot.effect;
    let name = caster.name.clone();
    if takes_default_branch(effect) {
        if !lasting {
            silly(world, id, spl, "None");
        } else if !state.told {
            let out = add_lasting_user(world, id, caster, spl, amount);
            lasting_user_lines(world, told, id, id, spl, amount, out);
            state.told = true;
        }
        return;
    }
    let was_told = state.told;
    let out = apply_to_user(
        world,
        id,
        caster,
        id,
        spl,
        slot,
        CastPath::OnSelf,
        amount,
        state,
    );
    match out {
        Outcome::Silly => silly(world, id, spl, "None"),
        Outcome::Lasting(o) => {
            if !was_told {
                lasting_user_lines(world, told, id, id, spl, amount, o);
            }
            // the stat alters raise only when the add worked (or an earlier slot told);
            // T-049 has done the raise. Alter HP's comes after the lines, below.
            if matches!(effect, SpellEffect::StatAlter(_)) && o.is_success() {
                crate::items::equip::recalc(world, id);
            }
            if !o.is_success() && matches!(effect, SpellEffect::StatAlter(_) | SpellEffect::AlterHp)
            {
                state.told = was_told;
            }
        }
        Outcome::Applied {
            amount: shown,
            knocked_out,
        } => match effect {
            SpellEffect::Damage { .. } => {
                if !was_told {
                    display_spell_success(world, told, id, Some(id), spl, &name, shown);
                }
                state.told = true;
                after_user_damage(world, id, knocked_out);
            }
            SpellEffect::Heal => {
                // the whole roll is shown, to nobody as the target
                if !was_told {
                    display_spell_success(world, told, id, None, spl, &name, roll);
                }
                state.told = true;
            }
            SpellEffect::HealMana => {
                if !was_told {
                    display_spell_success(world, told, id, None, spl, &name, shown);
                }
                state.told = true;
            }
            SpellEffect::Poison => {
                if !was_told {
                    display_spell_success(world, told, id, Some(id), spl, &name, amount);
                }
                state.told = true;
            }
            SpellEffect::Hunger => {
                // (sic) the slot index is passed as the target; nobody here
                if !was_told {
                    display_spell_success(world, told, id, None, spl, &name, amount);
                }
                state.told = true;
            }
            // Energy and Thirst say nothing
            _ => {}
        },
        Outcome::Ended { spells, poison } => {
            match effect {
                SpellEffect::CurePoison => {
                    // the poison spell ends first, then the cast is shown
                    report_ended(world, id, &spells, poison);
                    if !was_told {
                        display_spell_success(world, told, id, None, spl, &name, amount);
                    }
                }
                _ => {
                    // shown first, then each spell ends
                    if !was_told {
                        display_spell_success(world, told, id, Some(id), spl, &name, amount);
                    }
                    report_ended(world, id, &spells, poison);
                }
            }
            state.told = true;
        }
        Outcome::RemoveCurse { worn_slots } => {
            if !was_told {
                display_spell_success(world, told, id, Some(id), spl, &name, amount);
            }
            state.told = true;
            strip_worn(world, id, &worn_slots);
        }
        Outcome::Summon(n, _) => {
            // to nobody, as "everyone", without setting told
            if !was_told {
                display_spell_success(world, told, id, None, spl, "everyone", amount);
            }
            summon(world, id, n, Summoned::Servant);
        }
        Outcome::Immune | Outcome::NoEffect => {}
        other => tracing::debug!(?other, "cast_no_target self: outcome not shown"),
    }
    // the add has shown the cast with its prompt; then the hit points rise
    alter_hp_after_lines(world, id, state);
}

/// "Alter HP" took on `tid` this slot: the maximum and current hit points both rise by the
/// amount and the stats are recomputed keeping the current values, after the add's display
/// and prompt (in every casting engine and in room casts), so that prompt shows the figures
/// from before the raise (T-100).
pub(crate) fn alter_hp_after_lines(world: &mut World, tid: SessionId, state: &mut CastState) {
    let Some(v) = state.alter_hp.take() else {
        return;
    };
    if let Some(p) = crate::combat::player_mut(world, tid) {
        silvermere_rules::spells::raise_hp(p, v);
    }
    crate::items::equip::recalc(world, tid);
}

/// What a summoned monster does, by the summon's target.
#[derive(Debug, Clone, Copy)]
enum Summoned {
    /// On the caster himself: it follows the caster as a friendly, charmed servant.
    Servant,
    /// On a player: it follows that player, unfriendly, so the monsters' round sets it on
    /// him.
    Follows(SessionId),
    /// On a monster: it hunts that monster, unfriendly.
    Hunts(ActiveMonsterId),
}

/// A summon: known monster `n` in the caster's room, forced past the room's spawn rules,
/// then set up as `what` says.
fn summon(world: &mut World, id: SessionId, n: i32, what: Summoned) {
    let key = room_of_id(world, id);
    let req = crate::monsters::generate::Request {
        areatype: -1,
        monnum: silvermere_types::ids::MonsterNo(n),
        minlevel: 0,
        maxlevel: 0x7FFF,
        owner: None,
        ally: false,
        force: true,
    };
    let Some(newmon) = crate::monsters::generate::generate_monster(world, key, req) else {
        return;
    };
    match what {
        Summoned::Servant | Summoned::Follows(_) => {
            let master = match what {
                Summoned::Follows(t) => t,
                _ => id,
            };
            let name = plr(world, master)
                .map(|p| p.firstname.clone())
                .unwrap_or_default();
            let servant = matches!(what, Summoned::Servant);
            if let Some(m) = crate::combat::monster_mut(world, newmon) {
                m.following = name;
                m.friendly = u8::from(servant);
                if servant {
                    m.flags
                        .insert(silvermere_types::flags::ActiveMonsterFlags::CHARMED);
                }
            }
        }
        Summoned::Hunts(target) => {
            // only when both monsters are still there
            if crate::combat::monster(world, target).is_none() {
                return;
            }
            if let Some(m) = crate::combat::monster_mut(world, newmon) {
                m.hunting = target;
                m.friendly = 0;
            }
            // (sic) the new monster goes into every free hunter slot of the target, not
            // only the first
            if let Some(t) = crate::combat::monster_mut(world, target) {
                for h in t.hunters.iter_mut().filter(|h| h.0 == 0) {
                    *h = newmon;
                }
            }
        }
    }
}

/// The area columns of `cast_no_target` for one slot.
#[allow(clippy::too_many_arguments)]
fn area_slot(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    amount: i32,
    peramt: i32,
    nplayers: i32,
    targets: &AreaTargets,
    caster: &CasterInfo,
    told: &mut Told,
    state: &mut CastState,
) {
    use SpellEffect as E;
    let kind = spl.target;
    let lasting = spl.duration != 0;
    let effect = slot.effect;
    // Heal and Heal Mana on "Divided Area Not You" need company
    if matches!(effect, E::Heal | E::HealMana) && nplayers <= 1 && kind == SpellTarget::DivArea {
        tell_user(world, id, Msg::CastAlone);
        return;
    }
    let addable = matches!(
        effect,
        E::Damage {
            ignore_armour: true
        } | E::Energy
            | E::Hunger
            | E::Thirst
            | E::Heal
            | E::HealMana
            | E::Poison
            | E::CurePoison
            | E::Lasting(_)
            | E::Component
            | E::Guarded
            | E::Unknown(_)
            | E::GiveSpell
            | E::Slay
            | E::Shatter
            | E::Lore
            | E::Scatter
    );
    let always_silly = matches!(
        effect,
        E::RemoveCurse
            | E::Freedom
            | E::DispelMagic
            | E::Enslave
            | E::Summon
            | E::StatAlter(_)
            | E::AlterHp
    ) || (effect == E::StealHp && lasting);
    if always_silly {
        silly(world, id, spl, "None");
        return;
    }
    if lasting && addable {
        if effect == E::Poison {
            // the poison levels rise first, silently
            for &(tid, _) in &targets.players {
                if let Some(tp) = plr(world, tid)
                    && !user_has(world, tp, Ability::PoisonImmune)
                    && let Some(tp) = plr_mut(world, tid)
                    && i32::from(tp.poison) < peramt
                {
                    tp.poison = peramt as i16;
                }
            }
            for &(mon, _) in &targets.monsters {
                if let Some(m) = combat::monster(world, mon)
                    && !monster_has(world, m, Ability::PoisonImmune)
                    && let Some(m) = combat::monster_mut(world, mon)
                    && i32::from(m.poison) < amount
                {
                    m.poison = amount as i16;
                }
            }
        }
        if !state.told {
            add_duration_spell_to_room(world, told, id, caster, spl, peramt, targets);
        }
        state.told = true;
        return;
    }
    if !lasting
        && !matches!(
            effect,
            E::Damage { .. }
                | E::StealHp
                | E::Energy
                | E::Heal
                | E::HealMana
                | E::Poison
                | E::CurePoison
                | E::Hunger
                | E::Thirst
        )
    {
        silly(world, id, spl, "None");
        return;
    }
    let path = CastPath::Area { undivided: amount };
    // a resisted area Steal Hit Points zeroes `peramt` for the later monsters (sic; T-049
    // carries it in the cast state); `peramt` is taken afresh for each slot, so the zeroing
    // ends with the slot
    state.area_stealhp_zeroed = false;
    // The player loop: every marked player (the damage spells skip the caster unless the
    // kind includes him); the line's guard is `told` but the loop does not set it.
    let was_told = state.told;
    let warns = plr(world, id).is_some_and(|c| c.settings.contains(PlayerSettings::EVILWARN));
    for &(tid, tevil) in &targets.players {
        // (and each area player loop) an evil act on a hidden player the caster could not
        // see is skipped for a caster with evil warnings on
        if tevil
            && targets.evilspell
            && targets.only_hidden
            && warns
            && plr(world, tid).is_some_and(|t| t.hidden != 0)
        {
            continue;
        }
        if matches!(
            effect,
            E::Damage {
                ignore_armour: true
            }
        ) && tid == id
            && !kind.area_includes_caster()
        {
            continue;
        }
        let Some(tp) = plr(world, tid) else {
            continue;
        };
        let name = tp.firstname.clone();
        let out = apply_to_user(world, tid, caster, id, spl, slot, path, peramt, state);
        match (effect, out.clone()) {
            (_, Outcome::NoEffect | Outcome::Immune) => {}
            (
                E::Damage { .. } | E::Energy,
                Outcome::Applied {
                    amount: shown,
                    knocked_out,
                },
            ) => {
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &name, shown);
                }
                if check_kill_user(world, tid).died() {
                    continue;
                }
                if knocked_out && effect != E::Energy {
                    combat::knock_out_user(world, tid);
                }
            }
            (E::StealHp, Outcome::StolenHp { amount: shown })
            | (
                E::StealHp,
                Outcome::Applied {
                    amount: shown,
                    knocked_out: true,
                },
            ) => {
                // the caster gains even when the target went down
                if let Some(c) = plr_mut(world, id) {
                    gain_stolen_hp(c, shown);
                }
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &name, shown);
                }
                if !check_kill_user(world, tid).died()
                    && let Some(t) = plr(world, tid)
                    && t.hp <= 0
                    && i32::from(t.hp) + peramt > 0
                {
                    combat::knock_out_user(world, tid);
                }
            }
            (E::Heal | E::Poison | E::CurePoison, _) => {
                // Cure Poison: the poison spell ends first, then the cast is shown
                if let Outcome::Ended { spells, poison } = out {
                    report_ended(world, tid, &spells, poison);
                }
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &name, peramt);
                }
            }
            (E::HealMana, Outcome::Applied { amount: shown, .. }) if !was_told => {
                display_spell_success(world, told, id, Some(tid), spl, &name, shown);
            }
            _ => {}
        }
    }
    // The monster loop, for the kinds that reach them; its line sets `told`.
    if kind.area_reaches_monsters() {
        for &(mon, _) in &targets.monsters {
            let Some(m) = combat::monster(world, mon) else {
                continue;
            };
            let name = m.name.clone();
            let known = world.monsters.get(&m.knownnum);
            let mr = monster_magic_resistance(
                monster_ability(world, m, Ability::MagicResist),
                known.map_or(0, |k| k.magicres),
                None,
            );
            if matches!(effect, E::Damage { .. }) {
                // the resistance die is rolled but never wins (sic)
                monster_resists_area(spl, mr, &mut world.rng);
            }
            if matches!(effect, E::Hunger | E::Thirst | E::HealMana) {
                continue;
            }
            let out = apply_to_monster(world, mon, caster, spl, slot, path, peramt, mr, state);
            match (effect, out) {
                (E::Damage { .. }, Outcome::Applied { amount: shown, .. }) => {
                    if !state.told {
                        display_spell_success(world, told, id, None, spl, &name, shown);
                    }
                    state.told = true;
                    after_monster_damage(world, id, mon);
                }
                (E::StealHp, Outcome::Resisted) => {
                    if !state.told {
                        display_spell_success(world, told, id, None, spl, &name, peramt);
                    }
                    state.told = true;
                    tell_user(world, id, Msg::CastMonsterResists { target: name });
                    after_monster_damage(world, id, mon);
                }
                (E::StealHp, Outcome::StolenHp { amount: shown }) => {
                    if !state.told {
                        display_spell_success(world, told, id, None, spl, &name, shown);
                    }
                    state.told = true;
                    if let Some(c) = plr_mut(world, id) {
                        gain_stolen_hp(c, shown);
                    }
                    after_monster_damage(world, id, mon);
                }
                (E::Heal, Outcome::Applied { amount: gain, .. }) => {
                    if gain != 0 {
                        if !state.told {
                            display_spell_success(world, told, id, None, spl, &name, peramt);
                        }
                        state.told = true;
                    }
                }
                (E::Poison | E::CurePoison, Outcome::Applied { .. }) => {
                    if !state.told {
                        display_spell_success(world, told, id, None, spl, &name, amount);
                    }
                    state.told = true;
                }
                _ => {}
            }
        }
    }
    state.told = true;
}

// ---------------------------------------------------------------------------------------
// cast_user_target

/// A spell on one player. Attack spells in auto-combat loop while the energy lasts (at most
/// [`MAXTRIES`] attempts).
pub fn cast_user_target(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    tid: SessionId,
    opts: CastOpts,
) -> CastResult {
    let mut res = CastResult::new(CastOutcome::Refused);
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return res;
    };
    if plr(world, id).is_none() || plr(world, tid).is_none() {
        return res;
    }
    let manual = opts.mode == CastMode::Item;
    let target_engage = Some(AutoTarget::Player(tid));
    let engage_t = |res: &mut CastResult| {
        res.engaged = Some(Engage {
            target: target_engage,
            spell: spl.number,
        });
    };
    // confusion
    if super::active::check_confusion(world, id) {
        if opts.autocombat {
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            engage_t(&mut res);
        }
        return res;
    }
    let Some(p) = plr(world, id) else {
        return res;
    };
    if p.hp <= 0 || p.status2.contains(PlayerStatus2::AFRAID) {
        if opts.autocombat {
            display_autocombat_broken(world, id);
        }
        return res;
    }
    // Magebind
    if user_has(world, p, Ability::Magebind) {
        tell_user(world, id, Msg::CastSpellFails);
        return res;
    }
    let Some(cls) = world.classes.get(&p.class).cloned() else {
        return res;
    };
    let kai = cls.magery == Magery::Kai;
    // Kai Bind
    if kai && !manual && user_has(world, p, Ability::KaiBind) {
        tell_user(world, id, Msg::CastPowerFails);
        return res;
    }
    let key = room_of(p);
    let Some(rm) = world.room(key).cloned() else {
        return res;
    };
    // the caster (and, unless General, the target) gives himself away
    if let Some(p) = plr_mut(world, id) {
        p.status
            .remove(PlayerStatus::RESTING | PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
    if spl.spell_type != SpellType::General
        && let Some(t) = plr_mut(world, tid)
    {
        t.status
            .remove(PlayerStatus::RESTING | PlayerStatus::SNEAKING);
    }
    let attack = is_attack_spell(&spl);
    if attack && id == tid {
        tell_user(world, id, Msg::CastAttackSelf);
        return res;
    }
    // the protected room
    if (attack || spell_has_ability(&spl, Ability::EvilInCombat))
        && protected_refuses(world, id, &rm)
    {
        if opts.autocombat {
            kill_autocombat(world, id);
            display_autocombat_broken(world, id);
        } else {
            guilt(world, id, &rm);
        }
        return res;
    }
    // on oneself it is a no-target cast
    if id == tid && spl.target != SpellTarget::Any {
        return cast_no_target(world, id, spell, opts);
    }
    let arena = arena_safe_room(world, &rm);
    let debug = combat::debug_on(world, id);
    let range_ok_v = combat::attack::pvp_level_range_ok(world, id, tid);
    let range_ok = || range_ok_v;
    let mut success = false;
    if manual {
        if attack && !arena && !range_ok() && !debug {
            tell_user(world, id, Msg::CombatUnbalanced);
            return res;
        }
        if spl.magery != Magery::None && (cls.magery != spl.magery || cls.magelvl < spl.magelvl) {
            return res;
        }
        success = true;
        if attack
            && !arena
            && add_evil_points(
                world,
                id,
                Some(tid),
                i32::from(EVIL_SPELL_PTS),
                EVIL_TICKS,
                EvilKind::Attack,
            )
        {
            return res;
        }
    }
    if !matches!(
        spl.target,
        SpellTarget::Single
            | SpellTarget::SelfOrSingle
            | SpellTarget::MonsterOrSingle
            | SpellTarget::Any
    ) {
        tell_user(world, id, Msg::CastNotOnUser);
        return res;
    }
    // the target left the room
    let (Some(p), Some(t)) = (plr(world, id), plr(world, tid)) else {
        return res;
    };
    if p.room != t.room || p.map != t.map {
        if opts.autocombat {
            let gone = p.status.contains(PlayerStatus::TARGETGONE);
            if gone && let Some(p) = plr_mut(world, id) {
                p.pvpcnt += 1;
                if p.pvpcnt > 1 {
                    kill_autocombat(world, id);
                    display_autocombat_broken(world, id);
                    return res;
                }
            }
            engage_t(&mut res);
        }
        return res;
    }
    if let Some(p) = plr_mut(world, id) {
        p.status.remove(PlayerStatus::TARGETGONE);
    }
    crate::state::login::leave::stop_users_exit(world, tid);
    let Some(p) = plr(world, id) else {
        return res;
    };
    if p.level < spl.level {
        tell_user(world, id, Msg::CastTooPowerful);
        return res;
    }
    // the first pass
    let mut immune = false;
    let mut nonmagic = false;
    for (ability, value) in spl.abilities.iter().copied() {
        match ability {
            Ability::EvilInCombat if !arena => {
                if spl.spell_type == SpellType::General && !range_ok() && !debug {
                    tell_user(world, id, Msg::CombatUnbalanced);
                    return res;
                }
                if add_evil_points(
                    world,
                    id,
                    Some(tid),
                    i32::from(EVIL_SPELL_PTS),
                    EVIL_TICKS,
                    EvilKind::Attack,
                ) {
                    return res;
                }
            }
            Ability::NonMagical => nonmagic = true,
            Ability::HoldPerson | Ability::Paralyze => {
                if let Some(t) = plr(world, tid)
                    && user_has(world, t, Ability::Freedom)
                {
                    immune = true;
                }
            }
            Ability::Poison => {
                if let Some(t) = plr(world, tid)
                    && user_has(world, t, Ability::PoisonImmune)
                {
                    immune = true;
                }
            }
            Ability::Component
                if !use_spell_component(world, id, &spl, ItemNo(i32::from(value))) =>
            {
                return res;
            }
            _ => {}
        }
    }
    if !immune
        && spl.duration != 0
        && spl.spell_type == SpellType::Autocombat
        && spl.element == Element::Poison
        && let Some(t) = plr(world, tid)
        && user_has(world, t, Ability::PoisonImmune)
    {
        immune = true;
    }
    // the auto-combat pre-check
    let Some(p) = plr(world, id) else {
        return res;
    };
    let mut done = false;
    res.outcome = CastOutcome::Cast;
    if !(p.energy >= spl.energy && p.mana >= spl.mana) && opts.autocombat {
        if let Some(p) = plr_mut(world, id)
            && p.energy >= spl.energy
        {
            p.energy -= spl.energy;
        }
        done = true;
    }
    // Spell Immunity
    if let Some(t) = plr(world, tid) {
        let immunity = user_ability(world, t, Ability::SpellImmune);
        if spell_immune(immunity, spl.level, nonmagic, opts.autocombat) {
            let name = t.firstname.clone();
            tell_user(world, id, Msg::CastNoEffectOn { target: name });
            res.outcome = CastOutcome::Refused;
            return res;
        }
    }
    let caster = match plr(world, id) {
        Some(p) => CasterInfo::of(world, p),
        None => return res,
    };
    let effects = SpellEffects::from_record(&spl);
    let mut tries = 0;
    let mut killed = false;
    // once per cast
    let mut endspell: Option<SpellNo> = None;
    let mut endchance = 100;
    while !done && !killed && tries < MAXTRIES {
        let mut told = Told::default();
        let mut resisted = immune;
        tries += 1;
        if manual {
            done = true;
        } else {
            success = false;
        }
        if kai {
            success = true;
        }
        let roll = world.rng.range(0, 100);
        if !manual {
            if spl.spell_type == SpellType::General {
                done = true;
                let Some(p) = plr_mut(world, id) else {
                    return res;
                };
                if p.settings.contains(PlayerSettings::CANCAST) {
                    p.settings.remove(PlayerSettings::CANCAST);
                    let p = p.clone();
                    if roll_succeeds(&p, &spl, roll) {
                        success = true;
                    }
                } else {
                    tell_user(
                        world,
                        id,
                        Msg::CastAlreadyThisRound {
                            what: round_word(kai),
                        },
                    );
                    res.outcome = CastOutcome::Refused;
                    return res;
                }
            } else {
                if !opts.autocombat && !arena && !range_ok() && !debug {
                    tell_user(world, id, Msg::CombatUnbalanced);
                    res.outcome = CastOutcome::Refused;
                    return res;
                }
                // combat starts; the cast comes with the rounds
                if !opts.autocombat && spl.duration == 0 {
                    if !arena
                        && add_evil_points(
                            world,
                            id,
                            Some(tid),
                            i32::from(EVIL_SPELL_PTS),
                            EVIL_TICKS,
                            EvilKind::Attack,
                        )
                    {
                        res.outcome = CastOutcome::Refused;
                        return res;
                    }
                    tell_user(world, id, Msg::CombatEngaged);
                    let who = caster.name.clone();
                    let tname = plr(world, tid)
                        .map(|t| t.firstname.clone())
                        .unwrap_or_default();
                    if opts.announce {
                        // the target's waiting output is thrown away first
                        if plr(world, tid)
                            .is_some_and(|t| t.settings.contains(PlayerSettings::ATKINTERRUPT))
                        {
                            combat::interrupt_output(world, tid);
                        }
                        if let Some(s) = world.session_mut(tid) {
                            s.output
                                .push(Msg::CastMovesToAttackYou { who: who.clone() });
                            s.push_prompt();
                        }
                        tell_room(
                            world,
                            key,
                            [Some(id), Some(tid)],
                            Noise::Always,
                            TellBlind::All,
                            true,
                            None,
                            &Msg::CastMovesToAttack { who, target: tname },
                        );
                    }
                    if let Some(p) = plr_mut(world, id) {
                        p.energy = 0;
                    }
                    res.outcome = CastOutcome::Engaged;
                    engage_t(&mut res);
                    return res;
                }
                let p = plr(world, id).cloned();
                if p.is_some_and(|p| roll_succeeds(&p, &spl, roll)) {
                    success = true;
                }
            }
        }
        let modifier = if success {
            plr(world, tid).map_or(0, |t| user_element_resist(world, t, &spl))
        } else {
            0
        };
        let Some(p) = plr(world, id) else {
            return res;
        };
        if p.energy >= spl.energy && p.mana >= spl.mana {
            // resistance the roll is drawn whether or not the target is immune (`resisted`
            // starts as `immune`)
            if let Some(t) = plr(world, tid) {
                let worn: Vec<_> = t.worn.iter().filter_map(|n| world.items.get(n)).collect();
                let negated = item_negates(worn, spl.number);
                let anti = user_has(world, t, Ability::AntiMagic);
                let mr = i32::from(t.mr);
                resisted |= negated
                    || (success
                        && silvermere_rules::spells::resist::may_resist(&spl, anti)
                        && world.rng.range(1, 100)
                            <= silvermere_rules::spells::resist::resist_chance(mr, MAXCHANCE));
            }
            let tname = plr(world, tid)
                .map(|t| t.firstname.clone())
                .unwrap_or_default();
            if resisted {
                resist_lines(world, id, tid, &caster.name, &tname, &spl, key);
                charge_failure(world, id, &spl);
            } else if !success {
                user_fail_lines(world, id, tid, &caster.name, &tname, &spl, key);
                charge_failure(world, id, &spl);
            } else {
                charge(world, id, i32::from(spl.energy), i32::from(spl.mana).max(0));
                let rolled = roll_amount(&spl, caster.level, &mut world.rng);
                let amount = (i64::from(rolled) * i64::from(100 - modifier) / 100) as i32;
                let mut state = CastState::new();
                // Remove spell / Terminate Spell
                let mut removed = false;
                for slot in effects.iter() {
                    let SpellEffect::RemoveSpell { chain } = slot.effect else {
                        continue;
                    };
                    let value = slot.amount(amount);
                    let rem = SpellNo(value as i16);
                    while let Some(t) = plr(world, tid)
                        && let Some(k) = t.spells.iter().position(|&s| s == rem)
                    {
                        let val = i32::from(t.spellvals[k]);
                        if let Some(t) = plr_mut(world, tid) {
                            t.spells[k] = SpellNo(0);
                            t.spellvals[k] = 0;
                            t.spellrnds[k] = 0;
                        }
                        let Some(spl2) = world.spells.get(&rem).cloned() else {
                            continue;
                        };
                        if !state.told {
                            display_spell_success(
                                world,
                                &mut told,
                                id,
                                Some(tid),
                                &spl,
                                &tname,
                                value,
                            );
                        }
                        state.told = true;
                        end_player_spell(world, tid, &spl2, val, chain);
                        removed = true;
                        done = true;
                        break;
                    }
                }
                if removed {
                    break;
                }
                let mut value = amount;
                for slot in effects.iter() {
                    if killed {
                        break;
                    }
                    value = slot.amount(amount);
                    killed = user_slot(
                        world, id, tid, &spl, slot, value, amount, &caster, &mut told, &mut state,
                    );
                }
                // the teleport: both halves non-zero and the map loaded
                if !killed
                    && let Some((room, map)) = state.teleport()
                    && room != 0
                    && map != 0
                    && world.room(RoomKey::new(map as u32, room as u32)).is_some()
                {
                    // (sic) the value the last slot left, the caster as the target
                    if !state.told {
                        display_spell_success(world, &mut told, id, Some(id), &spl, &tname, value);
                    }
                    if let Some(s) = world.session_mut(tid) {
                        s.party.dragging = None;
                    }
                    // stop_following(targetnum, -1)
                    crate::party::stop_following_all(world, tid);
                    crate::movement::move_user::set_user_location(
                        world,
                        tid,
                        RoomKey::new(map as u32, room as u32),
                        false,
                    );
                }
                if let Some(n) = state.chain.filter(|n| n.0 != 0) {
                    endspell = Some(n);
                    endchance = state.chain_chance;
                }
            }
        } else {
            if opts.autocombat {
                if let Some(p) = plr_mut(world, id)
                    && p.energy >= spl.energy
                {
                    p.energy -= spl.energy;
                }
                done = true;
            } else {
                short_lines(world, id, &spl, false);
            }
            if tries == 1 {
                res.outcome = CastOutcome::Refused;
            }
        }
        // the chain: rolled only when nothing died and a spell is set; a failed roll leaves
        // it for the next try
        if !killed
            && let Some(next) = endspell
            && world.rng.range(0, 100) < endchance
        {
            cast_user_target(world, id, next, tid, opts);
            endspell = None;
            endchance = 100;
            done = true;
        }
    }
    if killed {
        res.outcome = CastOutcome::KilledTarget;
        return res;
    }
    if opts.autocombat {
        if let Some(p) = plr_mut(world, id)
            && p.mana < spl.mana
        {
            p.energy = 0;
        }
        engage_t(&mut res);
    }
    crate::items::equip::recalc(world, tid);
    res
}

fn resist_lines(
    world: &mut World,
    id: SessionId,
    tid: SessionId,
    who: &str,
    tname: &str,
    spl: &Spell,
    key: RoomKey,
) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::CastResisted {
            spell: spl.name.clone(),
            target: tname.to_owned(),
        });
        s.push_prompt();
    }
    if let Some(s) = world.session_mut(tid) {
        s.output.push(Msg::CastYouResisted {
            who: who.to_owned(),
            spell: spl.name.clone(),
        });
        s.push_prompt();
    }
    tell_room(
        world,
        key,
        [Some(id), Some(tid)],
        Noise::Medium,
        TellBlind::All,
        true,
        None,
        &Msg::CastOtherResisted {
            target: tname.to_owned(),
            who: who.to_owned(),
            spell: spl.name.clone(),
        },
    );
}

fn user_fail_lines(
    world: &mut World,
    id: SessionId,
    tid: SessionId,
    who: &str,
    tname: &str,
    spl: &Spell,
    key: RoomKey,
) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::CastFailAt {
            spell: spl.name.clone(),
            target: tname.to_owned(),
        });
        s.push_prompt();
    }
    if let Some(s) = world.session_mut(tid) {
        s.output.push(Msg::CastFailOnYou {
            who: who.to_owned(),
            spell: spl.name.clone(),
        });
        s.push_prompt();
    }
    tell_room(
        world,
        key,
        [Some(id), Some(tid)],
        Noise::Medium,
        TellBlind::All,
        true,
        None,
        &Msg::CastFailAtRoom {
            who: who.to_owned(),
            spell: spl.name.clone(),
            target: tname.to_owned(),
        },
    );
}

/// One slot of `cast_user_target`. `true` when the target died.
#[allow(clippy::too_many_arguments)]
fn user_slot(
    world: &mut World,
    id: SessionId,
    tid: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    value: i32,
    amount: i32,
    caster: &CasterInfo,
    told: &mut Told,
    state: &mut CastState,
) -> bool {
    use SpellEffect as E;
    let lasting = spl.duration != 0;
    let effect = slot.effect;
    let tname = plr(world, tid)
        .map(|t| t.firstname.clone())
        .unwrap_or_default();
    match effect {
        E::Marker(_) | E::KillDead | E::DetectMagic | E::RemoveSpell { .. } => return false,
        E::TextBlock => {
            crate::scripts::special::run_text_block(
                world,
                tid,
                silvermere_types::ids::TextBlockNo(value),
            );
            if !state.told {
                display_spell_success(world, told, id, Some(tid), spl, &tname, value);
            }
            state.told = true;
            return false;
        }
        E::ClearItem => {
            crate::scripts::actions::erase_item_from_room(
                world,
                room_of_id(world, id),
                ItemNo(value),
            );
            return false;
        }
        _ => {}
    }
    if takes_default_branch(effect) {
        if !lasting {
            silly(world, id, spl, "User");
        } else if !state.told {
            let out = add_lasting_user(world, tid, caster, spl, value);
            lasting_user_lines(world, told, id, tid, spl, value, out);
            state.told = true;
        }
        return false;
    }
    let was_told = state.told;
    let out = apply_to_user(
        world,
        tid,
        caster,
        id,
        spl,
        slot,
        CastPath::Single,
        amount,
        state,
    );
    match out {
        Outcome::Silly => {
            let kind = if effect == E::RemoveCurse {
                "None"
            } else {
                "User"
            };
            silly(world, id, spl, kind);
        }
        Outcome::Lasting(o) => {
            if !was_told {
                lasting_user_lines(world, told, id, tid, spl, value, o);
            }
            if matches!(effect, E::StatAlter(_)) && o.is_success() {
                crate::items::equip::recalc(world, tid);
            }
        }
        Outcome::AlreadyActive => {
            if !told.caster
                && let Some(s) = world.session_mut(id)
            {
                s.output.push(Msg::CastFail {
                    spell: spl.name.clone(),
                });
                s.push_prompt();
            }
        }
        Outcome::Applied {
            amount: shown,
            knocked_out,
        } => match effect {
            E::Damage { .. } | E::Slay => {
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, shown);
                }
                state.told = true;
                return after_user_damage(world, tid, knocked_out);
            }
            E::StealHp => {
                // (sic) the caster gains even when the target went down
                if let Some(c) = plr_mut(world, id) {
                    gain_stolen_hp(c, value);
                }
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, value);
                }
                state.told = true;
                return after_user_damage(world, tid, knocked_out);
            }
            E::Heal => {
                // (sic) the roll is shown, not the gain
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, amount);
                }
                state.told = true;
            }
            E::HealMana | E::Energy | E::Poison => {
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, shown);
                }
                state.told = true;
            }
            // Hunger and Thirst say nothing
            _ => {}
        },
        Outcome::StolenHp { amount: shown } => {
            if let Some(c) = plr_mut(world, id) {
                gain_stolen_hp(c, shown);
            }
            if !was_told {
                display_spell_success(world, told, id, Some(tid), spl, &tname, shown);
            }
            state.told = true;
        }
        Outcome::Ended { spells, poison } => {
            if effect == E::CurePoison {
                // the poison spell ends first, then the cast is shown
                report_ended(world, tid, &spells, poison);
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, value);
                }
            } else {
                // shown first, then each spell ends
                if !was_told {
                    display_spell_success(world, told, id, Some(tid), spl, &tname, value);
                }
                report_ended(world, tid, &spells, poison);
            }
            state.told = true;
        }
        Outcome::RemoveCurse { worn_slots } => {
            if !was_told {
                display_spell_success(world, told, id, Some(tid), spl, &tname, value);
            }
            state.told = true;
            strip_worn(world, tid, &worn_slots);
        }
        Outcome::Shattered { item, destroyed } => shatter_lines(world, id, tid, item, destroyed),
        Outcome::Summon(n, _) => {
            // without setting told (sic)
            if !was_told {
                display_spell_success(world, told, id, Some(tid), spl, &tname, value);
            }
            summon(world, id, n, Summoned::Follows(tid));
        }
        _ => {}
    }
    // the add has shown the cast with its prompts; then the hit points rise
    alter_hp_after_lines(world, tid, state);
    false
}

/// "Shatter": the weapon's abilities come off; outside a safe arena the item goes and both
/// are told; inside one only the lines of what would have happened.
fn shatter_lines(world: &mut World, id: SessionId, tid: SessionId, item: ItemNo, destroyed: bool) {
    let Some(itm) = world.items.get(&item).cloned() else {
        return;
    };
    crate::items::equip::unequip_apply_item_abilities(world, tid, &itm);
    let tname = plr(world, tid)
        .map(|t| t.firstname.clone())
        .unwrap_or_default();
    if destroyed {
        if crate::items::inventory::remove_item_from_inventory(world, tid, item).is_some() {
            tell_user(
                world,
                id,
                Msg::CastShattered {
                    who: tname,
                    item: itm.name.clone(),
                },
            );
            if let Some(s) = world.session_mut(tid) {
                s.output.push(Msg::CastYourShattered { item: itm.name });
                s.push_prompt();
            }
        }
    } else {
        tell_user(
            world,
            id,
            Msg::CastArenaShattered {
                who: tname,
                item: itm.name.clone(),
            },
        );
        if let Some(s) = world.session_mut(tid) {
            s.output
                .push(Msg::CastArenaYourShattered { item: itm.name });
            s.push_prompt();
        }
    }
    crate::items::equip::recalc(world, tid);
}

// ---------------------------------------------------------------------------------------
// cast_monster_target

/// A spell on one monster. Attack spells loop while the energy lasts (at most
/// [`MAXTRIES`]): with 1000 energy a 250-energy spell is cast four times a round.
pub fn cast_monster_target(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    mut mon: ActiveMonsterId,
    opts: CastOpts,
) -> CastResult {
    let mut res = CastResult::new(CastOutcome::Refused);
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return res;
    };
    let manual = opts.mode == CastMode::Item;
    let engage_m = |res: &mut CastResult, mon: ActiveMonsterId| {
        res.engaged = Some(Engage {
            target: Some(AutoTarget::Monster(mon)),
            spell: spl.number,
        });
    };
    if super::active::check_confusion(world, id) {
        if opts.autocombat {
            if let Some(p) = plr_mut(world, id) {
                p.energy = 0;
            }
            engage_m(&mut res, mon);
        }
        return res;
    }
    let Some(p) = plr(world, id) else {
        return res;
    };
    // (sic) only the line: auto-combat is not stopped here
    if p.hp <= 0 || p.status2.contains(PlayerStatus2::AFRAID) {
        if opts.autocombat {
            display_autocombat_broken(world, id);
        }
        return res;
    }
    if user_has(world, p, Ability::Magebind) {
        tell_user(world, id, Msg::CastSpellFails);
        return res;
    }
    let Some(cls) = world.classes.get(&p.class).cloned() else {
        return res;
    };
    let kai = cls.magery == Magery::Kai;
    if kai && !manual && user_has(world, p, Ability::KaiBind) {
        tell_user(world, id, Msg::CastPowerFails);
        return res;
    }
    if !matches!(
        spl.target,
        SpellTarget::Monster | SpellTarget::MonsterOrSingle | SpellTarget::Any
    ) {
        tell_user(world, id, Msg::CastNotOnMonster);
        return res;
    }
    let Some(m) = combat::monster(world, mon) else {
        return res;
    };
    let Some(kmon) = world.monsters.get(&m.knownnum).cloned() else {
        return res;
    };
    if m.room != p.room || m.map != p.map {
        if !opts.autocombat {
            let name = m.name.clone();
            tell_user(world, id, Msg::CastMonsterGone { mon: name });
        }
        return res;
    }
    if p.level < spl.level {
        tell_user(world, id, Msg::CastLevelTooLow);
        return res;
    }
    let key = room_of(p);
    let Some(rm) = world.room(key).cloned() else {
        return res;
    };
    if let Some(p) = plr_mut(world, id) {
        p.status.remove(PlayerStatus::TARGETGONE);
    }
    if protected_refuses(world, id, &rm) {
        guilt(world, id, &rm);
        return res;
    }
    let Some(m) = combat::monster(world, mon) else {
        return res;
    };
    let reqtohit = monster_ability(world, m, Ability::ReqToHit);
    if monster_has(world, m, Ability::ReqToHit)
        && !silvermere_rules::spells::spell_can_hit_monster(reqtohit, &spl)
    {
        tell_user(world, id, Msg::CastReqtohit);
        return res;
    }
    // (sic) a magery spell from an item is always refused
    if manual && spl.magery != Magery::None {
        return res;
    }
    let mut success = manual;
    let arena = arena_safe_room(world, &rm);
    if manual && is_attack_spell(&spl) && !arena && evil_act(world, id, mon, true) {
        return res;
    }
    if let Some(p) = plr_mut(world, id) {
        p.status
            .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
        p.hidden = 0;
    }
    // the pre-pass, when the cast can be paid for
    let mut nonmagical = false;
    let mut magres: Option<i32> = None;
    let Some(p) = plr(world, id) else {
        return res;
    };
    let mut done;
    if p.energy >= spl.energy && p.mana >= spl.mana {
        for (ability, value) in spl.abilities.iter().copied() {
            match ability {
                Ability::NonMagical => nonmagical = true,
                Ability::KillDead | Ability::AffectsAnimal | Ability::AffectsLiving => {
                    let Some(m) = combat::monster(world, mon) else {
                        return res;
                    };
                    let animal = monster_has(world, m, Ability::Animal);
                    let nonliving = monster_has(world, m, Ability::NonLiving);
                    let refused = match ability {
                        Ability::KillDead => kmon.undead == 0,
                        Ability::AffectsAnimal => !animal,
                        _ => nonliving,
                    };
                    if refused {
                        let name = m.name.clone();
                        tell_user(world, id, Msg::CastNoEffectOn { target: name });
                        return res;
                    }
                }
                Ability::Enslave => magres = Some(i32::from(kmon.charmres)),
                Ability::EvilInCombat => {
                    if !arena && evil_act(world, id, mon, true) {
                        return res;
                    }
                }
                Ability::Component
                    if !use_spell_component(world, id, &spl, ItemNo(i32::from(value))) =>
                {
                    return res;
                }
                _ => {}
            }
        }
        done = false;
    } else if opts.autocombat {
        if let Some(p) = plr_mut(world, id)
            && p.energy >= spl.energy
        {
            p.energy -= spl.energy;
        }
        done = true;
    } else {
        done = false;
    }
    // Spell Immunity
    let Some(m) = combat::monster(world, mon) else {
        return res;
    };
    let immunity = monster_ability(world, m, Ability::SpellImmune);
    if spell_immune(immunity, spl.level, nonmagical, opts.autocombat) {
        let name = m.name.clone();
        tell_user(world, id, Msg::CastNoEffectOn { target: name });
        return res;
    }
    let mr = match magres {
        Some(c) if c != 0 => c,
        _ => monster_magic_resistance(
            monster_ability(world, m, Ability::MagicResist),
            kmon.magicres,
            None,
        ),
    };
    let tgtname = m.name.clone();
    let caster = match plr(world, id) {
        Some(p) => CasterInfo::of(world, p),
        None => return res,
    };
    let effects = SpellEffects::from_record(&spl);
    res.outcome = CastOutcome::Cast;
    let mut tries = 0;
    let mut killed = false;
    let mut resval = 0;
    // once per cast
    let mut endspell: Option<SpellNo> = None;
    let mut endchance = 100;
    while !done && !killed && tries < MAXTRIES {
        let mut told = Told::default();
        if manual {
            done = true;
        } else {
            success = false;
        }
        if kai {
            success = true;
        }
        tries += 1;
        let roll = world.rng.range(0, 100);
        if !manual {
            if spl.spell_type == SpellType::General {
                done = true;
                let Some(p) = plr_mut(world, id) else {
                    return res;
                };
                if p.settings.contains(PlayerSettings::CANCAST) {
                    p.settings.remove(PlayerSettings::CANCAST);
                    let p = p.clone();
                    if roll_succeeds(&p, &spl, roll) {
                        success = true;
                    }
                } else {
                    tell_user(
                        world,
                        id,
                        Msg::CastAlreadyThisRound {
                            what: round_word(kai),
                        },
                    );
                    res.outcome = CastOutcome::Refused;
                    return res;
                }
            } else {
                if !opts.autocombat && evil_act(world, id, mon, false) {
                    res.outcome = CastOutcome::Refused;
                    return res;
                }
                // combat starts
                if !opts.autocombat && spl.duration == 0 {
                    tell_user(world, id, Msg::CombatEngaged);
                    mon = guard_redirect(world, id, mon, &kmon);
                    let Some(m) = combat::monster(world, mon) else {
                        res.outcome = CastOutcome::Refused;
                        return res;
                    };
                    let mname = m.name.clone();
                    tell_room(
                        world,
                        key,
                        [Some(id), None],
                        Noise::Always,
                        TellBlind::All,
                        true,
                        None,
                        &Msg::CastMovesToCast {
                            who: caster.name.clone(),
                            spell: spl.name.clone(),
                            target: mname,
                        },
                    );
                    if let Some(p) = plr_mut(world, id) {
                        p.energy = 0;
                    }
                    turn_roll(world, id, mon);
                    res.outcome = CastOutcome::Engaged;
                    engage_m(&mut res, mon);
                    return res;
                }
                let p = plr(world, id).cloned();
                if p.is_some_and(|p| roll_succeeds(&p, &spl, roll)) {
                    success = true;
                }
            }
        }
        // the element defence (kept from the last success, sic)
        if success
            && is_attack_spell(&spl)
            && let Some(m) = combat::monster(world, mon)
        {
            resval = monster_element_resist(world, m, &spl);
        }
        let Some(p) = plr(world, id) else {
            return res;
        };
        if p.energy >= spl.energy && p.mana >= spl.mana {
            let anti = combat::monster(world, mon)
                .is_some_and(|m| monster_has(world, m, Ability::AntiMagic));
            let resisted = success && monster_resists_single(&spl, mr, anti, &mut world.rng);
            if resisted {
                if let Some(s) = world.session_mut(id) {
                    s.output.push(Msg::CastResisted {
                        spell: spl.name.clone(),
                        target: tgtname.clone(),
                    });
                    s.push_prompt();
                }
                tell_room(
                    world,
                    key,
                    [Some(id), None],
                    Noise::Medium,
                    TellBlind::All,
                    true,
                    None,
                    &Msg::CastOtherResisted {
                        target: tgtname.clone(),
                        who: caster.name.clone(),
                        spell: spl.name.clone(),
                    },
                );
                charge_failure(world, id, &spl);
            } else if success {
                if let Some(p) = plr_mut(world, id) {
                    p.energy = p.energy.saturating_sub(spl.energy);
                    if p.mana > 0 {
                        p.mana -= spl.mana.max(0);
                    }
                }
                let rolled = roll_amount(&spl, caster.level, &mut world.rng);
                let power = (i64::from(rolled) * i64::from(100 - resval) / 100) as i32;
                let mut state = CastState::new();
                for slot in effects.iter() {
                    if killed {
                        break;
                    }
                    let value = slot.amount(power);
                    killed = monster_slot(
                        world, id, mon, &spl, slot, value, power, mr, &tgtname, &caster, &mut told,
                        &mut state,
                    );
                }
                if let Some(n) = state.chain.filter(|n| n.0 != 0) {
                    endspell = Some(n);
                    endchance = state.chain_chance;
                }
            } else {
                charge_failure(world, id, &spl);
                fail_lines(world, id, &spl);
            }
        } else {
            if opts.autocombat {
                if let Some(p) = plr_mut(world, id)
                    && p.energy >= spl.energy
                {
                    p.energy -= spl.energy;
                }
                done = true;
            } else {
                short_lines(world, id, &spl, false);
            }
            if tries == 1 {
                res.outcome = CastOutcome::Refused;
            }
        }
        // rolled only when nothing died and a spell is set; a failed roll leaves it for the
        // next try
        if !killed
            && let Some(next) = endspell
            && world.rng.range(0, 100) < endchance
        {
            cast_monster_target(world, id, next, mon, opts);
            endspell = None;
            endchance = 100;
            done = true;
        }
    }
    if !killed && opts.autocombat {
        if let Some(p) = plr_mut(world, id)
            && p.mana < spl.mana
        {
            p.energy = 0;
        }
        engage_m(&mut res, mon);
    }
    res
}

/// The evil act of hitting a good monster: a Good or Lawful Good monster that does not
/// follow the caster earns the caster evil points; `true` when he backed off at the
/// warning. With `turn`, the monster may turn on him.
fn evil_act(world: &mut World, id: SessionId, mon: ActiveMonsterId, turn: bool) -> bool {
    let (Some(p), Some(m)) = (plr(world, id), combat::monster(world, mon)) else {
        return false;
    };
    if is_good_aligned(m.alignment) && !m.following.eq_ignore_ascii_case(&p.firstname) {
        if add_evil_points(
            world,
            id,
            None,
            i32::from(EVIL_SPELL_PTS),
            EVIL_TICKS,
            EvilKind::Attack,
        ) {
            return true;
        }
        if turn {
            turn_roll(world, id, mon);
        }
    }
    false
}

/// The turn roll T: an uncharmed monster that is not an angel turns on the caster when the
/// roll is under its follow chance or it is Neutral or good, unless it is a guard already
/// following someone.
fn turn_roll(world: &mut World, id: SessionId, mon: ActiveMonsterId) {
    let name = plr(world, id)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    let Some(m) = combat::monster(world, mon) else {
        return;
    };
    if m.flags.contains(ActiveMonsterFlags::CHARMED) || m.montype == MonsterType::Angel {
        return;
    }
    let followpct = i32::from(m.followpct);
    let good = matches!(
        m.alignment,
        Alignment::Neutral | Alignment::Good | Alignment::LawfulGood
    );
    let guard_busy = m.montype == MonsterType::Guard && !m.following.is_empty();
    let roll = world.rng.range(1, 100);
    if (roll < followpct || good)
        && !guard_busy
        && let Some(m) = combat::monster_mut(world, mon)
    {
        m.following = name;
        m.friendly = 0;
    }
}

/// GUARDED: a monster with guards of a known kind in the room has the last of them step in;
/// the caster is told.
fn guard_redirect(
    world: &mut World,
    id: SessionId,
    mon: ActiveMonsterId,
    kmon: &silvermere_types::records::KnownMonster,
) -> ActiveMonsterId {
    let guardnum = kmon
        .abilities
        .iter()
        .rev()
        .find(|(a, _)| *a == Ability::Guarded)
        .map(|(_, v)| *v)
        .unwrap_or(0);
    if guardnum == 0 {
        return mon;
    }
    let name = plr(world, id)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    let Some(m) = combat::monster_mut(world, mon) else {
        return mon;
    };
    let protected = m.name.clone();
    m.following = name.clone();
    let key = RoomKey {
        map: m.map,
        room: m.room,
    };
    let mut protnum = None;
    let ids: Vec<ActiveMonsterId> = world
        .room_state
        .get(&key)
        .map(|rs| rs.monsters.iter().copied().filter(|n| n.0 != 0).collect())
        .unwrap_or_default();
    for n in ids {
        if n == mon {
            continue;
        }
        if let Some(g) = combat::monster_mut(world, n)
            && g.knownnum.0 as i16 == guardnum
        {
            g.following = name.clone();
            protnum = Some(n);
        }
    }
    let Some(guard) = protnum else {
        return mon;
    };
    let gname = combat::monster(world, guard)
        .map(|g| g.name.clone())
        .unwrap_or_default();
    tell_user(
        world,
        id,
        Msg::CastMovesToProtect {
            guard: gname,
            who: protected,
        },
    );
    guard
}

/// One slot of `cast_monster_target`. `true` when the monster died.
#[allow(clippy::too_many_arguments)]
fn monster_slot(
    world: &mut World,
    id: SessionId,
    mon: ActiveMonsterId,
    spl: &Spell,
    slot: &EffectSlot,
    value: i32,
    power: i32,
    mr: i32,
    tgtname: &str,
    caster: &CasterInfo,
    told: &mut Told,
    state: &mut CastState,
) -> bool {
    use SpellEffect as E;
    let lasting = spl.duration != 0;
    let effect = slot.effect;
    match effect {
        E::Marker(_) | E::KillDead | E::DetectMagic | E::RemoveSpell { .. } | E::TextBlock => {
            return false;
        }
        E::ClearItem => {
            crate::scripts::actions::erase_item_from_room(
                world,
                room_of_id(world, id),
                ItemNo(value),
            );
            return false;
        }
        E::EndCast | E::EndCastChance => {
            let stats = caster.stats();
            let cast = Cast {
                spell: spl,
                caster: Some(&stats),
                path: CastPath::Single,
                power,
            };
            apply_effect(slot, &cast, &mut Target::None, state, &mut world.rng);
            return false;
        }
        _ => {}
    }
    // (as the original shows) Component, Guarded, Teleport and unknown slots come out as
    // any unlisted ability
    if takes_default_branch(effect) || matches!(effect, E::Teleport(_)) {
        if !lasting {
            silly(world, id, spl, "Monster");
        } else if !state.told {
            let rounds = silvermere_rules::spells::duration(
                spl,
                Some(silvermere_rules::spells::CasterLevel {
                    level: caster.level,
                    spellduration_pct: caster.spellduration_pct,
                }),
                &mut world.rng,
            );
            if let Some(m) = combat::monster_mut(world, mon) {
                let out = add_spell_to_monster(&mut m.spells, spl.number, rounds, value);
                lasting_monster_lines(world, told, id, tgtname, spl, value, out);
            }
            state.told = true;
        }
        return false;
    }
    let was_told = state.told;
    let out = apply_to_monster(
        world,
        mon,
        caster,
        spl,
        slot,
        CastPath::Single,
        power,
        mr,
        state,
    );
    match out {
        Outcome::Silly => silly(world, id, spl, "Monster"),
        Outcome::Lasting(o) => {
            if !was_told {
                lasting_monster_lines(world, told, id, tgtname, spl, value, o);
            }
        }
        Outcome::Applied { amount: shown, .. } => match effect {
            E::Damage { .. } | E::Slay => {
                if !was_told {
                    display_spell_success(world, told, id, None, spl, tgtname, shown);
                }
                state.told = true;
                return after_monster_damage(world, id, mon);
            }
            E::Heal => {
                // (sic) the roll is shown, and only when something was healed
                if shown != 0 {
                    if !was_told {
                        display_spell_success(world, told, id, None, spl, tgtname, value);
                    }
                    state.told = true;
                }
            }
            E::Poison | E::CurePoison => {
                if !was_told {
                    display_spell_success(world, told, id, None, spl, tgtname, value);
                }
                state.told = true;
            }
            // Enslave and Energy say nothing
            _ => {}
        },
        Outcome::StolenHp { amount: shown } => {
            if let Some(c) = plr_mut(world, id) {
                gain_stolen_hp(c, shown);
            }
            if !was_told {
                display_spell_success(world, told, id, None, spl, tgtname, shown);
            }
            state.told = true;
            return after_monster_damage(world, id, mon);
        }
        Outcome::Summon(n, _) => {
            if !was_told {
                display_spell_success(world, told, id, None, spl, tgtname, value);
            }
            summon(world, id, n, Summoned::Hunts(mon));
        }
        _ => {}
    }
    false
}

// ---------------------------------------------------------------------------------------
// cast_item_target

/// A spell on a carried item. Only Clear Item, Detect Magic and Lore do anything.
pub fn cast_item_target(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    item: ItemNo,
    opts: CastOpts,
) -> CastResult {
    let mut res = CastResult::new(CastOutcome::Refused);
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return res;
    };
    if plr(world, id).is_none() {
        return res;
    }
    let manual = opts.mode == CastMode::Item;
    if !matches!(spl.target, SpellTarget::Item | SpellTarget::Any) {
        tell_user(world, id, Msg::CastNotOnItem);
        return res;
    }
    if super::active::check_confusion(world, id) {
        return res;
    }
    let Some(p) = plr(world, id) else {
        return res;
    };
    if p.hp <= 0 || p.status2.contains(PlayerStatus2::AFRAID) {
        if opts.autocombat {
            display_autocombat_broken(world, id);
        }
        return res;
    }
    let Some(cls) = world.classes.get(&p.class).cloned() else {
        return res;
    };
    let kai = cls.magery == Magery::Kai;
    let mut success = kai;
    if manual {
        if spl.magery != Magery::None && (cls.magery != spl.magery || cls.magelvl < spl.magelvl) {
            return res;
        }
        success = true;
    }
    let key = room_of(p);
    let who = p.firstname.clone();
    res.outcome = CastOutcome::Cast;
    if !(p.energy >= spl.energy && p.mana >= spl.mana && p.level >= spl.level) {
        // still counts as handled
        short_lines(world, id, &spl, true);
        return res;
    }
    let roll = world.rng.range(0, 100);
    {
        let Some(p) = plr_mut(world, id) else {
            return res;
        };
        if spl.spell_type == SpellType::General {
            if p.settings.contains(PlayerSettings::CANCAST) {
                p.settings.remove(PlayerSettings::CANCAST);
                let p = p.clone();
                if roll_succeeds(&p, &spl, roll) {
                    success = true;
                }
            } else {
                tell_user(
                    world,
                    id,
                    Msg::CastAlreadyThisRound {
                        what: round_word(kai),
                    },
                );
                res.outcome = CastOutcome::Refused;
                return res;
            }
        } else if roll_succeeds(&p.clone(), &spl, roll) {
            success = true;
        }
    }
    // the component, even when the roll failed (sic)
    for (ability, value) in spl.abilities.iter().copied() {
        if ability == Ability::Component
            && !use_spell_component(world, id, &spl, ItemNo(i32::from(value)))
        {
            res.outcome = CastOutcome::Refused;
            return res;
        }
    }
    if !success {
        charge_failure(world, id, &spl);
        fail_lines(world, id, &spl);
        return res;
    }
    // (sic) the raw energy and the mana without the `> 0` guard
    charge(world, id, i32::from(spl.energy), i32::from(spl.mana).max(0));
    let caster = match plr(world, id) {
        Some(p) => CasterInfo::of(world, p),
        None => return res,
    };
    let power = roll_amount(&spl, caster.level, &mut world.rng);
    let Some(itm) = world.items.get(&item).cloned() else {
        return res;
    };
    let mut state = CastState::new();
    for slot in SpellEffects::from_record(&spl).iter() {
        let stats = caster.stats();
        let cast = Cast {
            spell: &spl,
            caster: Some(&stats),
            path: CastPath::Single,
            power,
        };
        let mut target = Target::Item(ItemTarget { item: &itm });
        let out = apply_effect(slot, &cast, &mut target, &mut state, &mut world.rng);
        let room_line = |world: &mut World| {
            tell_room(
                world,
                key,
                [Some(id), None],
                Noise::Medium,
                TellBlind::All,
                true,
                None,
                &Msg::CastItemRoom {
                    who: who.clone(),
                    spell: spl.name.clone(),
                    item: itm.name.clone(),
                },
            );
        };
        match out {
            Outcome::Silly => silly(world, id, &spl, "Item"),
            Outcome::ClearItem(n) => {
                crate::scripts::actions::erase_item_from_room(world, key, ItemNo(n));
            }
            Outcome::DetectMagic { magical } => {
                let name = itm.name.clone();
                let msg = match magical {
                    1 => Msg::CastDetectFaint { item: name },
                    2 | 3 => Msg::CastDetectSoft { item: name },
                    4 | 5 => Msg::CastDetectBright { item: name },
                    m if m >= 6 => Msg::CastDetectBlinding { item: name },
                    _ => Msg::CastDetectNone,
                };
                tell_user(world, id, msg);
                room_line(world);
            }
            Outcome::Lore { revealed } => {
                tell_user(world, id, Msg::CastLoreGrey);
                if revealed && !itm.hiddesc[0].is_empty() {
                    for line in itm.hiddesc.iter().filter(|l| !l.is_empty()) {
                        tell_user(world, id, Msg::CastLoreLine { text: line.clone() });
                    }
                } else {
                    tell_user(world, id, Msg::CastLoreNothing);
                }
                room_line(world);
            }
            _ => {}
        }
    }
    res
}

#[cfg(test)]
mod tests;
