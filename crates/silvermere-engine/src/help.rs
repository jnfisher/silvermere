//! HELP: [`help_text`] over the topics of `WCCMMHLP.MSG`
//! (`silvermere-text`'s `help_topics.toml`, T-011).
//!
//! The original switched the channel to page mode ("Hit any key to continue...") for the
//! help screens; that is the host's and is not carried.

use std::sync::OnceLock;

use silvermere_rules::progression::{calc_exp_needed_old, new_calc_exp_needed};
use silvermere_text::Msg;
use silvermere_text::catalogue::{Catalogue, HELP_END_MARK, HELP_TOPICS_TOML, HelpTopics};
use silvermere_types::enums::DebugLevel;
use silvermere_types::ids::{ClassNo, SessionId};

use crate::commands::{eq_ignore_case, starts_with_ignore_case};
use crate::display::status::big_exp;
use crate::world::World;

/// `HLPWIDTH`: the widest line of the topic list.
const HLPWIDTH: usize = 79;
/// `HLPLEVELS`: levels in the experience table.
const HLPLEVELS: i32 = 10;
/// `HLPLOWLVL`: up to this level the table starts at level 1.
const HLPLOWLVL: i32 = 5;
/// `HLPBELOW` and `HLPABOVE`: the window around a higher level.
const HLPBELOW: i32 = 4;
const HLPABOVE: i32 = 5;
/// `HLPCLSARGS`: words in `HELP EXP n`.
const HLPCLSARGS: usize = 3;

/// The help topics, parsed once.
pub fn topics() -> &'static HelpTopics {
    static TOPICS: OnceLock<HelpTopics> = OnceLock::new();
    TOPICS.get_or_init(|| Catalogue::help_topics(HELP_TOPICS_TOML).expect("help_topics.toml"))
}

/// `typed` abbreviates the text
/// starting at any word of `name` (`starts_with_ignore_case` on the whole rest, so several typed
/// words may match several words of the name).
pub fn matches_any_word(typed: &str, name: &str) -> bool {
    let mut rest = name;
    loop {
        if starts_with_ignore_case(typed, rest) {
            return true;
        }
        // skip_word, then skip_blanks
        let Some(sp) = rest.find(' ') else {
            return false;
        };
        rest = rest[sp..].trim_start_matches(' ');
        if rest.is_empty() {
            return false;
        }
    }
}

/// A help text with the say and yell characters in place of `{talk_char}` and
/// `{yell_char}`.
fn fill(world: &World, text: &str) -> String {
    text.replace("{talk_char}", &world.config.talk_char.to_string())
        .replace("{yell_char}", &world.config.yell_char.to_string())
}

/// The TOPICS list: the first word of each topic, `, ` after each,
/// a line break before a word that would pass column 79, and the end marker last.
fn topic_list() -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for t in &topics().topics {
        if eq_ignore_case(&t.words, HELP_END_MARK) {
            col += 1 + t.words.len();
            if col > HLPWIDTH {
                out.push('\r');
            }
            out.push_str(&t.words);
            break;
        }
        let word = t.words.split(' ').next().unwrap_or("");
        col += 2 + word.len();
        if col > HLPWIDTH {
            out.push('\r');
            col = 2 + word.len();
        }
        out.push_str(word);
        out.push_str(", ");
    }
    out
}

