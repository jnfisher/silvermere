//! The character sheet: [`show_status`], [`show_health`] and the EXPERIENCE line
//! ([`show_experience`]), with the `printf` field helpers the other info displays share.

use silvermere_rules::abilities::{AbilityLookup, CharacterAbilities};
use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_text::{Msg, ansi};
use silvermere_types::enums::{Ability, ArmourType, DebugLevel, ItemType, Magery};
use silvermere_types::ids::{ItemNo, SessionId, SpellNo};
use silvermere_types::records::{BigExp, Character};

use crate::world::World;

/// `%-W.Ws`: at most `w` characters, padded on the right.
pub fn left(s: &str, w: usize) -> String {
    let cut: String = s.chars().take(w).collect();
    format!("{cut:<w$}")
}

/// `%W.Ws`: at most `w` characters, padded on the left.
pub fn right(s: &str, w: usize) -> String {
    let cut: String = s.chars().take(w).collect();
    format!("{cut:>w$}")
}

/// `%Wd` (or `%-Wd` with `minus`) of an integer.
pub fn num(n: impl std::fmt::Display, w: usize, minus: bool) -> String {
    if minus {
        format!("{n:<w$}")
    } else {
        format!("{n:>w$}")
    }
}

/// `%lu`, or `%lu%09lu` when the billions are not 0: how the 1.11 two-part experience is
/// printed everywhere.
pub fn big_exp(bil: impl std::fmt::Display + PartialEq<u32>, rem: u32) -> String {
    if bil != 0 {
        format!("{bil}{rem:09}")
    } else {
        rem.to_string()
    }
}

