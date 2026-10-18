//! CAST and INVOKE, a spell typed by its name (stage 9 of `execute_input`) and PURGE
//! ([`crate::spells::spellbook::cmd_purge`]).
//!
//! The command finds the spell in the spellbook, checks that the character may cast it,
//! finds the target by the kinds the spell may be aimed at (T-049's `match_kinds`), stops the
//! caster's auto-combat (`*Combat Off*` when it ran) and hands the cast to [`crate::spells::cast`].

use silvermere_rules::spells::{MatchKinds, match_kinds};
use silvermere_text::Msg;
use silvermere_types::enums::Magery;
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, SessionId, SpellNo};

use super::Cmd;
use super::dispatch::Handled;
use super::parse::{CommandLine, eq_ignore_case};
use crate::combat::{
    self, display_autocombat_broken, is_inside_autocombat, kill_autocombat, room_of, user_has,
};
use crate::items::inventory::{Search, find_item_in_inventory, matches_any_word, show_options};
use crate::output::clear_output;
use crate::spells::cast::{CastMode, CastOpts, CastTarget, Caster, apply_engage, cast};
use crate::spells::spellbook::{find_spell_in_spellbook, user_can_use_spell};
use crate::tell::tell_user;
use crate::world::World;

/// The most passes a name search makes, dropping a trailing word each time.
const MAX_PASSES: usize = 50;

/// What [`find_action_target`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Nothing.
    None,
    /// A player in the room.
    User(SessionId),
    /// A monster in the room.
    Monster(ActiveMonsterId),
    /// An item carried.
    Inventory(ItemNo),
    /// An item on the floor.
    Floor(ItemNo),
    /// A spell in the spellbook.
    Spell(SpellNo),
    /// Several matches: the candidate of the last kind found, and the "which one?" names.
    Multiple(Box<Found>, Vec<String>),
}

/// Every kind of thing.
pub const ANYTHING: MatchKinds = MatchKinds {
    users: true,
    monsters: true,
    inventory: true,
    floor: true,
    spells: true,
    not_self: false,
    charmed_last: false,
};

/// The thing `name` names, among the kinds `flags` names, in the original's order: the monsters
/// of the room (the uncharmed ones first with `charmed_last`), dropping trailing words a
/// pass at a time; the players of the room by the first word (not the searcher with
/// `not_self`; a hidden one only for a searcher who sees hidden things); the items carried;
/// the items on the floor (a partial name only while nothing else was counted); the
/// spellbook. A full name ends the search at once. More than one candidate is ambiguous
/// unless the last kind found was a monster.
pub fn find_action_target(
    world: &mut World,
    id: SessionId,
    name: &str,
    flags: MatchKinds,
) -> Found {
    let Some(plr) = combat::player(world, id).cloned() else {
        return Found::None;
    };
    let key = room_of(&plr);
    let mut count = 0usize;
    let mut options: Vec<String> = Vec::new();
    let mut found = Found::None;
    if flags.monsters {
        let mons: Vec<_> = world
            .room_state
            .get(&key)
            .map(|rs| rs.monsters.iter().copied().filter(|n| n.0 != 0).collect())
            .unwrap_or_default();
        let passes: &[Option<bool>] = if flags.charmed_last {
            &[Some(false), Some(true)]
        } else {
            &[None]
        };
        for &charmed in passes {
            let mut words: Vec<&str> = name.split(' ').collect();
            let mut dropped = false;
            for _ in 0..MAX_PASSES {
                let typed = words.join(" ");
                for &n in &mons {
                    let Some(m) = combat::monster(world, n) else {
                        continue;
                    };
                    if let Some(c) = charmed
                        && m.flags.contains(ActiveMonsterFlags::CHARMED) != c
                    {
                        continue;
                    }
                    if matches_any_word(&typed, &m.name) {
                        if count == 0 {
                            found = Found::Monster(n);
                        }
                        count += 1;
                        if !dropped && eq_ignore_case(&typed, &m.name) {
                            return Found::Monster(n);
                        }
                        options.push(m.name.clone());
                    }
                }
                if words.len() <= 1 {
                    break;
                }
                words.pop();
                dropped = true;
            }
        }
    }
    if flags.users {
        let first = name.split(' ').next().unwrap_or("");
        let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
        let mut ids = world.session_ids();
        ids.sort();
        for other in ids {
            if (flags.not_self && other == id) || !world.is_in_game(other) {
                continue;
            }
            let Some(v) = combat::player(world, other) else {
                continue;
            };
            if v.room != plr.room || v.map != plr.map || !matches_any_word(first, &v.firstname) {
                continue;
            }
            if v.hidden != 0 && other != id && !sees_hidden {
                continue;
            }
            count += 1;
            found = Found::User(other);
            if eq_ignore_case(first, &v.firstname) {
                return found;
            }
            options.push(v.firstname.clone());
        }
    }
    if flags.inventory {
        let f = find_item_in_inventory(world, &plr, name, Search::Any);
        if let Some(item) = f.item {
            if f.count == 0 {
                return Found::Inventory(item);
            }
            count += f.count;
            options.extend(f.options.iter().cloned());
            found = Found::Inventory(item);
        }
    }
    if flags.floor
        && let Some(st) = world.room_state.get(&key)
    {
        let counted_before = count;
        let mut first: Option<ItemNo> = None;
        for slot in st.items.iter().filter(|s| s.item.0 != 0) {
            let Some(itm) = world.items.get(&slot.item) else {
                continue;
            };
            if !matches_any_word(name, &itm.name) {
                continue;
            }
            if eq_ignore_case(name, &itm.name) {
                return Found::Floor(slot.item);
            }
            if first != Some(slot.item) {
                count += 1;
                options.push(itm.name.clone());
                first.get_or_insert(slot.item);
            }
        }
        if let Some(item) = first
            && counted_before == 0
        {
            found = Found::Floor(item);
        }
    }
    if flags.spells
        && let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take())
    {
        let f = find_spell_in_spellbook(world, &mut c, name, true);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
        if let Some(n) = f.spell {
            if f.count == 0 {
                return Found::Spell(n);
            }
            count += f.count;
            options.extend(f.options);
            found = Found::Spell(n);
        }
    }
    if count > 1 && !matches!(found, Found::Monster(_)) {
        return Found::Multiple(Box::new(found), options);
    }
    found
}

