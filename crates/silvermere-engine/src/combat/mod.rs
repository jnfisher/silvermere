//! Combat orchestration (T-033): one attack between two fighters, from the checks before
//! `*Combat Engaged*` to the kill, as the original does it.
//!
//! - [`attack`]: `attack_user_monster`, `attack_monster_user`, `attack_user_user` and
//!   `attack_monster_monster`. Each blow is rolled by `rules::combat::resolve`; the
//!   fighters are built by `rules::combat::{player_fighter, monster_fighter}` and the verbs
//!   picked here ([`Verbs`]).
//! - [`messages`]: the you/victim/room texts of a blow ([`messages::CombatMsg`]).
//! - [`kill_user`]: `check_kill_user` and `knock_out_user`, returning [`KillOutcome`].
//! - [`kill_monster`]: `check_kill_monster` and the experience hand-over.
//! - [`rob`]: `rob_user`, `rob_monster`, `monster_rob_user` and ROB.
//! - [`autocombat`]: the auto-combat ring, `do_autocombat` and the energy round (T-034).
//! - [`experience_split`]: `distribute_experience` (T-034).
//!
//! The command front half (ATTACK, BASH, SMASH, KICK, ...) is `commands::combat`.
//!
//! Work other cards own is called through the functions at the end of this module, each a
//! "nothing happens" stand-in until its card lands: monster spells and confusion (T-036,
//! T-049, T-050) and item auto-use (T-053); the evil law is T-038's [`crate::evil`]. The
//! attack move, attack type and auto-combat flag are arguments here ([`Move`], [`Mode`]),
//! not globals.

pub mod attack;
pub mod autocombat;
pub mod experience_split;
pub mod kill_monster;
pub mod kill_user;
pub mod messages;
pub mod rob;

use silvermere_rules::abilities::{AbilityLookup, CharacterAbilities, MonsterAbilities};
use silvermere_rules::combat::{
    self, AlignGroup, Blow, Fighter, Move, monster_fighter, player_fighter,
};
use silvermere_rules::rng::SeededRng;
use silvermere_text::component::{self, Which};
use silvermere_types::enums::{Ability, DebugLevel};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MessageNo, SessionId};
use silvermere_types::records::{ActiveMonster, Character, Item, KnownMonster};

use crate::commands::parse::eq_ignore_case;
use crate::world::{ActiveSlot, World};

pub use attack::{
    Mode, attack_monster_monster, attack_monster_user, attack_user_monster, attack_user_user,
};
pub use kill_monster::check_kill_monster;
pub use kill_user::{KillOutcome, check_kill_user, knock_out_user};

/// The movement hooks combat fills (`movement::CombatHooks`): the monsters' free swing
/// at a player walking in is a round of [`attack_monster_user`] (T-033); the auto-combat
/// queries and stops are [`autocombat`]'s (T-034); `is_in_retaliation` is T-038's evil
/// timers.
pub struct Hooks;

impl crate::movement::CombatHooks for Hooks {
    fn in_autocombat(&self, world: &World, id: SessionId) -> bool {
        autocombat::is_inside_autocombat(world, id)
    }
    fn being_attacked(&self, world: &World, id: SessionId) -> bool {
        autocombat::is_being_attacked(world, id)
    }
    fn monster_could_attack(&self, world: &mut World, id: SessionId) -> bool {
        monster_could_attack(world, id)
    }
    fn kill_autocombat(&self, world: &mut World, id: SessionId) {
        autocombat::kill_autocombat(world, id);
    }
    fn kill_autocombat_against(&self, world: &mut World, id: SessionId) {
        autocombat::kill_autocombat_against(world, id);
    }
    fn display_autocombat_broken(&self, world: &mut World, id: SessionId) {
        autocombat::display_autocombat_broken(world, id, false);
    }
    fn clear_backstab_attack_type(&self, world: &mut World, id: SessionId) {
        autocombat::clear_backstab_attack_type(world, id);
    }
    // T-038: `is_in_retaliation`
    fn in_retaliation(&self, world: &World, id: SessionId) -> bool {
        crate::evil::is_in_retaliation(world, id)
    }
    fn attack_monster_user(&self, world: &mut World, monster: ActiveMonsterId, id: SessionId) {
        attack_monster_user(world, monster, id);
    }
}