/// The ability totals of `plr` over the world's records (the ability lookups of the
/// displays), handed to `f`.
pub fn with_abilities<T>(
    world: &World,
    plr: &Character,
    f: impl FnOnce(&CharacterAbilities<'_>) -> T,
) -> T {
    let spells = |n: SpellNo| world.spells.get(&n);
    let items = |n: ItemNo| world.items.get(&n);
    let lookup = CharacterAbilities {
        character: plr,
        race: world.races.get(&plr.race),
        class: world.classes.get(&plr.class),
        spells: &spells,
        items: &items,
    };
    f(&lookup)
}

/// The armour class and the damage resistance, both in tenths. The weapon's `ac` counts,
/// then every worn armour item's `ac` and `dr`; Protective Shield adds 10 per point,
/// divided by 4 over platemail, by 3 over chain or scale and by 2 over leather; Alter
/// Damage Resistance adds to the resistance and Alter Defence Value 10 AC per point; the
/// class never goes below 0. A worn slot whose item has vanished is not cleared: the
/// display does not change the character.
pub fn armour_rating(world: &World, plr: &Character) -> (i32, i32) {
    let mut ac = 0i32;
    let mut absorb = 0i32;
    let (mut leather, mut chain, mut scale, mut plate) = (false, false, false, false);
    if plr.weapon.0 != 0
        && let Some(itm) = world.items.get(&plr.weapon)
    {
        ac += i32::from(itm.ac);
    }
    for w in plr.worn.iter().filter(|w| w.0 != 0) {
        let Some(itm) = world.items.get(w) else {
            continue;
        };
        if itm.item_type != ItemType::Armour {
            continue;
        }
        ac += i32::from(itm.ac);
        absorb += i32::from(itm.dr);
        match itm.armtype {
            ArmourType::SoftLeather
            | ArmourType::SoftStuddedLeather
            | ArmourType::RigidLeather
            | ArmourType::StuddedRigidLeather => leather = true,
            ArmourType::Chainmail => chain = true,
            ArmourType::Scalemail => scale = true,
            ArmourType::Platemail => plate = true,
            _ => {}
        }
    }
    let (shield, (dr, defval)) = with_abilities(world, plr, |a| {
        (
            a.value(Ability::Shield),
            a.pair(Ability::DamageResist, Ability::Defence),
        )
    });
    if shield != 0 {
        let shield = if plate {
            shield / 4
        } else if chain || scale {
            shield / 3
        } else if leather {
            shield / 2
        } else {
            shield
        };
        ac += shield * 10;
    }
    absorb += dr;
    ac += defval * 10;
    (ac.max(0), absorb)
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// `ANSI_FGCYAN " "` or `ANSI_LTRED "*"`: the mark before a stat on the sheet.
fn stat_mark(same: bool) -> String {
    let (a, c): (&[u8], &str) = if same {
        (ansi::ANSI_FGCYAN, " ")
    } else {
        (ansi::ANSI_LTRED, "*")
    };
    format!("{}{c}", std::str::from_utf8(a).expect("ANSI is ASCII"))
}

/// The sheet of the character on `shown`, sent to `to` (the STATUS command passes the same
/// session twice; the sysop's look at another player, T-055, passes two). A character
/// without a class record gets nothing. Restructuring pre-1.11 experience is the loader's
/// job (T-023).
pub fn show_status(world: &mut World, shown: SessionId, to: SessionId) {
    let Some(s) = world.session(shown) else {
        return;
    };
    let debug = s.flags.debug != DebugLevel::Off;
    let Some(plr) = s.player() else {
        return;
    };
    let Some(cls) = world.classes.get(&plr.class) else {
        return;
    };
    let mut msgs = Vec::new();
    msgs.push(Msg::StName {
        name: left(&format!("{} {}", plr.firstname, plr.lastname), 29),
        lives: num(plr.lives, 5, false),
        cp: num(plr.cp, 5, true),
    });
    // "Race E0!" for a missing record
    let race = world
        .races
        .get(&plr.race)
        .map_or("Race E0!", |r| r.name.as_str());
    msgs.push(Msg::StRace {
        race: left(race, 11),
        exp: left(&big_exp(plr.expbil, plr.exprem), 15),
        perception: num(plr.perception, 5, false),
    });
    msgs.push(Msg::StClass {
        class: left(&cls.name, 10),
        level: num(plr.level, 5, true),
        stealth: num(plr.stealth, 5, false),
    });
    let (ac, absorb) = armour_rating(world, plr);
    msgs.push(Msg::StHits {
        hp: num(plr.hp, 5, false),
        maxhp: num(plr.maxhp, 5, true),
        ac: num(ac / 10, 3, false),
        dr: num(absorb / 10, 2, true),
        thievery: num(plr.thievery, 5, false),
    });
    // The magery line; the Mana ability marks the mana or kai.
    let has_mana = with_abilities(world, plr, |a| a.has(Ability::Mana));
    let mark = if has_mana {
        format!(
            "{}*{}",
            std::str::from_utf8(ansi::ANSI_LTRED).expect("ASCII"),
            std::str::from_utf8(ansi::ANSI_CYAN).expect("ASCII")
        )
    } else {
        stat_mark(true)
    };
    let traps = num(plr.traps, 5, false);
    msgs.push(match cls.magery {
        Magery::Mage | Magery::Priest | Magery::Druid | Magery::Bard => Msg::StMana {
            mark,
            mana: num(plr.mana, 4, false),
            maxmana: num(plr.maxmana, 5, true),
            spellcast: num(plr.spellcast, 5, true),
            traps,
        },
        Magery::Kai => Msg::StKai {
            mark,
            mana: num(plr.mana, 4, false),
            maxmana: num(plr.maxmana, 5, true),
            traps,
        },
        _ => Msg::StTraps { traps },
    });
    msgs.push(Msg::StPicklocks {
        picklocks: num(plr.picklocks, 5, false),
    });
    // A red star for a stat away from its base value
    msgs.push(Msg::StStr {
        mark1: stat_mark(plr.str == plr.basstr),
        stat1: num(plr.str, 5, true),
        mark2: stat_mark(plr.agi == plr.basagi),
        stat2: num(plr.agi, 5, true),
        stat3: num(plr.tracking, 5, false),
    });
    msgs.push(Msg::StInt {
        mark1: stat_mark(plr.intel == plr.basint),
        stat1: num(plr.intel, 5, true),
        mark2: stat_mark(plr.hea == plr.bashea),
        stat2: num(plr.hea, 5, true),
        stat3: num(plr.martial, 5, false),
    });
    msgs.push(Msg::StWil {
        mark1: stat_mark(plr.wil == plr.baswil),
        stat1: num(plr.wil, 5, true),
        mark2: stat_mark(plr.cha == plr.bascha),
        stat2: num(plr.cha, 5, true),
        stat3: num(plr.mr, 5, false),
    });
    if plr.poison > 0 {
        msgs.push(Msg::StPoisoned);
    }
    msgs.push(Msg::AnsiNormal);
    match plr.rank {
        1 => msgs.push(Msg::StRankFront),
        2 => msgs.push(Msg::StRankBack),
        _ => {}
    }
    let cast: Vec<(SpellNo, i16, i16)> = plr
        .spells
        .iter()
        .zip(plr.spellvals.iter().zip(plr.spellrnds.iter()))
        .filter(|(n, _)| n.0 != 0)
        .map(|(n, (v, r))| (*n, *v, *r))
        .collect();
    if debug {
        // The debug list; the plain forms are unreachable (sic).
        let list: Vec<String> = cast
            .iter()
            .filter_map(|(n, v, r)| world.spells.get(n).map(|spl| (spl, v, r)))
            .map(|(spl, v, r)| format!("{} ({v},{r})", spl.name))
            .collect();
        if !list.is_empty() {
            msgs.push(Msg::StSpellsOn {
                list: format!("  {}", list.join(", ")),
            });
        }
        msgs.push(Msg::StBlank);
    } else {
        // The third text of each spell's Descriptive Message
        for (n, _, _) in &cast {
            let Some(spl) = world.spells.get(n) else {
                continue;
            };
            let msgnum = silvermere_rules::abilities::spell_ability_value(spl, Ability::DescMsg);
            if msgnum == 0 {
                continue;
            }
            let Some(msg) = world
                .messages
                .get(&silvermere_types::ids::MessageNo(msgnum))
            else {
                continue;
            };
            if !msg.msgs[2].is_empty() {
                msgs.push(Msg::StSpellDesc {
                    text: msg.msgs[2].clone(),
                });
            }
        }
    }
    if let Some(s) = world.session_mut(to) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// Hit points, and the mana or kai of a caster, as `current/maximum [percent]`. Nothing for
/// a character with no maximum hit points or no class record.
pub fn show_health(world: &mut World, id: SessionId) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    if plr.maxhp == 0 {
        return;
    }
    let Some(cls) = world.classes.get(&plr.class) else {
        return;
    };
    // The percentage in 32 bits, truncated
    let hppct = (i32::from(plr.hp) * 100 / i32::from(plr.maxhp)).to_string();
    let hp = format!("{}/{}", num(plr.hp, 5, false), num(plr.maxhp, 5, true));
    let msg = if plr.maxmana > 0 {
        let manapct = (i32::from(plr.mana) * 100 / i32::from(plr.maxmana)).to_string();
        Msg::HealthMana {
            hp,
            hppct,
            kind: if cls.magery == Magery::Kai {
                "Kai"
            } else {
                "Mana"
            }
            .into(),
            mana: format!("{}/{}", num(plr.mana, 3, false), num(plr.maxmana, 3, true)),
            manapct,
        }
    } else {
        Msg::Health { hp, hppct }
    };
    push(world, id, msg);
}

/// How far `have` is towards `need`, in percent, both squeezed into 32 bits by a scale
/// chosen from the billions of `have`. The quirks are kept: from 11 billion up the scales
/// of "need" do not keep the 100:1 rule, the fourth case tests `need_lo` (sic), and a need
/// of 0 counts as 1. Unsigned 32-bit arithmetic throughout, wrapping, so the percent
/// shown for huge figures is the original's.
pub fn calc_exp_percent(have_hi: u32, have_lo: u32, need_hi: u32, need_lo: u32) -> u32 {
    let (divisor, mult, have): (u32, u32, u32);
    if have_hi <= 10 && have_hi > 0 {
        // Units of 100
        divisor = 10_000;
        mult = 100_000;
        have = if have_lo > 100 {
            (have_lo / 100).wrapping_add(have_hi.wrapping_mul(10_000_000))
        } else {
            have_hi.wrapping_mul(10_000_000)
        };
    } else if have_hi <= 100 && have_hi > 10 {
        divisor = 100_000;
        mult = 1_000_000;
        have = if have_lo >= 1000 {
            (have_lo / 1000).wrapping_add(have_hi.wrapping_mul(1_000_000))
        } else {
            have_hi.wrapping_mul(1_000_000)
        };
    } else if have_hi <= 1000 && have_hi > 100 {
        divisor = 1_000_000;
        mult = 10_000;
        have = if have_lo >= 10_000 {
            (have_lo / 10_000).wrapping_add(have_hi.wrapping_mul(100_000))
        } else {
            have_hi.wrapping_mul(100_000)
        };
    } else if have_hi >= 10_000_000 && need_lo < 100_000 {
        // Sic: tests need_lo
        divisor = 1_000_000;
        mult = 100_000;
        have = if have_lo != 0 {
            (have_lo / 10_000).wrapping_add(have_hi.wrapping_mul(100_000))
        } else {
            have_hi.wrapping_mul(100_000)
        };
    } else if have_lo > 999_999 {
        divisor = 10_000;
        mult = 1;
        have = have_lo / 100;
    } else {
        divisor = 100;
        mult = 1;
        have = have_lo;
    }
    let mut need = if need_hi > 0 && need_hi <= 1000 {
        if need_lo != 0 {
            (need_lo / divisor).wrapping_add(need_hi.wrapping_mul(mult))
        } else {
            need_hi.wrapping_mul(mult)
        }
    } else {
        need_lo / divisor
    };
    if need == 0 {
        need = 1;
    }
    have / need
}

/// The figures of the EXPERIENCE line (the "new calculation"): the total needed for the
/// next level from the level and the race and class percentages, what is still needed
/// (borrowing a billion, 0 once past the level) and the percentage.
pub fn exp_figures(plr: &Character, exp_mod: i16) -> (BigExp, (i64, i64), u32) {
    let need = new_calc_exp_needed(plr.level as i16, exp_mod);
    let pct = calc_exp_percent(plr.expbil, plr.exprem, need.expbil, need.exprem);
    // In `long`
    let mut leftbil = i64::from(need.expbil) - i64::from(plr.expbil);
    let mut leftrem = i64::from(need.exprem) - i64::from(plr.exprem);
    if leftrem < 0 {
        leftbil -= 1;
        leftrem = i64::from(need.exprem) + 1_000_000_000 - i64::from(plr.exprem);
    }
    if leftbil < 0 {
        leftbil = 0;
        leftrem = 0;
    }
    (need, (leftbil, leftrem), pct)
}

/// The EXPERIENCE command for the player on `id`: the one line
/// `Exp: .. Level: .. Exp needed for next level: left (total) [pct%]`. Only the new
/// two-part display is carried; the old 32-bit display and the `CMD EXP :` lines are sysop
/// debugging (not carried). With a missing race or class record the line is not printed.
pub fn show_experience(world: &mut World, id: SessionId) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    let (Some(rac), Some(cls)) = (world.races.get(&plr.race), world.classes.get(&plr.class)) else {
        return;
    };
    let (need, (leftbil, leftrem), pct) = exp_figures(plr, rac.exp + cls.exp);
    let left = if leftbil != 0 {
        format!("{leftbil}{leftrem:09}")
    } else {
        leftrem.to_string()
    };
    let msg = Msg::ExpLine {
        exp: big_exp(plr.expbil, plr.exprem),
        level: plr.level.to_string(),
        left,
        need: big_exp(need.expbil, need.exprem),
        pct: pct.to_string(),
    };
    push(world, id, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    // No billions, need counted in hundreds
    #[test]
    fn percent_below_a_million() {
        assert_eq!(calc_exp_percent(0, 0, 0, 1000), 0);
        assert_eq!(calc_exp_percent(0, 346, 0, 1650), 21);
        assert_eq!(calc_exp_percent(0, 999, 0, 1000), 99);
    }

    // From one billion, have in hundreds and need in ten-thousands
    #[test]
    fn percent_with_billions() {
        // 1.5 billion of 2 billion: have 15_000_000 (hundreds), need 200_000
        assert_eq!(calc_exp_percent(1, 500_000_000, 2, 0), 75);
        // a need of 0 counts as 1
        assert_eq!(calc_exp_percent(0, 5, 0, 0), 5);
    }

    #[test]
    fn printf_helpers() {
        assert_eq!(left("Kasimir Drakemoor", 5), "Kasim");
        assert_eq!(left("ab", 4), "ab  ");
        assert_eq!(right("1", 3), "  1");
        assert_eq!(num(9, 5, false), "    9");
        assert_eq!(num(12, 5, true), "12   ");
        assert_eq!(big_exp(0u32, 1712), "1712");
        assert_eq!(big_exp(2u32, 5), "2000000005");
    }
}
