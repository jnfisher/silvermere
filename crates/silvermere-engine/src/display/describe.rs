//! LOOK at something: [`display_user_desc`], [`display_monster_desc`],
//! [`display_item_desc`], [`display_spell_desc`] and the WCCTEXT texts they may show
//! (`long_text::display_long_text`).

use silvermere_rules::abilities::{AbilityLookup, MonsterAbilities, spell_ability_value};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, DebugLevel, ItemType};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MessageNo, SessionId, SpellNo, TextBlockNo};
use silvermere_types::records::{ActiveMonster, Character};

use super::long_text::display_long_text;
use super::room::desc_from_file;
use super::status::{left, with_abilities};
use crate::world::World;

/// A Descriptive Message printed as a format with `arg`: the first `%s` becomes `arg`, `%%`
/// a `%`.
fn printf_s(fmt: &str, arg: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + arg.len());
    let mut used = false;
    let mut rest = fmt;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("%%") {
            out.push('%');
            rest = after;
        } else if let Some(after) = tail.strip_prefix("%s")
            && !used
        {
            out.push_str(arg);
            used = true;
            rest = after;
        } else {
            out.push('%');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Trailing blanks off.
fn depad(s: &str) -> &str {
    s.trim_end_matches(' ')
}

/// A Descriptive Message's text `which` (0-2) for a spell, if the spell carries one (its
/// Descriptive Message ability, then that message's record).
pub fn desc_message(world: &World, spell: SpellNo, which: usize) -> Option<String> {
    let spl = world.spells.get(&spell)?;
    let n = spell_ability_value(spl, Ability::DescMsg);
    if n == 0 {
        return None;
    }
    let msg = world.messages.get(&MessageNo(n))?;
    let text = &msg.msgs[which];
    (!text.is_empty()).then(|| text.clone())
}

/// The subject (capitalised), object and possessive pronouns by `player.sex`.
fn pronouns(sex: u8) -> (&'static str, &'static str, &'static str, &'static str) {
    match sex {
        b'F' => ("She", "she", "her", "her"),
        b'M' => ("He", "he", "him", "his"),
        _ => ("It", "it", "it", "its"),
    }
}

/// The phrase for a stat in steps of 10 from below 30.
fn step(v: u16, phrases: &[&'static str; 10]) -> &'static str {
    let i = if v < 30 {
        0
    } else {
        (usize::from(v) / 10 - 2).min(9)
    };
    phrases[i]
}

/// The player on `shown` described to `to`: the heading, then `A shadowy figure` under
/// Shadow Stealth, a WCCTEXT text under Shadowform, or the description from the stats and
/// looks, the Descriptive Messages of the spells on him, and what he has equipped. The
/// add-on description hooks are not carried.
pub fn display_user_desc(world: &mut World, shown: SessionId, to: SessionId) {
    let debug = world
        .session(to)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off);
    let Some(plr) = world.session(shown).and_then(|s| s.player()).cloned() else {
        return;
    };
    let mut msgs = Vec::new();
    msgs.push(Msg::LookPlayerHeader {
        first: plr.firstname.clone(),
        last: plr.lastname.clone(),
        close: if plr.lastname.is_empty() { "]" } else { " ]" }.into(),
        immortal: if plr.immortal != 0 {
            "\x1B[31m -- Immortal !".into()
        } else {
            String::new()
        },
        gang: if plr.gang.is_empty() {
            String::new()
        } else {
            format!(" ({})", plr.gang)
        },
    });
    let (shadow, shadowform) = with_abilities(world, &plr, |a| {
        (a.has(Ability::ShadowStealth), a.value(Ability::Shadowform))
    });
    if shadow {
        msgs.push(Msg::LookShadowy);
        send(world, to, msgs);
        return;
    }
    if shadowform != 0 {
        msgs.push(Msg::LookPlayerCr);
        send(world, to, msgs);
        display_long_text(world, to, TextBlockNo(shadowform), None);
        return;
    }
    msgs.push(describe_player(world, &plr));
    let (_, _, obj, _) = pronouns(plr.sex);
    for n in plr.spells.iter().filter(|n| n.0 != 0) {
        if let Some(text) = desc_message(world, *n, 1) {
            msgs.push(Msg::LookDescMsg {
                text: printf_s(&text, obj),
            });
        }
    }
    let (cap, _, _, _) = pronouns(plr.sex);
    msgs.push(Msg::LookEquipped {
        pronoun: cap.into(),
    });
    equipment(world, &plr, debug, &mut msgs);
    send(world, to, msgs);
}

fn send(world: &mut World, to: SessionId, msgs: Vec<Msg>) {
    if let Some(s) = world.session_mut(to) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// The description sentence.
fn describe_player(world: &World, plr: &Character) -> Msg {
    let haircol = [
        "black",
        "white",
        "silver",
        "red",
        "brown",
        "dark brown",
        "blonde",
        "green",
        "blue",
        "grey",
    ]
    .get(usize::from(plr.haircol))
    .copied()
    .unwrap_or("black");
    // A bald character has no hair colour either
    let (hairlen, haircol) = match plr.hairlen {
        1 => ("short ", haircol),
        2 => ("shoulder-length ", haircol),
        3 => ("long ", haircol),
        4 => ("waist-length ", haircol),
        5 => ("ankle-length ", haircol),
        _ => ("no", ""),
    };
    // ("chesnut-brown", sic)
    let eyecol = [
        "black",
        "crimson",
        "yellow",
        "pale blue",
        "sea-blue",
        "dark blue",
        "grey-blue",
        "slate grey",
        "bright green",
        "forest-green",
        "pale green",
        "chesnut-brown",
        "dark brown",
        "hazel",
        "violet",
        "lavender",
        "golden",
    ]
    .get(usize::from(plr.eyecol))
    .copied()
    .unwrap_or("black");
    // Hit points as a percentage of the maximum
    let pct = if plr.maxhp != 0 {
        i32::from(plr.hp) * 100 / i32::from(plr.maxhp)
    } else {
        0
    };
    let mut wound = match pct {
        p if p < 20 => "critically wounded",
        p if p < 40 => "severely wounded",
        p if p < 60 => "heavily wounded",
        p if p < 80 => "moderately wounded",
        p if p < 100 => "slightly wounded",
        _ => "unwounded",
    };
    let mut bleed = "";
    if plr.hp <= 0 {
        wound = "mortally wounded";
        if plr.aided == 0 {
            bleed = " and is bleeding heavily!";
        }
    }
    let (cap, subj, obj, poss) = pronouns(plr.sex);
    let str = step(
        plr.str,
        &[
            "puny",
            "weak",
            "slightly built",
            "moderately built",
            "well built",
            "muscular",
            "powerfully built",
            "heroically proportioned",
            "Herculean",
            "physically Godlike",
        ],
    );
    // ("slugishly", sic)
    let agi = step(
        plr.agi,
        &[
            "slowly",
            "clumsily",
            "slugishly",
            "cautiously",
            "gracefully",
            "very swiftly",
            "with uncanny speed",
            "with catlike agility",
            "blindingly fast",
            "blindingly fast",
        ],
    );
    let hea = step(
        plr.hea,
        &[
            "sickly", "frail", "thin", "healthy", "stout", "solid", "massive", "gigantic",
            "colossal", "colossal",
        ],
    );
    let cha = match plr.cha {
        c if c < 30 => "openly hostile and quite revolting".to_owned(),
        c if c < 40 => "hostile and rather unappealing".to_owned(),
        c if c < 50 => "quite unfriendly and aloof".to_owned(),
        c if c < 60 => "likable in an unassuming sort of way".to_owned(),
        c if c < 70 => "quite attractive and pleasant to be around".to_owned(),
        c if c < 80 => format!("charismatic and outgoing.  You can't help but like {obj}"),
        c if c < 90 => "extremely likeable, and fairly radiates charisma".to_owned(),
        c if c < 100 => format!(
            "incredibly charismatic.  You are almost overpowered by {poss} strong personality"
        ),
        _ => format!(
            "overwhelmingly charismatic.  You almost drop to your knees in wonder at the sight of {obj}!"
        ),
    };
    let int = step(
        plr.intel,
        &[
            "utterly moronic",
            "quite stupid",
            "slightly dull",
            "intelligent",
            "bright",
            "extremely clever",
            "brilliant",
            "a genius",
            "all-knowing",
            "all-knowing",
        ],
    );
    let wil = match plr.wil {
        w if w < 30 => "seems selfish and hot-tempered".to_owned(),
        w if w < 40 => "seems sullen and impulsive".to_owned(),
        w if w < 50 => "seems a little naive".to_owned(),
        w if w < 60 => "looks fairly knowledgeable".to_owned(),
        w if w < 70 => "looks quite experienced and wise".to_owned(),
        w if w < 80 => format!("has a worldly air about {obj}"),
        w if w < 90 => format!("seems to possess a wisdom beyond {poss} years"),
        w if w < 100 => "seems to be in an enlightened state of mind".to_owned(),
        _ => format!("looks like {subj} is one with the Gods"),
    };
    let race = world
        .races
        .get(&plr.race)
        .map_or("Race E0!", |r| r.name.as_str());
    let class = world
        .classes
        .get(&plr.class)
        .map_or("Class E0!", |c| c.name.as_str());
    Msg::LookPlayer {
        first: plr.firstname.clone(),
        build: format!("{hea}, {str} {race} {class}"),
        looks: format!("{hairlen}{haircol} hair and {eyecol} eyes"),
        manner: format!("{cap} moves {agi}, and is {cha}"),
        mind: format!("{int} and {wil}"),
        state: format!("{cap} is {wound}{bleed}"),
    }
}

/// The equipment list: the readied item if it is a light source, each worn slot once in
/// inventory order with its place, then the weapon, or `Nothing`.
fn equipment(world: &World, plr: &Character, debug: bool, msgs: &mut Vec<Msg>) {
    let mut cnt = 0;
    let mut shown = [false; silvermere_types::sizes::NWORN];
    for (i, &n) in plr.items.iter().enumerate() {
        if n.0 == 0 {
            continue;
        }
        if i32::from(plr.readied) == i as i32
            && let Some(itm) = world.items.get(&n)
            && itm.item_type == ItemType::Light
        {
            cnt += 1;
            msgs.push(Msg::LookEquipReadied {
                name: left(&itm.name, 30),
            });
        }
        for (j, &w) in plr.worn.iter().enumerate() {
            if shown[j] || n != w {
                continue;
            }
            let Some(itm) = world.items.get(&n) else {
                // the item no longer exists; it is left out and logged
                tracing::error!(item = n.0, "display_user_desc: Item not found");
                continue;
            };
            shown[j] = true;
            cnt += 1;
            let loc = itm.worn.name().unwrap_or_default().to_owned();
            let uses = plr.itemuses.get(i).copied().unwrap_or(-1);
            msgs.push(if uses != -1 && debug {
                Msg::LookEquipWornDebug {
                    name: itm.name.clone(),
                    uses: i32::from(uses),
                    loc,
                }
            } else {
                Msg::LookEquipWorn {
                    name: left(&itm.name, 30),
                    loc,
                }
            });
        }
    }
    if plr.weapon != ItemNo(0)
        && let Some(itm) = world.items.get(&plr.weapon)
    {
        cnt += 1;
        msgs.push(Msg::LookEquipWeapon {
            name: left(&itm.name, 30),
        });
    }
    msgs.push(Msg::LookEquipEnd {
        nothing: if cnt == 0 { "Nothing\r" } else { "" }.into(),
    });
}

/// The active monster with this number.
fn active_monster(world: &World, n: ActiveMonsterId) -> Option<ActiveMonster> {
    // Looking the monster up (T-086) brings it into the cache
    world.active_monsters.by_number(n).cloned()
}

/// The monster `monnum` described to `to`: the name, then `A shadowy figure` under Shadow
/// Stealth, a WCCTEXT text under Shadowform, or the known monster's description, how
/// wounded it is, whether it is poisoned, and the Descriptive Messages of the spells on it.
/// The sysop debug lines are not carried.
pub fn display_monster_desc(world: &mut World, to: SessionId, monnum: ActiveMonsterId) {
    let Some(mon) = active_monster(world, monnum) else {
        tracing::error!(monster = monnum.0, "no monster");
        return;
    };
    let Some(kmon) = world.monsters.get(&mon.knownnum).cloned() else {
        tracing::error!(monster = monnum.0, "no monster");
        return;
    };
    let mut msgs = vec![Msg::LookMonsterName {
        name: mon.name.clone(),
    }];
    let spells = |n: SpellNo| world.spells.get(&n);
    let items = |n: ItemNo| world.items.get(&n);
    let lookup = MonsterAbilities {
        monster: &mon,
        known: Some(&kmon),
        spells: &spells,
        items: &items,
    };
    let shadow = lookup.has(Ability::ShadowStealth);
    let shadowform = lookup.value(Ability::Shadowform);
    if shadow {
        msgs.push(Msg::LookShadowyMonster);
        send(world, to, msgs);
        return;
    }
    if shadowform != 0 {
        send(world, to, msgs);
        display_long_text(world, to, TextBlockNo(shadowform), None);
        return;
    }
    // Up to the first empty line
    let mut text = String::new();
    for d in &kmon.desc {
        if d.is_empty() {
            break;
        }
        text.push_str(depad(d));
        text.push(' ');
    }
    if !text.is_empty() {
        msgs.push(Msg::LookMonsterDesc { text });
    }
    let pct = if kmon.maxhp != 0 {
        i32::from(mon.hp) * 100 / i32::from(kmon.maxhp)
    } else {
        0
    };
    let wound = match pct {
        p if p < 20 => "very critically wounded",
        p if p < 30 => "critically wounded",
        p if p < 50 => "severely wounded",
        p if p < 70 => "heavily wounded",
        p if p < 85 => "moderately wounded",
        p if p < 100 => "slightly wounded",
        _ => "unwounded",
    };
    let pronoun = kmon.gender.name().unwrap_or("It").to_owned();
    msgs.push(Msg::LookMonster {
        pronoun: pronoun.clone(),
        wound: wound.into(),
    });
    if mon.poison > 0 {
        msgs.push(Msg::LookPoisoned {
            pronoun: pronoun.clone(),
        });
    }
    for sp in mon.spells.iter().filter(|s| s.spell.0 != 0) {
        if let Some(text) = desc_message(world, sp.spell, 1) {
            msgs.push(Msg::LookMonsterDescMsg {
                text: printf_s(&text, &pronoun),
            });
        }
    }
    send(world, to, msgs);
}

/// The item `itemnum` described to `to`: the description lines run together and wrapped, or
/// the `FILE DESCRIPTION` file, then the item's WCCTEXT text if it has one.
pub fn display_item_desc(world: &mut World, to: SessionId, itemnum: ItemNo) {
    let Some(itm) = world.items.get(&itemnum).cloned() else {
        return;
    };
    if !itm.desc[0].is_empty()
        && itm.desc[0].eq_ignore_ascii_case("FILE DESCRIPTION")
        && !itm.desc[1].is_empty()
    {
        // ANSI_CLRLINE and ANSI_FGGREY come first
        let lines = desc_from_file(world, &itm.desc[1], &itm.desc[2], false).unwrap_or_default();
        if let Some(s) = world.session_mut(to) {
            s.output.push(Msg::Clrline);
            s.output.push_raw(silvermere_text::ansi::ANSI_FGGREY);
            for line in lines {
                s.output.push(Msg::RoomFileLine { line });
            }
        }
        return;
    }
    let mut text = String::new();
    for d in itm.desc.iter().filter(|d| !d.is_empty()) {
        text.push_str(depad(d));
        text.push(' ');
    }
    send(world, to, vec![Msg::LookItem { text }]);
    if itm.textnum.0 != 0 {
        display_long_text(world, to, itm.textnum, None);
    }
}

/// The spell `spellnum` described to `to`: the name with its short name, then the two
/// description lines that are not empty.
pub fn display_spell_desc(world: &mut World, to: SessionId, spellnum: SpellNo) {
    let Some(spl) = world.spells.get(&spellnum) else {
        return;
    };
    let mut msgs = vec![if spl.shortnm.is_empty() {
        Msg::LookSpellNoshort {
            name: spl.name.clone(),
        }
    } else {
        Msg::LookSpell {
            name: spl.name.clone(),
            short: spl.shortnm.clone(),
        }
    }];
    for d in spl.descs.iter().filter(|d| !d.is_empty()) {
        msgs.push(Msg::LookSpellDesc { text: d.clone() });
    }
    send(world, to, msgs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_one_string() {
        assert_eq!(printf_s("%s is glowing.", "He"), "He is glowing.");
        assert_eq!(printf_s("100%% %s", "x"), "100% x");
    }

    // Below 30, then steps of ten to 110 and over
    #[test]
    fn stat_steps() {
        let p = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        assert_eq!(step(29, &p), "a");
        assert_eq!(step(30, &p), "b");
        assert_eq!(step(59, &p), "d");
        assert_eq!(step(109, &p), "i");
        assert_eq!(step(110, &p), "j");
        assert_eq!(step(500, &p), "j");
    }
}
