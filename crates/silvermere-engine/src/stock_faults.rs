//! Known faults in the stock content, fixed as it loads.
//!
//! Each entry names one record and what it holds as shipped: a text block's line, a message's
//! text, a spell's name. A record that was edited, or is missing, does not match and is left
//! alone. The fix is made on the loaded copy only, so an existing database needs no re-import
//! and nothing a player owns is written.
//!
//! The spell and message faults are the ones whose texts do not fit the values the game
//! passes them (a number read as a text, which the game prints as garbage or crashes on).

use std::collections::BTreeMap;

use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{MessageNo, SpellNo, TextBlockNo};
use silvermere_types::records::{Message, Spell, TextBlock};

/// One known fault in the stock text blocks.
pub struct Fault {
    /// The block holding the line.
    pub block: i32,
    /// The whole line as shipped.
    pub stock: &'static str,
    /// The line it is replaced by.
    pub fixed: &'static str,
    /// What was wrong, for the load log.
    pub reason: &'static str,
}

/// The known stock data faults.
pub const FAULTS: &[Fault] = &[Fault {
    block: 9578,
    stock: "go nightblack portal:roomitem 1419 1373:minlevel 40 1374:message 1375:2683 1:message 1376",
    fixed: "go nightblack portal:roomitem 1419 1373:minlevel 40 1374:message 1375:teleport 2683 1:message 1376",
    reason: "the Negative Power Plane's `go nightblack portal` lacked `teleport`, so it showed \
             its messages and left the player in place",
}];

/// A stock spell that is not a real spell: its number and name.
pub const JUNK_SPELLS: &[(i16, &str)] = &[
    // alternate messages but no message record: the game's built-in texts then get a
    // number where they print a name
    (1055, "BCNS"),
];

/// Stock spells whose alternate-messages flag does not fit their texts: the number, the
/// name, and whether the flag belongs.
pub const SPELL_STYLES: &[(i16, &str, bool)] = &[
    (945, "acid burns", true),
    (1173, "destructo smash", true),
    (230, "gout of flame", false),
    (451, "strain", false),
    (508, "giant troll death", false),
    (873, "smite", false),
    (874, "speed", false),
    (875, "magic armour", false),
    (876, "rapid healing", false),
    (940, "fear", false),
    (1010, "unholy light", false),
    (1109, "strut", false),
    (1187, "pris weapon effect", false),
    (1212, "gnaj explosion", false),
    (1221, "vanishing jumpkick", false),
    (1222, "ether death", false),
    (1249, "bone necklace", false),
    (1373, "ray of petrification", false),
];

/// A stock message text that does not fit what the game passes it.
pub struct MessageFault {
    pub number: i32,
    /// Which of the three texts (0 the actor's, 1 the target's, 2 the room's).
    pub text: usize,
    /// The text as shipped.
    pub stock: &'static str,
    /// The text it is replaced by.
    pub fixed: &'static str,
}

