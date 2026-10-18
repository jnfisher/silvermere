//! WHO and SCAN, and the menu's `S`/`W`: [`display_scan_of_users`].
//!
//! The list is the sessions in the game in the original's alphabetical order (first names
//! upper cased, ties in channel order). The sysop modes 1 and 2 are T-055's.

use silvermere_text::{Msg, ansi};
use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;

use super::status::{left, right};
use crate::world::World;

/// The narrowest name column.
const MINNAMELEN: usize = 5;
/// The widest name column: a 10-letter first name, a blank and 18 letters.
const MAXNAMELEN: usize = 29;
/// `player.lawful` of a character who chose Lawful.
const FORCEDLAWFUL: i8 = -10;

/// The legal level of an evil-point count: the index into [`LEGAL_NAMES`] and
/// [`legal_colour`].
pub fn legal_level(evil: i16) -> usize {
    match evil {
        e if e < -200 => 7,
        e if e < -50 => 6,
        e if e < 30 => 0,
        e if e < 40 => 1,
        e if e < 80 => 2,
        e if e < 120 => 3,
        e if e < 210 => 4,
        _ => 5,
    }
}

/// The legal level names.
pub const LEGAL_NAMES: [&str; 8] = [
    "Neutral", "Seedy", "Outlaw", "Criminal", "Villain", "FIEND", "Good", "Saint",
];

/// The legal level colours.
pub fn legal_colour(level: usize) -> &'static str {
    let a: &[u8] = match level {
        0 => ansi::ANSI_FGCYAN,
        1 => ansi::ANSI_FGGREY,
        2 => ansi::ANSI_FGRED,
        3 => ansi::ANSI_FGBROWN,
        4 => ansi::ANSI_YELLOW,
        5 => ansi::ANSI_LTRED,
        _ => ansi::ANSI_WHITE,
    };
    std::str::from_utf8(a).expect("ANSI strings are ASCII")
}

/// The players in the game in user-list order: by first name upper cased, ties in channel
/// order.
pub fn players_in_list_order(world: &World) -> Vec<SessionId> {
    let mut ids: Vec<(String, SessionId)> = world
        .session_ids()
        .into_iter()
        .filter(|&id| world.is_in_game(id))
        .filter_map(|id| {
            world
                .session(id)
                .and_then(|s| s.player())
                .map(|p| (p.firstname.to_ascii_uppercase(), id))
        })
        .collect();
    ids.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.0.cmp(&b.1.0)));
    ids.into_iter().map(|(_, id)| id).collect()
}

/// The name column's width, as the user list sets it: the longest "first last" in the game,
/// at least 5 and at most 29.
pub fn max_name_len(world: &World) -> usize {
    players_in_list_order(world)
        .iter()
        .filter_map(|&id| world.session(id).and_then(|s| s.player()))
        .map(|p| p.firstname.chars().count() + p.lastname.chars().count() + 1)
        .fold(MINNAMELEN, usize::max)
        .min(MAXNAMELEN)
}

/// The reputation word: `Lawful` for a forced-lawful character while ENABLAWF, else the
/// legal level's name.
fn reputation(world: &World, plr: &Character) -> &'static str {
    if plr.lawful == FORCEDLAWFUL && world.config.enable_lawful {
        "Lawful"
    } else {
        LEGAL_NAMES[legal_level(plr.evil)]
    }
}

/// One player's line, `None` for a missing record (the original printed `ERROR in users!`
/// for a channel without one; a session in the game here always has its character).
fn row(world: &World, plr: &Character, technical: bool, namelen: usize) -> Msg {
    let name = format!("{} {}", plr.firstname, plr.lastname);
    let edited = if plr.immortal != 0 { "EDITED" } else { " " };
    let level = legal_level(plr.evil);
    if technical {
        return Msg::WhoTechRow {
            title: left(&plr.title, 15),
            name: left(&name, 20),
            flags: format!(
                "{}{}",
                if plr.flags.contains(PlayerFlags::NOGOSSIP) {
                    'g'
                } else {
                    ' '
                },
                if plr.flags.contains(PlayerFlags::NOAUCTION) {
                    'a'
                } else {
                    ' '
                }
            ),
            rep: format!(
                "{}{}",
                legal_colour(level),
                left(reputation(world, plr), 10)
            ),
            gang: left(
                if plr.gang.is_empty() {
                    "None"
                } else {
                    &plr.gang
                },
                19,
            ),
            edited: left(edited, 6),
        };
    }
    // `%8.8s ` of "" for a Neutral player, else in its colour
    let rep = if level == 0 {
        format!("{} ", right("", 8))
    } else {
        format!(
            "{}{} ",
            legal_colour(level),
            right(reputation(world, plr), 8)
        )
    };
    let gossip = if plr.flags.contains(PlayerFlags::NOGOSSIP) {
        'x'
    } else {
        '-'
    };
    let name = left(&name, namelen);
    if plr.gang.is_empty() {
        Msg::WhoRow {
            rep,
            name,
            gossip,
            title: plr.title.clone(),
            edited: edited.into(),
        }
    } else {
        Msg::WhoRowGang {
            rep,
            name,
            gossip,
            title: plr.title.clone(),
            gang: plr.gang.clone(),
            edited: edited.into(),
        }
    }
}

/// The player list for the viewer `id`: the heading before the first player (the Technical
/// message style gets the table), one line per player, then a blank line, or
/// `There are no users in the game at the moment.` The BBS `INVISB` flag that hid a sysop
/// is not carried (no card sets it), so every player in the game is listed; a player who is
/// hiding (`player.hidden`) is listed too, as the original lists him.
pub fn display_scan_of_users(world: &mut World, id: SessionId) {
    let technical = world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.settings.contains(PlayerSettings::TECHNICAL));
    let namelen = max_name_len(world);
    let mut msgs = Vec::new();
    for pid in players_in_list_order(world) {
        let Some(plr) = world.session(pid).and_then(|s| s.player()) else {
            continue;
        };
        if msgs.is_empty() {
            if technical {
                msgs.push(Msg::WhoTechHeader);
            } else {
                msgs.push(Msg::WhoHeader);
                msgs.push(Msg::WhoRule);
            }
        }
        msgs.push(row(world, plr, technical, namelen));
    }
    msgs.push(if msgs.is_empty() {
        Msg::WhoNone
    } else {
        Msg::WhoEnd
    });
    if let Some(s) = world.session_mut(id) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_levels() {
        assert_eq!(legal_level(-201), 7);
        assert_eq!(legal_level(-200), 6);
        assert_eq!(legal_level(-51), 6);
        assert_eq!(legal_level(-50), 0);
        assert_eq!(legal_level(29), 0);
        assert_eq!(legal_level(30), 1);
        assert_eq!(legal_level(40), 2);
        assert_eq!(legal_level(80), 3);
        assert_eq!(legal_level(120), 4);
        assert_eq!(legal_level(210), 5);
    }
}
