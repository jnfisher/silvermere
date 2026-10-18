//! The texts of a blow: the attacker's, the victim's and the room's lines become one
//! [`CombatMsg`] per outcome.
//!
//! The original gives every move the same three formats, so a damaging hit has one form
//! here. The verbs are the attacker's [`Verbs`]; `critically` is put before the verb of a
//! critical hit. Monster lines are the WCCMSG texts of the known monster (`hitmsgs`,
//! `defmsgs`, `missmsgs`) with their blanks filled as the original's lines show, or the built-in
//! forms, with the first letter upper-cased.

use silvermere_rules::combat::Outcome;
use silvermere_text::Msg;

use super::Verbs;
use crate::movement::exits::{Arg, fmt_msg};

/// The three points of view of one blow: the attacker's line, the victim's (a player
/// victim only) and the room's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatMsg {
    pub you: Msg,
    pub victim: Option<Msg>,
    pub room: Msg,
}

/// The text with its first letter upper-cased.
pub fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// The verb of a hit, with `critically` for a critical one.
fn hit_verb(verbs: &Verbs, form: usize, critical: bool) -> String {
    if critical {
        format!("critically {}", verbs.hit[form])
    } else {
        verbs.hit[form].clone()
    }
}

/// A player's blow on a monster (`attack_user_monster`). `None` for a hit: hits are
/// [`CombatMsg`] with the damage; this covers every outcome.
pub fn player_on_monster(who: &str, verbs: &Verbs, mon: &str, outcome: Outcome) -> CombatMsg {
    match outcome {
        Outcome::Hit { damage, critical } => CombatMsg {
            // "You %s %s for %d damage!"
            you: Msg::CombatYouHit {
                verb: hit_verb(verbs, 0, critical),
                target: mon.to_owned(),
                dmg: damage,
            },
            victim: None,
            // "%s %s %s for %s damage!" (the damage through dmgdesc)
            room: Msg::CombatRoomHit {
                who: who.to_owned(),
                verb: hit_verb(verbs, 2, critical),
                target: mon.to_owned(),
                dmg: damage.to_string(),
            },
        },
        // the armour took it: both texts use the first miss verb
        Outcome::NoDamage => CombatMsg {
            you: Msg::CombatYouGlanceMon {
                verb: verbs.miss[0].clone(),
                target: mon.to_owned(),
            },
            victim: None,
            room: Msg::CombatRoomGlanceMon {
                who: who.to_owned(),
                verb: verbs.miss[0].clone(),
                target: mon.to_owned(),
            },
        },
        Outcome::Dodged => CombatMsg {
            you: Msg::CombatYouDodged {
                verb: verbs.miss[0].clone(),
                target: mon.to_owned(),
            },
            victim: None,
            room: Msg::CombatRoomDodgedMon {
                who: who.to_owned(),
                verb: verbs.miss[2].clone(),
                target: mon.to_owned(),
            },
        },
        Outcome::Miss => CombatMsg {
            you: Msg::CombatYouMiss {
                verb: verbs.miss[0].clone(),
                target: mon.to_owned(),
            },
            victim: None,
            room: Msg::CombatRoomMiss {
                who: who.to_owned(),
                verb: verbs.miss[2].clone(),
                target: mon.to_owned(),
            },
        },
    }
}

/// A player's blow on a player (`attack_user_user`). `his_atk` is the attacker's possessive
/// pronoun, `his_def` the victim's.
pub fn player_on_player(
    who: &str,
    verbs: &Verbs,
    target: &str,
    his_atk: &str,
    his_def: &str,
    outcome: Outcome,
) -> CombatMsg {
    match outcome {
        Outcome::Hit { damage, critical } => CombatMsg {
            you: Msg::CombatYouHit {
                verb: hit_verb(verbs, 0, critical),
                target: target.to_owned(),
                dmg: damage,
            },
            victim: Some(Msg::CombatVictimHit {
                who: who.to_owned(),
                verb: hit_verb(verbs, 1, critical),
                dmg: damage,
            }),
            room: Msg::CombatRoomHit {
                who: who.to_owned(),
                verb: hit_verb(verbs, 2, critical),
                target: target.to_owned(),
                dmg: damage.to_string(),
            },
        },
        // all three use the first miss verb
        Outcome::NoDamage => CombatMsg {
            you: Msg::CombatYouGlancePvp {
                verb: verbs.miss[0].clone(),
                target: target.to_owned(),
                his: his_def.to_owned(),
            },
            victim: Some(Msg::CombatVictimGlancePvp {
                who: who.to_owned(),
                verb: verbs.miss[0].clone(),
            }),
            room: Msg::CombatRoomGlancePvp {
                who: who.to_owned(),
                verb: verbs.miss[0].clone(),
                target: target.to_owned(),
                his: his_def.to_owned(),
            },
        },
        // a fixed text for the attacker, the dodge formats for the others
        Outcome::Dodged => CombatMsg {
            you: Msg::CombatPvpDodges {
                who: target.to_owned(),
            },
            victim: Some(Msg::CombatVictimDodgePvp {
                who: who.to_owned(),
                verb: verbs.miss[1].clone(),
                his: his_atk.to_owned(),
                weapon: verbs.weapon.clone(),
            }),
            room: Msg::CombatRoomDodgePvp {
                who: who.to_owned(),
                verb: verbs.miss[2].clone(),
                target: target.to_owned(),
                his: his_atk.to_owned(),
                weapon: verbs.weapon.clone(),
            },
        },
        Outcome::Miss => CombatMsg {
            you: Msg::CombatYouMiss {
                verb: verbs.miss[0].clone(),
                target: target.to_owned(),
            },
            victim: Some(Msg::CombatVictimMiss {
                who: who.to_owned(),
                verb: verbs.miss[1].clone(),
            }),
            room: Msg::CombatRoomMiss {
                who: who.to_owned(),
                verb: verbs.miss[2].clone(),
                target: target.to_owned(),
            },
        },
    }
}