/// HELP for the player on `id`. `topic` is `None` for HELP alone
/// (the overview, HL1B0), else the rest of the line; `words` is the number of words the
/// line had and `class_word` its third word, for `HELP EXP n` (which the joined
/// topic in practice never lets through).
pub fn help_text(
    world: &mut World,
    id: SessionId,
    topic: Option<&str>,
    words: usize,
    class_word: &str,
) {
    let mut msgs = Vec::new();
    match topic {
        None => msgs.push(Msg::HelpOverview {
            text: fill(world, &topics().overview),
        }),
        Some(t) if matches_any_word(t, "Topics") => {
            msgs.push(Msg::HelpTopicsHead);
            msgs.push(Msg::HelpTopicList { list: topic_list() });
        }
        Some(t) => {
            // The first topic whose words match
            let mut shown = false;
            for h in &topics().topics {
                if matches_any_word(t, &h.words) {
                    msgs.push(Msg::HelpBody {
                        text: fill(world, &h.text),
                    });
                    shown = true;
                    break;
                }
            }
            // The built-in topics, tried whatever was found
            if matches_any_word(t, "Experience xp exp. exps") {
                if exp_table(world, id, words, class_word, &mut msgs) {
                    shown = true;
                }
            } else if matches_any_word(t, "classes") {
                let mut rows: Vec<(u16, String)> = world
                    .classes
                    .values()
                    .map(|c| (c.number.0, c.name.clone()))
                    .collect();
                rows.sort_unstable();
                list_names(&mut msgs, rows, 100);
                shown = true;
            } else if matches_any_word(t, "races") {
                let mut rows: Vec<(u16, String)> = world
                    .races
                    .values()
                    .map(|r| (r.number.0, r.name.clone()))
                    .collect();
                rows.sort_unstable();
                list_names(&mut msgs, rows, 200);
                shown = true;
            } else if matches_any_word(t, "conversion exchange rates") {
                let c = &world.config;
                msgs.push(Msg::ConversionRates {
                    plat_per_runic: c.plat_per_runic,
                    gold_per_plat: c.gold_per_plat,
                    silv_per_gold: c.silv_per_gold,
                    copper_per_silv: c.copper_per_silv,
                    runic: c.usr_cur_name.clone(),
                });
                shown = true;
            } else if matches_any_word(t, "actions") {
                msgs.push(crate::display::actions::list_of_actions(
                    &world.action_order,
                ));
                shown = true;
            }
            if !shown {
                msgs.push(Msg::HelpNone);
            }
        }
    }
    if let Some(s) = world.session_mut(id) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// `display_class_list` / `display_race_list` in the brief form:
/// ANSI_NORMAL, then the names in number order.
fn list_names(msgs: &mut Vec<Msg>, rows: Vec<(u16, String)>, max: usize) {
    msgs.push(Msg::AnsiNormal);
    for (_, name) in rows.into_iter().take(max) {
        msgs.push(Msg::HelpListName { name });
    }
}

/// HELP EXPERIENCE: the experience to reach each of ten levels
/// around the player's own, from his race and class percentages. Returns whether the
/// table was shown (both records found).
fn exp_table(
    world: &World,
    id: SessionId,
    words: usize,
    class_word: &str,
    msgs: &mut Vec<Msg>,
) -> bool {
    let Some(s) = world.session(id) else {
        return false;
    };
    let debug = s.flags.debug != DebugLevel::Off;
    let Some(plr) = s.player() else {
        return false;
    };
    // HELP EXP n names a class by number (low word)
    let cls = if words == HLPCLSARGS {
        let n = class_word.trim().parse::<i64>().unwrap_or(0) as u16;
        world.classes.get(&ClassNo(n))
    } else {
        world.classes.get(&plr.class)
    };
    let (Some(cls), Some(rac)) = (cls, world.races.get(&plr.race)) else {
        return false;
    };
    let modifier = cls.exp + rac.exp;
    msgs.push(Msg::HelpExpHeader);
    let level = i32::from(plr.level);
    let (first, last) = if level > HLPLOWLVL {
        (level - HLPBELOW, level + HLPABOVE)
    } else {
        (1, HLPLEVELS)
    };
    for l in first..=last {
        let label = format!("{:2}", l + 1);
        // A sysop with debug on also sees the old calculation
        if debug {
            msgs.push(Msg::HelpExpRow {
                level: label.clone(),
                exp: calc_exp_needed_old(l as i16, modifier).to_string(),
            });
        }
        let need = new_calc_exp_needed(l as i16, modifier);
        msgs.push(Msg::HelpExpRow {
            level: label,
            exp: big_exp(need.expbil, need.exprem),
        });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_any_word_checks_each_word() {
        assert!(matches_any_word("bash", "Combat attacking bash punch"));
        assert!(matches_any_word("COM", "Combat attacking bash punch"));
        assert!(matches_any_word("class pow", "Class Powers"));
        assert!(!matches_any_word("xyzzy", "Combat attacking bash punch"));
        assert!(!matches_any_word("exp 3", "Experience xp exp. exps"));
    }

    #[test]
    fn the_topic_list_ends_with_the_marker() {
        let l = topic_list();
        assert!(l.starts_with("Commands, "), "{l}");
        assert!(l.ends_with("Help ?"), "{l}");
        for line in l.split('\r') {
            assert!(line.len() <= HLPWIDTH + 2, "{line}");
        }
    }
}