/// Would a monster in the player's room attack him? Asked before CLOSE, LOCK, HIDE and
/// SNEAK.
///
/// A sneaker who only just came in is never attacked. Otherwise the room's monster slots
/// are walked in order: a charmed monster following him and a friendly one leave him alone;
/// one with "Do not Attack if Item Nmbr" whose value is his `lastmon` counts `lastmoncnt`
/// down and is hostile, else holds back when he carries that item (inventory or keys); any
/// other monster is hostile. Kept (sic): the first such monster ends the walk with `false`,
/// so it makes the room safe whether or not the item is carried. Empty slots are already 0
/// here and need no clearing.
pub fn monster_could_attack(world: &mut World, id: SessionId) -> bool {
    let Some(p) = player(world, id) else {
        return false;
    };
    if p.status.contains(PlayerStatus::SNEAKING) && p.status.contains(PlayerStatus::JUSTARRIVED) {
        return false;
    }
    let key = room_of(p);
    let Some(slots) = world.room_state.get(&key).map(|r| r.monsters) else {
        return false;
    };
    for n in slots {
        if n.0 == 0 {
            continue;
        }
        let Some(mon) = monster(world, n).cloned() else {
            continue;
        };
        let Some(p) = player(world, id) else {
            return false;
        };
        if mon.flags.contains(ActiveMonsterFlags::CHARMED)
            && eq_ignore_case(&p.firstname, &mon.following)
        {
            continue;
        }
        if mon.friendly != 0 {
            continue;
        }
        if !monster_has(world, &mon, Ability::NoAttackItem) {
            return true;
        }
        let itemnum = monster_ability(world, &mon, Ability::NoAttackItem);
        if itemnum == p.lastmon {
            if let Some(p) = player_mut(world, id) {
                p.lastmoncnt = p.lastmoncnt.wrapping_sub(1);
            }
            return true;
        }
        // carried, the monster holds back; not carried, the spent counter ends the walk
        // (sic): either way the room counts as safe
        return false;
    }
    false
}

/// One side of a fight: a player or a monster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combatant {
    Player(SessionId),
    Monster(ActiveMonsterId),
}

/// The three verb forms of a fighter's hits and misses: first person ("smack"), second
/// ("smacks you", text 2) and third ("smacks"), and the weapon's name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verbs {
    pub hit: [String; 3],
    pub miss: [String; 3],
    pub weapon: String,
}

/// The verb-component picker over the world's random source.
pub struct PickRng<'a>(pub &'a mut SeededRng);

impl component::Rng for PickRng<'_> {
    fn below(&mut self, n: u8) -> u8 {
        silvermere_rules::rng::Rng::range(self.0, 0, i32::from(n)) as u8
    }
}

/// The slot of the active monster numbered `n`.
pub fn monster_slot(world: &World, n: ActiveMonsterId) -> Option<ActiveSlot> {
    if n.0 == 0 {
        return None;
    }
    // the cache walk (T-086), then the number index, not a scan: the start-up fill (T-076)
    // makes thousands of look-ups
    world.active_monsters.touch(n);
    world.active_monsters.slot_of(n)
}

/// The active monster numbered `n`.
pub fn monster(world: &World, n: ActiveMonsterId) -> Option<&ActiveMonster> {
    monster_slot(world, n).and_then(|k| world.active_monsters.get(k))
}

/// The active monster numbered `n`, to change.
pub fn monster_mut(world: &mut World, n: ActiveMonsterId) -> Option<&mut ActiveMonster> {
    monster_slot(world, n).and_then(|k| world.active_monsters.get_mut(k))
}

/// The playing character of a session.
pub fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

/// The playing character of a session, to change.
pub fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

/// Whether the session has debug output on.
pub fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// A character's ability value from every source.
pub fn user_ability(world: &World, plr: &Character, ability: Ability) -> i32 {
    lookup_user(world, plr, |l| l.value(ability))
}