/// CAST and INVOKE typed as commands (the spell name from the second word).
pub fn cmd_cast(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    cast_command(world, id, cmd == Cmd::Invoke, line, 1)
}

/// CAST or INVOKE. `invoke` is true for INVOKE; `argbase` is the word the spell name starts at (1
/// after CAST or INVOKE, 0 for a spell typed by its name).
pub fn cast_command(
    world: &mut World,
    id: SessionId,
    invoke: bool,
    line: &CommandLine,
    argbase: usize,
) -> Handled {
    let Some(plr) = combat::player(world, id) else {
        return Handled::NotCommand;
    };
    // Flee makes a player too afraid
    if plr.status2.contains(PlayerStatus2::AFRAID) {
        tell_user(world, id, Msg::CombatTooAfraid);
        return Handled::Done;
    }
    let Some(cls) = world.classes.get(&plr.class).cloned() else {
        return Handled::NotCommand;
    };
    // an action: no longer just arrived, resting or meditating
    if let Some(p) = combat::player_mut(world, id) {
        p.status
            .remove(PlayerStatus::JUSTARRIVED | PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
    let Some(plr) = combat::player(world, id) else {
        return Handled::NotCommand;
    };
    if invoke {
        // INVOKE is for Kai classes only
        if cls.magery != Magery::Kai {
            let msg = if cls.magery != Magery::None {
                Msg::CastKaiNotKai
            } else {
                Msg::CastNoPowers
            };
            tell_user(world, id, msg);
            return Handled::Done;
        }
        if user_has(world, plr, silvermere_types::enums::Ability::KaiBind) {
            tell_user(world, id, Msg::CastPowerFails);
            return Handled::Done;
        }
        if line.argc() <= 1 && argbase == 1 {
            tell_user(world, id, Msg::CastInvokeSyntax);
            return Handled::Done;
        }
    } else {
        // CAST is for the other casting classes
        if cls.magery == Magery::Kai {
            tell_user(world, id, Msg::CastMustInvoke);
            return Handled::Done;
        }
        if cls.magery == Magery::None {
            tell_user(world, id, Msg::CastNoSpells);
            return Handled::Done;
        }
        if user_has(world, plr, silvermere_types::enums::Ability::Magebind) {
            tell_user(world, id, Msg::CastSpellFails);
            return Handled::Done;
        }
        if line.argc() == 1 && argbase == 1 {
            // CAST alone repeats the auto-combat spell
            stop_autocombat(world, id);
            if combat::restart_autocombat_spell(world, id) {
                return Handled::Done;
            }
            tell_user(world, id, Msg::CastSyntax);
            return Handled::Done;
        }
    }
    // casting gives the caster away
    if let Some(p) = combat::player_mut(world, id) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
    // the spellbook
    let name = line.rest_from(argbase).to_owned();
    let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) else {
        return Handled::NotCommand;
    };
    let f = find_spell_in_spellbook(world, &mut c, &name, argbase != 0);
    if let Some(s) = world.session_mut(id) {
        s.character = Some(c);
    }
    if f.count > 1 {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the buffer is emptied, the dispatcher's colour reset with it (the
    // later clears after the target searches find it empty)
    clear_output(world, id);
    let Some(spl) = f.spell.and_then(|n| world.spells.get(&n)).cloned() else {
        // no such spell in the spellbook
        tell_user(world, id, Msg::CastUnknownSpell { spell: name });
        return Handled::Done;
    };
    let Some(plr) = combat::player(world, id) else {
        return Handled::NotCommand;
    };
    // may the character cast it at all
    if let Err(prefix) = user_can_use_spell(world, plr, &spl) {
        if prefix {
            tell_user(world, id, Msg::CastTooPowerfulPrefix);
        }
        tell_user(world, id, Msg::CastMayNot);
        return Handled::Done;
    }
    let words = f.words;
    let opts = CastOpts {
        mode: CastMode::Cast,
        autocombat: false,
        // (sic) the original always announces a typed cast
        announce: true,
    };
    if line.argc() == argbase + words {
        // no target
        stop_autocombat(world, id);
        let r = cast(
            world,
            Caster::Player(id),
            spl.number,
            CastTarget::None,
            opts,
        );
        apply_engage(world, id, &r);
        return Handled::Done;
    }
    // the target, by the spell's kinds, then by anything
    let tname = line.rest_from(argbase + words).to_owned();
    let mut found = find_action_target(world, id, &tname, match_kinds(&spl));
    if let Found::Multiple(_, options) = found {
        show_options(world, id, &options);
        return Handled::Done;
    }
    if found == Found::None {
        found = match find_action_target(world, id, &tname, ANYTHING) {
            // the second search keeps its candidate and drops the "multiple" flag; the list is
            // thrown away
            Found::Multiple(candidate, _) => *candidate,
            other => other,
        };
    }
    let target = match found {
        Found::None | Found::Multiple(..) => {
            tell_user(world, id, Msg::CastNotHere { what: tname });
            return Handled::Done;
        }
        Found::User(t) => {
            stop_autocombat(world, id);
            let hidden = combat::player(world, t).is_some_and(|v| v.hidden != 0);
            let sees = combat::player(world, id)
                .is_some_and(|p| p.status.contains(PlayerStatus::SEEHIDDEN));
            if hidden && !sees {
                tell_user(world, id, Msg::CastNotHere { what: tname });
                return Handled::Done;
            }
            CastTarget::User(t)
        }
        Found::Monster(m) => {
            stop_autocombat(world, id);
            CastTarget::Monster(m)
        }
        Found::Inventory(i) => {
            stop_autocombat(world, id);
            CastTarget::Item(i)
        }
        Found::Floor(_) => {
            stop_autocombat(world, id);
            tell_user(world, id, Msg::CastNotCarrying { what: tname });
            return Handled::Done;
        }
        Found::Spell(_) => {
            stop_autocombat(world, id);
            tell_user(world, id, Msg::CastWhyThat);
            return Handled::Done;
        }
    };
    let r = cast(world, Caster::Player(id), spl.number, target, opts);
    apply_engage(world, id, &r);
    Handled::Done
}

/// Every cast ends the caster's auto-combat first, so a cast in a fight shows `*Combat Off*` and
/// then the spell's own engage.
fn stop_autocombat(world: &mut World, id: SessionId) {
    if is_inside_autocombat(world, id) {
        display_autocombat_broken(world, id);
    }
    kill_autocombat(world, id);
}

/// Stage 9 of `execute_input`: a line naming a spell in the spellbook
/// (exact short name, or for a spell without one any abbreviation of its name) casts it,
/// with INVOKE for a Kai class. The caller has checked the hit points and paralysis.
pub struct SpellByName;

impl super::input::SpellByName for SpellByName {
    fn cast(&self, world: &mut World, id: SessionId, line: &str) -> bool {
        let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) else {
            return false;
        };
        let f = find_spell_in_spellbook(world, &mut c, line, false);
        let kai = world
            .classes
            .get(&c.class)
            .map(|cls| cls.magery == Magery::Kai);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
        let (Some(_), Some(kai)) = (f.spell, kai) else {
            return false;
        };
        let cl = CommandLine::parse(line);
        cast_command(world, id, kai, &cl, 0) == Handled::Done
    }
}