/// The WCCMSG records of the monster's attack slot (its hit, defend and miss messages),
/// each `None` when 0 or missing.
#[derive(Debug, Clone, Default)]
pub struct SlotTexts {
    pub hit: Option<[String; 3]>,
    pub def: Option<[String; 3]>,
    pub miss: Option<[String; 3]>,
}

/// A monster's blow on a player (`attack_monster_user`): the player's line and the room's.
/// `pron_poss`/`pron_subj` are the victim's pronouns.
pub fn monster_on_player(
    mon: &str,
    verbs: &Verbs,
    texts: &SlotTexts,
    victim: &str,
    pron_poss: &str,
    pron_subj: &str,
    outcome: Outcome,
) -> CombatMsg {
    let star = Arg::S("*");
    match outcome {
        Outcome::Hit { damage, critical } => {
            // to the player: text 1 with the damage, else the built-in form
            let you = match &texts.hit {
                Some(m) => fmt_msg(&m[0], &[Arg::S(mon), Arg::D(i64::from(damage)), star, star]),
                None => format!(
                    "{mon} {} you for {damage} damage!",
                    hit_verb(verbs, 1, critical)
                ),
            };
            // to the room: text 2, the damage as text
            let dmg = damage.to_string();
            let room = match &texts.hit {
                Some(m) => fmt_msg(
                    &m[1],
                    &[Arg::S(mon), Arg::S(victim), Arg::S(&dmg), star, star],
                ),
                None => format!(
                    "{mon} {} {victim} for {dmg} damage!",
                    hit_verb(verbs, 2, critical)
                ),
            };
            CombatMsg {
                you: Msg::CombatMonHitYou {
                    text: upper_first(&you),
                },
                victim: None,
                room: Msg::CombatMonHitRoom {
                    text: upper_first(&room),
                },
            }
        }
        Outcome::NoDamage => {
            // the deflect message: text 1 to the player, text 2 to the room
            let (you, room) = match &texts.def {
                Some(m) => (
                    fmt_msg(&m[0], &[Arg::S(mon), Arg::S(&verbs.miss[1]), star, star]),
                    fmt_msg(
                        &m[1],
                        &[
                            Arg::S(mon),
                            Arg::S(&verbs.miss[1]),
                            Arg::S(victim),
                            Arg::S(pron_poss),
                            star,
                            star,
                        ],
                    ),
                ),
                None => (
                    format!(
                        "{mon}'s {} hits you, but your armour deflects.",
                        verbs.miss[1]
                    ),
                    format!(
                        "{mon}'s {} hits {victim}, but glances off {pron_poss} armour.",
                        verbs.miss[1]
                    ),
                ),
            };
            CombatMsg {
                you: Msg::CombatMonDeflect {
                    text: upper_first(&you),
                },
                victim: None,
                room: Msg::CombatMonDeflect {
                    text: upper_first(&room),
                },
            }
        }
        Outcome::Dodged => {
            // both records: text 3 of the deflect message to the player and text 1 of the
            // miss message to the room; else built-in texts
            let (you, room) = match (&texts.def, &texts.miss) {
                (Some(d), Some(m)) => (
                    fmt_msg(
                        &d[2],
                        &[
                            Arg::S(mon),
                            Arg::S(&verbs.miss[1]),
                            Arg::S(&verbs.weapon),
                            star,
                            star,
                        ],
                    ),
                    fmt_msg(
                        &m[0],
                        &[
                            Arg::S(mon),
                            Arg::S(&verbs.miss[2]),
                            Arg::S(victim),
                            Arg::S(&verbs.weapon),
                            Arg::S(pron_subj),
                            star,
                            star,
                        ],
                    ),
                ),
                _ => (
                    format!(
                        "{mon} {} you with {}, but you dodge!",
                        verbs.miss[1], verbs.weapon
                    ),
                    format!(
                        "{mon} {} {victim} with its {}, but {pron_subj} dodges.",
                        verbs.miss[2], verbs.weapon
                    ),
                ),
            };
            CombatMsg {
                you: Msg::CombatMonDodge {
                    text: upper_first(&you),
                },
                victim: None,
                room: Msg::CombatMonDodge {
                    text: upper_first(&room),
                },
            }
        }
        Outcome::Miss => {
            // the miss message: text 2 to the player, text 3 to the room
            let (you, room) = match &texts.miss {
                Some(m) => (
                    fmt_msg(
                        &m[1],
                        &[
                            Arg::S(mon),
                            Arg::S(&verbs.miss[1]),
                            Arg::S(&verbs.weapon),
                            star,
                            star,
                        ],
                    ),
                    fmt_msg(
                        &m[2],
                        &[
                            Arg::S(mon),
                            Arg::S(&verbs.miss[2]),
                            Arg::S(victim),
                            Arg::S(&verbs.weapon),
                            star,
                            star,
                        ],
                    ),
                ),
                None => (
                    format!("{mon} {} you with {}.", verbs.miss[1], verbs.weapon),
                    format!(
                        "{mon} {} {victim} with its {}.",
                        verbs.miss[2], verbs.weapon
                    ),
                ),
            };
            CombatMsg {
                you: Msg::CombatMonMiss {
                    text: upper_first(&you),
                },
                victim: None,
                room: Msg::CombatMonMiss {
                    text: upper_first(&room),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_first_only_touches_the_first_letter() {
        assert_eq!(upper_first("the giant rat bites"), "The giant rat bites");
        assert_eq!(upper_first(""), "");
    }
}