/// Whether a character has the ability from any source.
pub fn user_has(world: &World, plr: &Character, ability: Ability) -> bool {
    lookup_user(world, plr, |l| l.has(ability))
}

fn lookup_user<T>(
    world: &World,
    plr: &Character,
    f: impl FnOnce(&CharacterAbilities<'_>) -> T,
) -> T {
    let spells = |n| world.spells.get(&n);
    let items = |n| world.items.get(&n);
    let l = CharacterAbilities {
        character: plr,
        race: world.races.get(&plr.race),
        class: world.classes.get(&plr.class),
        spells: &spells,
        items: &items,
    };
    f(&l)
}

/// A monster's ability value.
pub fn monster_ability(world: &World, mon: &ActiveMonster, ability: Ability) -> i32 {
    lookup_monster(world, mon, |l| l.value(ability))
}

/// Whether a monster has the ability.
pub fn monster_has(world: &World, mon: &ActiveMonster, ability: Ability) -> bool {
    lookup_monster(world, mon, |l| l.has(ability))
}

fn lookup_monster<T>(
    world: &World,
    mon: &ActiveMonster,
    f: impl FnOnce(&MonsterAbilities<'_>) -> T,
) -> T {
    let spells = |n| world.spells.get(&n);
    let items = |n| world.items.get(&n);
    let l = MonsterAbilities {
        monster: mon,
        known: world.monsters.get(&mon.knownnum),
        spells: &spells,
        items: &items,
    };
    f(&l)
}

/// An item's ability value: the first slot's value, 0 without it.
pub fn item_ability(itm: &Item, ability: Ability) -> i32 {
    silvermere_rules::abilities::item_ability_value(itm, ability)
}

/// Whether an item has the ability.
pub fn item_has(itm: &Item, ability: Ability) -> bool {
    silvermere_rules::abilities::item_has_ability(itm, ability)
}

/// The ability totals a player's fighter is built from.
fn player_abilities(world: &World, plr: &Character) -> combat::PlayerAbilities {
    lookup_user(world, plr, |l| combat::PlayerAbilities {
        dodge: l.value(Ability::Dodge),
        def_vs_good: l.value(Ability::DefVsGood),
        def_vs_evil: l.value(Ability::DefVsEvil),
        shadow_stealth: l.has(Ability::ShadowStealth),
        punch: l.value(Ability::Punch),
        kick: l.value(Ability::Kick),
        jumpkick: l.value(Ability::Jumpkick),
        punch_av: l.value(Ability::PunchAv),
        punch_dmg: l.value(Ability::PunchDmg),
        kick_av: l.value(Ability::KickAv),
        kick_dmg: l.value(Ability::KickDmg),
        jumpkick_av: l.value(Ability::JumpkickAv),
        jumpkick_dmg: l.value(Ability::JumpkickDmg),
        backstab_av: l.value(Ability::BackstabAv),
        backstab_min: l.value(Ability::BackstabMin),
        backstab_max: l.value(Ability::BackstabMax),
    })
}

/// The ability totals a monster's fighter is built from.
fn monster_abilities(world: &World, mon: &ActiveMonster) -> combat::MonsterAbilities {
    lookup_monster(world, mon, |l| combat::MonsterAbilities {
        def_vs_good: l.value(Ability::DefVsGood),
        def_vs_evil: l.value(Ability::DefVsEvil),
        shadow_stealth: l.has(Ability::ShadowStealth),
        dodge: l.value(Ability::Dodge),
        av_bless: l.value(Ability::AvBless),
        av_curse: l.value(Ability::AvCurse),
        av_third: l.value(Ability::AvThird),
        defence: l.value(Ability::Defence),
        damage: l.value(Ability::Damage),
        damage_resist: l.value(Ability::DamageResist),
        defence_mod: l.value(Ability::DefenceMod),
        energyuse: l.value(Ability::EnergyUse),
    })
}

/// A roll whose result nothing reads, kept for the random sequence.
pub fn pick_random_attack_move(rng: &mut SeededRng) {
    let _ = silvermere_rules::rng::Rng::range(rng, 0, 100);
}

/// The three texts of a WCCMSG record, or `None`.
fn message(world: &World, n: MessageNo) -> Option<[String; 3]> {
    if n.0 == 0 {
        return None;
    }
    world.messages.get(&n).map(|m| m.msgs.clone())
}

/// Three verb forms picked from their alternatives: the first at random, the other two the
/// same alternative.
fn pick3(rng: &mut SeededRng, texts: [&str; 3]) -> [String; 3] {
    let mut r = PickRng(rng);
    let first = component::pick(Some(texts[0]), Which::Random, &mut r);
    let which = Which::Index(first.chosen);
    let second = component::pick(Some(texts[1]), which, &mut r).text;
    let third = component::pick(Some(texts[2]), which, &mut r).text;
    [first.text, second, third]
}

/// The player's light level for the blindness test.
fn light_level(world: &World, id: SessionId) -> i32 {
    crate::display::light::light_at(world, id).map_or(0, |(l, _)| l)
}

/// A player's side of a fight, with the verbs, for the move `mv` of type `mtype`. `None`
/// when the class is missing. The verb picks roll the world's random source before the
/// attack values roll.
pub fn player_side(
    world: &mut World,
    id: SessionId,
    plr: &Character,
    mv: Move,
    mtype: Move,
    style: AlignGroup,
) -> Option<(Fighter, Verbs)> {
    let class = world.classes.get(&plr.class)?.clone();
    let weapon = if plr.weapon.0 != 0 {
        world.items.get(&plr.weapon).cloned()
    } else {
        None
    };
    let armour: Vec<Item> = plr
        .worn
        .iter()
        .filter(|n| n.0 != 0)
        .filter_map(|n| world.items.get(n).cloned())
        .collect();
    let armour_refs: Vec<&Item> = armour.iter().collect();
    let abilities = player_abilities(world, plr);
    let blow = Blow {
        atkmove: mv,
        atktype: mtype,
        style,
        light_level: light_level(world, id),
    };
    let fighter = player_fighter(plr, &class, weapon.as_ref(), &armour_refs, abilities, blow);
    let verbs = player_verbs(world, plr, weapon.as_ref(), mv);
    Some((fighter, verbs))
}

/// The verb part of [`player_side`].
fn player_verbs(world: &mut World, plr: &Character, weapon: Option<&Item>, mv: Move) -> Verbs {
    let mut v = Verbs::default();
    let mut missmsg = None;
    let rng = &mut world.rng;
    match mv {
        Move::Punch => {
            v.weapon = "fists of fury".into();
            v.hit = pick3(rng, ["punch", "punches", "punches"]);
        }
        Move::Kick => {
            v.weapon = "lightning feet".into();
            v.hit = pick3(rng, ["kick", "kicks", "kicks"]);
        }
        Move::JumpKick => {
            v.weapon = "flying feet".into();
            v.hit = pick3(rng, ["jumpkick", "jumpkicks", "jumpkicks"]);
        }
        _ if plr.weapon.0 != 0 => match weapon {
            Some(w) => {
                v.weapon = w.name.clone();
                let hitmsg = world
                    .messages
                    .get(&w.hitmsg)
                    .filter(|_| w.hitmsg.0 != 0)
                    .map(|m| m.msgs.clone());
                v.hit = match &hitmsg {
                    Some(m) => pick3(rng, [&m[0], &m[1], &m[2]]),
                    None => pick3(rng, ["attack", "attacks", "attacks"]),
                };
                missmsg = world
                    .messages
                    .get(&w.missmsg)
                    .filter(|_| w.missmsg.0 != 0)
                    .map(|m| m.msgs.clone());
                if let Some(m) = &missmsg {
                    v.miss = pick3(rng, [&m[0], &m[1], &m[2]]);
                }
            }
            None => {
                // the weapon's record is gone: fists (clearing the weapon slot is left to the
                // item code).
                v.weapon = "fists".into();
                v.hit = pick3(rng, ["punch", "punches", "punches"]);
            }
        },
        _ => {
            v.weapon = "fists".into();
            v.hit = pick3(rng, ["punch", "punches", "punches"]);
        }
    }
    // a backstab "surprise"s
    if mv == Move::Backstab {
        for h in &mut v.hit {
            *h = format!("surprise {h}");
        }
    }
    // no miss verbs of its own: a weapon's hit verbs, bare hands "swing at"
    if missmsg.is_none() {
        if plr.weapon.0 != 0 {
            v.miss = v.hit.clone();
        } else {
            v.miss = pick3(rng, ["swing at", "swings at", "swings at"]);
        }
    }
    v
}

/// A monster's side of a fight, with the verbs of its attack slot (copied whole, no
/// component pick). `None` without a known-monster record.
pub fn monster_side(
    world: &World,
    mon: &ActiveMonster,
    slot: usize,
    mv: Move,
    style: AlignGroup,
) -> Option<(Fighter, Verbs)> {
    let kmon = world.monsters.get(&mon.knownnum)?;
    let weapon = if mon.weapon.0 != 0 {
        world.items.get(&mon.weapon)
    } else {
        None
    };
    let abilities = monster_abilities(world, mon);
    let fighter = monster_fighter(mon, kmon, slot, weapon, abilities, Blow::new(mv, style));
    let mut v = Verbs::default();
    let mut missmsg = None;
    if let Some(w) = weapon {
        v.weapon = w.name.clone();
        if let Some(m) = message(world, w.hitmsg) {
            v.hit = m;
        }
        missmsg = message(world, w.missmsg);
        if let Some(m) = &missmsg {
            v.miss = m.clone();
        }
    }
    if missmsg.is_none() {
        v.miss = v.hit.clone();
    }
    Some((fighter, v))
}

/// A character's alignment group.
pub fn user_group(plr: &Character) -> AlignGroup {
    combat::user_align_group(plr.evil)
}

/// A known monster's alignment group.
pub fn monster_group(kmon: &KnownMonster) -> AlignGroup {
    combat::monster_align_group(kmon.alignment)
}

/// The player's room key.
pub fn room_of(plr: &Character) -> silvermere_types::ids::RoomKey {
    silvermere_types::ids::RoomKey {
        map: plr.map,
        room: plr.room,
    }
}

/// The subject pronoun.
pub fn subject_pronoun(plr: Option<&Character>) -> &'static str {
    match plr.map(|p| p.sex) {
        Some(b'F') => "she",
        Some(b'M') => "he",
        Some(_) => "it",
        None => "He",
    }
}

/// The object pronoun.
pub fn object_pronoun(plr: Option<&Character>) -> &'static str {
    match plr.map(|p| p.sex) {
        Some(b'F') => "her",
        Some(b'M') => "him",
        Some(_) => "it",
        None => "He",
    }
}

