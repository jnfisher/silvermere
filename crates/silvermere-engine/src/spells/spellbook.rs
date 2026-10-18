//! The spellbook: finding a spell in it by name ([`find_spell_in_spellbook`]), who may cast
//! what ([`user_can_use_spell`]), PURGE ([`cmd_purge`]) and the Kai powers
//! ([`kai_power_cache`], [`update_kai_powers`]).
//!
//! Learning and forgetting a spell are `items::equip::add_spell_to_spellbook` and
//! `purge_spell_from_spellbook`, re-exported here.
//!
//! Kai classes keep their powers in the same spellbook: `update_kai_powers` writes every
//! Kai power of the character's level or below into it. The list of Kai spells is read from
//! the spell table each time, in spell-number order, at most 60.

use silvermere_rules::spells::{CannotUse, LegalBand, can_use_spell};
use silvermere_text::Msg;
use silvermere_types::enums::Magery;
use silvermere_types::ids::{SessionId, SpellNo};
use silvermere_types::records::{Character, Spell};
use silvermere_types::sizes::NSPELLBOOK;

use crate::commands::Cmd;
use crate::commands::dispatch::Handled;
use crate::commands::parse::{CommandLine, eq_ignore_case};
pub use crate::items::equip::{add_spell_to_spellbook, purge_spell_from_spellbook};
use crate::items::inventory::{matches_any_word, show_options};
use crate::tell::tell_user;
use crate::world::World;

/// The most search passes, one word fewer each time.
const MAX_PASSES: usize = 50;

/// The most Kai powers the cache holds.
pub const NKAICACHE: usize = 60;

/// What [`find_spell_in_spellbook`] gave back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The spell found: the exact match, else the last candidate of the matching pass.
    pub spell: Option<SpellNo>,
    /// How many words of `name` named the spell.
    pub words: usize,
    /// The candidates of the matching pass; 0 after an exact match.
    pub count: usize,
    /// The "which one?" list queued: every abbreviation candidate (the caller shows the
    /// list or drops it).
    pub options: Vec<String>,
}

/// A spell in the spellbook by name. A spell with a short name matches its exact short
/// name, and (only with `partial`) any abbreviation of its full name; a spell without one
/// matches any abbreviation of its name (`matches_any_word`) either way. An exact name ends the
/// search; with no match the last word is dropped and the search runs again, up to
/// [`MAX_PASSES`] passes. A slot naming a spell that no longer exists is cleared on the
/// way.
pub fn find_spell_in_spellbook(
    world: &World,
    plr: &mut Character,
    name: &str,
    partial: bool,
) -> Found {
    if usize::from(plr.nspellslots) > NSPELLBOOK {
        plr.nspellslots = NSPELLBOOK as u8;
    }
    let nslots = usize::from(plr.nspellslots).min(plr.spellbook.len());
    let mut f = Found::default();
    let mut words: Vec<&str> = name.split(' ').collect();
    let mut exact = false;
    let mut matched = false;
    let mut nknown = 0usize;
    let mut passes = 0usize;
    loop {
        passes += 1;
        let cur = words.join(" ");
        for i in 0..nslots {
            if exact {
                break;
            }
            let n = plr.spellbook[i];
            if n == SpellNo(0) {
                continue;
            }
            nknown += 1;
            let Some(spl) = world.spells.get(&n) else {
                plr.spellbook[i] = SpellNo(0);
                continue;
            };
            if !spl.shortnm.is_empty() {
                if eq_ignore_case(&cur, &spl.shortnm) {
                    exact = true;
                    matched = true;
                    f.count += 1;
                    f.spell = Some(spl.number);
                } else if partial && matches_any_word(&cur, &spl.name) {
                    f.count += 1;
                    f.options.push(spl.name.clone());
                    matched = true;
                    f.spell = Some(spl.number);
                    if eq_ignore_case(&cur, &spl.name) {
                        exact = true;
                    }
                }
            } else if matches_any_word(&cur, &spl.name) {
                f.options.push(spl.name.clone());
                f.count += 1;
                matched = true;
                f.spell = Some(spl.number);
                if eq_ignore_case(&cur, &spl.name) {
                    exact = true;
                }
            }
        }
        let can_drop = words.len() > 1;
        if !matched && can_drop {
            words.pop();
        }
        if matched || !can_drop || nknown == 0 || passes >= MAX_PASSES {
            if matched {
                // every blank after the first character starts a word
                f.words = 1 + cur
                    .bytes()
                    .skip(1)
                    .filter(|b| b.is_ascii_whitespace())
                    .count();
            }
            break;
        }
    }
    if exact {
        f.count = 0;
    }
    f
}

