//! TOPTEN and the menu's `T` and `G`: [`display_only_topten`], [`display_topten`] and
//! [`display_next_topten`], and the top gangs ([`display_top_gangs`]).
//!
//! The list is `world.topten` (kept by T-039); its length is the number of entries.

use silvermere_text::Msg;
use silvermere_types::enums::{DebugLevel, SessionState};
use silvermere_types::flags::{GangFlags, SessionFlags};
use silvermere_types::ids::{ClassNo, SessionId};
use silvermere_types::records::{Gang, TopEntry};

use super::status::{big_exp, left, right};
use crate::world::World;

/// Gang records looked at, at most.
const TOPGANGSCAN: usize = 200;

/// The class's name; "Class E0!" for a missing record.
fn class_name(world: &World, n: ClassNo) -> String {
    world
        .classes
        .get(&n)
        .map_or_else(|| "Class E0!".to_owned(), |c| c.name.clone())
}

/// The entry's gang, or `None` when it has none.
fn gang_or_none(e: &TopEntry) -> &str {
    if e.gang.is_empty() { "None" } else { &e.gang }
}

fn push_all(world: &mut World, id: SessionId, msgs: Vec<Msg>) {
    if let Some(s) = world.session_mut(id) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// The main menu's `T`: the heading and the first ten entries at once, without paging.
/// Empty entries print nothing.
pub fn display_only_topten(world: &mut World, id: SessionId) {
    let mut msgs = vec![Msg::ToptenMenuHeader];
    for (i, e) in world.topten.iter().take(10).enumerate() {
        if e.classnum.0 == 0 {
            continue;
        }
        msgs.push(Msg::ToptenMenuRow {
            rank: right(&(i + 1).to_string(), 3),
            name: left(&format!("{} {}", e.name, e.lastname), 21),
            class: left(&class_name(world, e.classnum), 10),
            gang: left(gang_or_none(e), 19),
            edited: left(&e.edited, 6),
        });
    }
    push_all(world, id, msgs);
}

/// The TOPTEN command: the heading (`Class` in front of the titles while a class filter is
/// set), the column titles with the Experience column under EXPINTEN, then the paging
/// state: at most `count` entries, `SUBST_LISTING`, and the first line at once
/// ([`display_next_topten`]). The rest follows on the next poll
/// ([`crate::state::listing::on_tick`]); `X` stops it.
pub fn display_topten(world: &mut World, id: SessionId, count: usize) {
    let expinten = world.config.exp_in_topten;
    let total = world.topten.len();
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.output.push(Msg::ToptenHeader);
    if s.ust.topclass != 0 {
        s.output.push(Msg::ToptenClassMark);
    }
    s.output.push(if expinten {
        Msg::ToptenColumnsExp
    } else {
        Msg::ToptenColumns
    });
    s.ust.listmax = total.min(count) as i16;
    s.ust.listidx = 0;
    s.ust.flags.insert(SessionFlags::TOPTEN);
    s.state = SessionState::Listing;
    display_next_topten(world, id);
}

/// The entry at `listidx`, if it is in use and of the filtered class (numbered by `toprank`
/// then, else by its place), then on to the next; at the end the paging stops, the player
/// is back at `SUBST_PLAYING` and gets the prompt.
pub fn display_next_topten(world: &mut World, id: SessionId) {
    let expinten = world.config.exp_in_topten;
    let Some(s) = world.session(id) else {
        return;
    };
    let idx = s.ust.listidx.max(0) as usize;
    let topclass = s.ust.topclass;
    let mut toprank = s.ust.toprank;
    let msg = world.topten.get(idx).and_then(|e| {
        if e.classnum.0 == 0 || (topclass != 0 && i32::from(topclass) != i32::from(e.classnum.0)) {
            return None;
        }
        let rank = if topclass != 0 {
            let r = toprank;
            toprank += 1;
            i32::from(r)
        } else {
            idx as i32 + 1
        };
        let rank = right(&rank.to_string(), 3);
        let name = left(&format!("{} {}", e.name, e.lastname), 21);
        let class = left(&class_name(world, e.classnum), 10);
        let gang = left(gang_or_none(e), 19);
        Some(if expinten {
            Msg::ToptenRowExp {
                rank,
                name,
                class,
                gang,
                exp: left(&big_exp(e.exphi, e.explo), 12),
                edited: left(&e.edited, 6),
            }
        } else {
            Msg::ToptenRow {
                rank,
                name,
                class,
                gang,
            }
        })
    });
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.ust.toprank = toprank;
    if let Some(m) = msg {
        s.output.push(m);
    }
    s.ust.listidx += 1;
    if s.ust.listidx >= s.ust.listmax {
        s.ust.flags.remove(SessionFlags::TOPTEN);
        s.state = SessionState::Playing;
        s.push_prompt();
    }
}

/// A date as `MM/DD/YY` (the founding date, held here as Unix seconds
/// UTC; days to a civil date by the proleptic Gregorian rule).
fn format_date(unix: i64) -> String {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{month:02}/{day:02}/{:02}", year.rem_euclid(100))
}

/// One gang's line.
fn gang_row(g: &Gang, rank: usize, with_exp: bool) -> Msg {
    let rank = right(&rank.to_string(), 3);
    let name = left(&g.name, 19);
    let leader = left(&g.leader, 11);
    let members = right(&g.members.to_string(), 4);
    let created = format_date(g.created);
    if !with_exp {
        return Msg::GangRow {
            rank,
            name,
            leader,
            members,
            created,
        };
    }
    let mut exp = g.exp.to_string();
    if g.flags.contains(GangFlags::OVERFLOW) {
        if g.overflows != 0 {
            exp.push_str(&format!("\r{}({} times)", right("", 58), g.overflows + 1));
        }
        exp.push_str(&format!("\r{}+{}", right("", 58), g.overexp));
    }
    Msg::GangRowExp {
        rank,
        name,
        leader,
        members,
        created,
        exp,
    }
}

/// The gangs by experience, most first (WCCGANGS key 1), leaving out disbanded and hidden
/// ones, at most `count` shown of the first 200 looked at; the heading before the first, or
/// `There are no gangs currently established!`. A sysop in debug mode, or every player
/// under EXPTOPGANG, gets the experience column.
pub fn display_top_gangs(world: &mut World, id: SessionId, count: usize) {
    let with_exp = world.config.exp_in_top_gang
        || world
            .session(id)
            .is_some_and(|s| s.flags.debug != DebugLevel::Off);
    let mut gangs: Vec<&Gang> = world.gangs.values().collect();
    gangs.sort_by(|a, b| b.exp.cmp(&a.exp).then_with(|| a.key.cmp(&b.key)));
    let mut msgs = Vec::new();
    let mut cnt = 0;
    for g in gangs.into_iter().take(TOPGANGSCAN) {
        if cnt >= count {
            break;
        }
        if g.flags.contains(GangFlags::DISBANDED) || g.flags.contains(GangFlags::HIDDEN) {
            continue;
        }
        cnt += 1;
        if cnt == 1 {
            msgs.push(if with_exp {
                Msg::GangsHeaderExp
            } else {
                Msg::GangsHeader
            });
        }
        msgs.push(gang_row(g, cnt, with_exp));
    }
    if cnt == 0 {
        msgs.push(Msg::GangsNone);
    }
    push_all(world, id, msgs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gang_dates() {
        // 2026-10-14 00:00 UTC
        assert_eq!(format_date(1_791_936_000), "10/14/26");
    }
}