/// The possessive pronoun.
pub fn possessive_pronoun(plr: Option<&Character>) -> &'static str {
    match plr.map(|p| p.sex) {
        Some(b'F') => "her",
        Some(b'M') => "his",
        Some(_) => "its",
        None => "He",
    }
}

// ---------------------------------------------------------------------------------------
// Stand-ins for other cards' work. Each is the "nothing happens" answer until its card
// lands; each call site is where its effect falls in the order of events.

/// What [`engage_autocombat`] queues the attacker against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTarget {
    Player(SessionId),
    Monster(ActiveMonsterId),
}

/// Queue the attacker's next round against `target` with the move in progress `mv`
/// ([`autocombat::engage`]).
pub fn engage_autocombat(world: &mut World, id: SessionId, target: AutoTarget, mv: Move) {
    autocombat::engage(world, id, Some(target), mv, None);
}

/// The caster fights on, casting `spell` each round (`spells::cast`). The entry's move
/// stays what it was (the move the last attack left).
pub fn engage_autocombat_spell(
    world: &mut World,
    id: SessionId,
    target: Option<AutoTarget>,
    spell: silvermere_types::ids::SpellNo,
) {
    let mv = world.autocombat.entry(id).mv;
    autocombat::engage(world, id, target, mv, Some(spell));
}