/// Whether the player may cast the spell, as CAST asks it: `Err(true)` when the refusal is
/// the level (`This spell is too powerful for you - ` is printed first), `Err(false)` for
/// the others (silent).
pub fn user_can_use_spell(world: &World, plr: &Character, spell: &Spell) -> Result<(), bool> {
    let Some(cls) = world.classes.get(&plr.class) else {
        return Err(false);
    };
    match can_use_spell(spell, cls, plr.level, LegalBand::of_evil(plr.evil)) {
        Ok(()) => Ok(()),
        Err(CannotUse::TooPowerful) => Err(true),
        Err(_) => Err(false),
    }
}

/// `PURGE {spell}` forgets a spell. Only an unambiguous match is purged. As in the
/// original, the "which one?" list goes out with the line (a unique
/// abbreviation shows the list of one first); `PURGE` alone is no command (stage 7 onwards
/// gets the line).
pub fn cmd_purge(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() < 2 {
        return Handled::NotCommand;
    }
    let Some(mut plr) = world.session_mut(id).and_then(|s| s.character.take()) else {
        return Handled::NotCommand;
    };
    let f = find_spell_in_spellbook(world, &mut plr, line.rest_from(1), true);
    let purged = match f.spell.and_then(|n| world.spells.get(&n)) {
        Some(spl) if f.count < 2 => {
            purge_spell_from_spellbook(&mut plr, spl.number);
            Some(spl.name.clone())
        }
        _ => None,
    };
    if let Some(s) = world.session_mut(id) {
        s.character = Some(plr);
    }
    if !f.options.is_empty() {
        show_options(world, id, &f.options);
    }
    if let Some(spell) = purged {
        tell_user(world, id, Msg::SpellPurged { spell });
    }
    Handled::Done
}

/// The Kai power list: the Kai spells in number order; past the
/// 60th each later one overwrites the last slot (sic).
pub fn kai_power_cache(world: &World) -> Vec<SpellNo> {
    let mut numbers: Vec<SpellNo> = world
        .spells
        .values()
        .filter(|s| s.magery == Magery::Kai)
        .map(|s| s.number)
        .collect();
    numbers.sort();
    if numbers.len() > NKAICACHE {
        let last = *numbers.last().unwrap_or(&SpellNo(0));
        numbers.truncate(NKAICACHE - 1);
        numbers.push(last);
    }
    numbers
}

/// Every Kai power of the character's level or below that is not in the spellbook yet goes
/// in, under `You learn the following Kai abilities:` and one line per power. Called by
/// TRAIN for a Kai class.
pub fn update_kai_powers(world: &mut World, id: SessionId) {
    let cache = kai_power_cache(world);
    let mut first = true;
    for n in cache {
        let Some(spl) = world.spells.get(&n) else {
            continue;
        };
        let (number, level, name) = (spl.number, spl.level, spl.name.clone());
        let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return;
        };
        if level > plr.level || !add_spell_to_spellbook(plr, number) {
            continue;
        }
        if first {
            first = false;
            tell_user(world, id, Msg::KaiLearned);
        }
        tell_user(world, id, Msg::KaiRow { name });
    }
}