/// The known stock message faults: a `%'s` that was meant as `%s's`, a `%d` given a name,
/// and one text lost to a NUL in its first byte.
pub const MESSAGE_FAULTS: &[MessageFault] = &[
    MessageFault {
        number: 1,
        text: 0,
        stock: "",
        fixed: "You flake off pieces of the steak and eat them, savoring the flavor.      ",
    },
    MessageFault {
        number: 1219,
        text: 2,
        stock: "With a flash and a thunderclap, %'s enemies are flung into the ether!",
        fixed: "With a flash and a thunderclap, %s's enemies are flung into the ether!",
    },
    MessageFault {
        number: 2767,
        text: 0,
        stock: "You are draining %'s life away!",
        fixed: "You are draining %s's life away!",
    },
    MessageFault {
        number: 1553,
        text: 0,
        stock: "You are seared by the flames for %d damage!",
        fixed: "You are seared by the flames for %s damage!",
    },
    MessageFault {
        number: 2606,
        text: 0,
        stock: "You summon a %s which drains your enemies for %d damage!",
        fixed: "You summon a %s which drains your enemies for %s damage!",
    },
    MessageFault {
        number: 2718,
        text: 0,
        stock: "A magma explosion hits you for %d damage!",
        fixed: "A magma explosion hits you for %s damage!",
    },
    MessageFault {
        number: 2869,
        text: 1,
        stock: "An %s shoots out and rots you for %d damage!",
        fixed: "An %s shoots out and rots you for %s damage!",
    },
    MessageFault {
        number: 3279,
        text: 0,
        stock: "An explosion of spikes pierces your foes for %d damage!",
        fixed: "An explosion of spikes pierces your foes for %s damage!",
    },
    MessageFault {
        number: 3313,
        text: 0,
        stock: "The greatsword of the statue swings down and cleaves you for %d damage!",
        fixed: "The greatsword of the statue swings down and cleaves you for %s damage!",
    },
    MessageFault {
        number: 3313,
        text: 2,
        stock: "The greatsword the statue swings down and cleaves %s for %d damage!",
        fixed: "The greatsword the statue swings down and cleaves %s for %s damage!",
    },
    MessageFault {
        number: 3334,
        text: 0,
        stock: "You are fried by the lightning for %d damage!",
        fixed: "You are fried by the lightning for %s damage!",
    },
    MessageFault {
        number: 3335,
        text: 0,
        stock: "You are frozen by the blizzard for %d damage!",
        fixed: "You are frozen by the blizzard for %s damage!",
    },
    MessageFault {
        number: 3336,
        text: 0,
        stock: "You are pelted by rock shards for %d damage!",
        fixed: "You are pelted by rock shards for %s damage!",
    },
    MessageFault {
        number: 3447,
        text: 0,
        stock: "A large globule of magma drips down, burning you for %d damage!",
        fixed: "A large globule of magma drips down, burning you for %s damage!",
    },
];

/// Whether `spell` is one of the [`JUNK_SPELLS`].
pub fn is_junk_spell(spell: &Spell) -> bool {
    JUNK_SPELLS
        .iter()
        .any(|&(n, name)| spell.number.0 == n && spell.name == name)
}

/// `spell` with its [`SPELL_STYLES`] flag set right; true when it was changed.
pub fn fix_spell(spell: &mut Spell) -> bool {
    let Some(&(_, _, alt)) = SPELL_STYLES
        .iter()
        .find(|&&(n, name, _)| spell.number.0 == n && spell.name == name)
    else {
        return false;
    };
    if spell.flags.contains(SpellFlags::ALTMSG) == alt {
        return false;
    }
    spell.flags.set(SpellFlags::ALTMSG, alt);
    true
}

/// `message` with its [`MESSAGE_FAULTS`] fixed; true when one was.
pub fn fix_message(message: &mut Message) -> bool {
    let mut changed = false;
    for f in MESSAGE_FAULTS
        .iter()
        .filter(|f| f.number == message.number.0)
    {
        if message.msgs[f.text] == f.stock {
            message.msgs[f.text] = f.fixed.to_owned();
            changed = true;
        }
    }
    changed
}

/// Applies the spell faults to the loaded spells, logging how many were fixed.
pub fn apply_spells(spells: &mut BTreeMap<SpellNo, Spell>) {
    let before = spells.len();
    spells.retain(|_, sp| !is_junk_spell(sp));
    let restyled = spells.values_mut().map(fix_spell).filter(|&f| f).count();
    let dropped = before - spells.len();
    if dropped + restyled > 0 {
        tracing::info!(
            dropped,
            restyled,
            "stock data faults fixed: junk spells dropped, messaging flags set"
        );
    }
}

/// Applies the message faults to the loaded messages, logging how many were fixed.
pub fn apply_messages(messages: &mut BTreeMap<MessageNo, Message>) {
    let fixed = messages
        .values_mut()
        .map(fix_message)
        .filter(|&f| f)
        .count();
    if fixed > 0 {
        tracing::info!(fixed, "stock data faults fixed: message texts");
    }
}