/// Whether the player is in auto-combat.
pub fn is_inside_autocombat(world: &World, id: SessionId) -> bool {
    autocombat::is_inside_autocombat(world, id)
}

/// Whether the player is in auto-combat against `target`.
pub fn is_combatting_user(world: &World, id: SessionId, target: SessionId) -> bool {
    autocombat::is_combatting_user(world, id, target)
}

/// `*Combat Off*`, to the player only.
pub fn display_autocombat_broken(world: &mut World, id: SessionId) {
    autocombat::display_autocombat_broken(world, id, false);
}

/// The player's auto-combat ends, untold.
pub fn kill_autocombat(world: &mut World, id: SessionId) {
    autocombat::kill_autocombat(world, id);
}

/// Everyone fighting the monster stops, untold.
pub fn kill_autocombat_against_monster(world: &mut World, monster: ActiveMonsterId) {
    autocombat::kill_autocombat_against_monster(world, monster);
}

/// CAST alone repeats the auto-combat spell; `true` when it did.
pub fn restart_autocombat_spell(world: &mut World, id: SessionId) -> bool {
    autocombat::restart_autocombat(world, id, true)
}

/// ATTACK or BASH with no target.
pub fn restart_autocombat(world: &mut World, id: SessionId) -> bool {
    autocombat::restart_autocombat(world, id, false)
}

