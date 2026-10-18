//! SPELLS and POWERS: [`display_users_spells`] over [`sort_spell_list`].

use silvermere_text::Msg;
use silvermere_types::enums::{CommandCode as Cmd, Magery};
use silvermere_types::ids::{SessionId, SpellNo};
use silvermere_types::records::Spell;

use super::status::{left, num, right};
use crate::world::World;

/// An exchange sort of the book by spell level, then name (byte order), that starts over
/// from slot 0 after each swap and gives up after 500 swaps. Only neighbouring slots that
/// both hold a known spell are compared, so empty slots and vanished spells split the book
/// into runs.
pub fn sort_spell_list(book: &mut [SpellNo], spell: impl Fn(SpellNo) -> Option<(u16, String)>) {
    let mut swaps = 0;
    let mut i = 0;
    while i + 1 < book.len() && swaps < 500 {
        if book[i].0 != 0
            && book[i + 1].0 != 0
            && let (Some((l1, n1)), Some((l2, n2))) = (spell(book[i]), spell(book[i + 1]))
            && (l2 < l1 || (l2 == l1 && n2 < n1))
        {
            book.swap(i, i + 1);
            swaps += 1;
            i = 0;
            continue;
        }
        i += 1;
    }
}

/// The SPELLS or POWERS list for the character on `shown`, sent to `to`: Kai classes must
/// use POWERS and the others SPELLS; the book is sorted, a vanished spell taken out of it,
/// and each spell listed with its level, cost, short and full name under the heading, or
/// `You have no spells.` (powers).
pub fn display_users_spells(world: &mut World, shown: SessionId, to: SessionId, cmd: Cmd) {
    let Some(plr) = world.session(shown).and_then(|s| s.player()) else {
        return;
    };
    let Some(cls) = world.classes.get(&plr.class) else {
        return;
    };
    let kai = cls.magery == Magery::Kai;
    let powers = cmd == Cmd::Powers;
    let refusal = match (powers, kai) {
        (false, true) => Some(Msg::SpellsKaiRefused),
        (true, false) => Some(Msg::PowersRefused),
        _ => None,
    };
    if let Some(m) = refusal {
        if let Some(s) = world.session_mut(to) {
            s.output.push(m);
        }
        return;
    }
    let slots = usize::from(plr.nspellslots).min(plr.spellbook.len());
    let mut book: Vec<SpellNo> = plr.spellbook[..slots].to_vec();
    let spells = &world.spells;
    sort_spell_list(&mut book, |n| {
        spells.get(&n).map(|s: &Spell| (s.level, s.name.clone()))
    });
    let what = if powers { "powers" } else { "spells" };
    let mut msgs = Vec::new();
    for slot in book.iter_mut() {
        if slot.0 == 0 {
            continue;
        }
        // A spell gone from WCCSPELS is forgotten
        let Some(spl) = world.spells.get(slot) else {
            *slot = SpellNo(0);
            continue;
        };
        if msgs.is_empty() {
            msgs.push(Msg::SpellsHeader { what: what.into() });
            msgs.push(Msg::SpellsColumns {
                cost: if powers { "Kai " } else { "Mana" }.into(),
            });
        }
        msgs.push(Msg::SpellRow {
            level: num(spl.level, 3, false),
            mana: num(spl.mana, 4, true),
            short: right(&spl.shortnm, 4),
            name: left(&spl.name, 30),
        });
    }
    msgs.push(if msgs.is_empty() {
        Msg::SpellsNone { what: what.into() }
    } else {
        Msg::SpellsEnd
    });
    if let Some(p) = world.session_mut(shown).and_then(|s| s.character.as_mut()) {
        p.spellbook[..slots].copy_from_slice(&book);
    }
    if let Some(s) = world.session_mut(to) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Level first, then name; empty slots split the runs
    #[test]
    fn sorts_by_level_then_name() {
        let table = |n: SpellNo| -> Option<(u16, String)> {
            match n.0 {
                1 => Some((3, "zap".into())),
                2 => Some((1, "light".into())),
                3 => Some((1, "bless".into())),
                4 => Some((2, "armour".into())),
                _ => None,
            }
        };
        let mut book = vec![
            SpellNo(1),
            SpellNo(2),
            SpellNo(3),
            SpellNo(0),
            SpellNo(4),
            SpellNo(2),
        ];
        sort_spell_list(&mut book, table);
        assert_eq!(
            book,
            vec![
                SpellNo(3),
                SpellNo(2),
                SpellNo(1),
                SpellNo(0),
                SpellNo(2),
                SpellNo(4)
            ]
        );
    }
}