/// Replaces `fault.stock` by `fault.fixed` where it is a whole line of `text`.
fn fix_text(text: &str, fault: &Fault) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 16);
    let mut changed = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line == fault.stock {
            out.push_str(fault.fixed);
            changed = true;
        } else {
            out.push_str(line);
        }
    }
    changed.then_some(out)
}

/// Applies the known faults to the loaded text blocks, logging each one fixed.
pub fn apply(blocks: &mut BTreeMap<(i16, TextBlockNo), TextBlock>) {
    for fault in FAULTS {
        let mut fixed = false;
        for block in blocks.values_mut().filter(|b| b.number.0 == fault.block) {
            if let Some(text) = fix_text(&block.text, fault) {
                block.text = text;
                fixed = true;
            }
        }
        if fixed {
            tracing::info!(
                block = fault.block,
                "stock data fault fixed: {}",
                fault.reason
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spells::test_fixture::spell as fixture_spell;
    use silvermere_types::enums::{SpellTarget, SpellType};

    fn block(number: i32, text: &str) -> BTreeMap<(i16, TextBlockNo), TextBlock> {
        let b = TextBlock {
            part: 0,
            number: TextBlockNo(number),
            link: TextBlockNo(0),
            text: text.to_owned(),
        };
        BTreeMap::from([((0, b.number), b)])
    }

    #[test]
    fn fixes_only_the_exact_stock_line() {
        let f = &FAULTS[0];
        let mut m = block(f.block, &format!("go portal:x\n{}\nenter y:z", f.stock));
        apply(&mut m);
        assert_eq!(
            m[&(0, TextBlockNo(f.block))].text,
            format!("go portal:x\n{}\nenter y:z", f.fixed)
        );
    }

    fn spell(number: i16, name: &str, flags: SpellFlags) -> Spell {
        let mut sp = fixture_spell(
            number,
            name,
            SpellType::from_code(0),
            SpellTarget::from_code(0),
            (0, 0),
            0,
            0,
            &[],
        );
        sp.flags = flags;
        sp
    }

    #[test]
    fn drops_the_junk_spell_and_sets_the_messaging_flags() {
        let mut m = BTreeMap::new();
        for sp in [
            spell(1055, "BCNS", SpellFlags::ALTMSG),
            spell(945, "acid burns", SpellFlags::empty()),
            spell(230, "gout of flame", SpellFlags::ALTMSG),
            // another name in the slot: an edited record, left alone
            spell(451, "my strain", SpellFlags::ALTMSG),
        ] {
            m.insert(sp.number, sp);
        }
        apply_spells(&mut m);
        assert!(!m.contains_key(&SpellNo(1055)));
        assert!(m[&SpellNo(945)].flags.contains(SpellFlags::ALTMSG));
        assert!(!m[&SpellNo(230)].flags.contains(SpellFlags::ALTMSG));
        assert!(m[&SpellNo(451)].flags.contains(SpellFlags::ALTMSG));
    }

    #[test]
    fn fixes_only_a_stock_message_text() {
        let f = &MESSAGE_FAULTS[2];
        let mut msg = Message {
            number: MessageNo(f.number),
            msgs: Default::default(),
        };
        msg.msgs[f.text] = f.stock.to_owned();
        let mut edited = msg.clone();
        edited.msgs[f.text].push('!');
        let mut m = BTreeMap::from([(msg.number, msg)]);
        apply_messages(&mut m);
        assert_eq!(m[&MessageNo(f.number)].msgs[f.text], f.fixed);
        let mut m = BTreeMap::from([(edited.number, edited.clone())]);
        apply_messages(&mut m);
        assert_eq!(m[&MessageNo(f.number)], edited);
    }

    #[test]
    fn leaves_an_edited_line_or_another_block_alone() {
        let f = &FAULTS[0];
        let edited = format!("{} ", f.stock);
        let mut m = block(f.block, &edited);
        apply(&mut m);
        assert_eq!(m[&(0, TextBlockNo(f.block))].text, edited);
        let mut m = block(f.block + 1, f.stock);
        apply(&mut m);
        assert_eq!(m[&(0, TextBlockNo(f.block + 1))].text, f.stock);
    }
}