/// T-056's [`crate::session::cleanup_when_user_leaves`].
pub fn cleanup_when_user_leaves(world: &mut World, id: SessionId) {
    crate::session::cleanup_when_user_leaves(world, id);
}

/// The kinds of [`add_evil_points`].
pub use silvermere_rules::evil::Kind as EvilKind;

/// `true` when the act is refused (the player has been told why). T-038's
/// [`crate::evil::add_evil_points`], the points and timer narrowed to `i16` and `i8`.
pub fn add_evil_points(
    world: &mut World,
    id: SessionId,
    victim: Option<SessionId>,
    points: i32,
    timer: i32,
    kind: EvilKind,
) -> bool {
    crate::evil::add_evil_points(world, id, victim, points as i16, timer as i8, kind)
}

/// Whether the first holds a timer on the second (T-038's [`crate::evil::already_evil`]).
pub fn already_evil(world: &World, attacker: SessionId, victim: SessionId) -> bool {
    crate::evil::already_evil(world, attacker, victim).is_some()
}

/// A confused player loses the round ([`crate::spells::active::check_confusion`]).
pub fn check_confusion(world: &mut World, id: SessionId) -> bool {
    crate::spells::active::check_confusion(world, id)
}

/// Whether a confused monster loses the round
/// ([`crate::spells::active::check_monster_confusion`]).
pub fn check_monster_confusion(world: &mut World, monster: ActiveMonsterId) -> bool {
    crate::spells::active::check_monster_confusion(world, monster)
}

/// A monster's spell at the player: 0 nothing cast, 2 the player died, 1 anything else
/// ([`crate::monsters::cast::cast_at_player`], T-052).
pub fn monster_cast(
    world: &mut World,
    monster: ActiveMonsterId,
    slot: Option<usize>,
    id: SessionId,
    spell: i32,
) -> i32 {
    crate::monsters::cast::cast_at_player(world, monster, slot, id, spell).code()
}

/// A monster's random spell at the player ([`crate::monsters::cast::cast_random`], T-052).
pub fn monster_cast_random_spell(world: &mut World, monster: ActiveMonsterId, id: SessionId) {
    crate::monsters::cast::cast_random(world, monster, id);
}

/// An Auto Use weapon fires its abilities ([`crate::items::use_item::auto_use_on_hit`],
/// T-053); returns the spell result (2 a kill).
pub fn auto_use(world: &mut World, id: SessionId, item: ItemNo, target: Combatant) -> i32 {
    crate::items::use_item::auto_use_on_hit(world, id, item, target)
}

/// The attack-interrupt flush: the victim's waiting output is thrown away so the warning is
/// seen at once. The kernel's input injection is not carried; the output batch is cleared
/// here.
pub fn interrupt_output(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.output.clear();
    }
}
