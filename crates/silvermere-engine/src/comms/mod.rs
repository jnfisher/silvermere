//! Communication support the comms commands share (T-045): finding a player by name, the
//! per-period page allowance and the forget list.
//!
//! The commands themselves are `commands::comms` (telepath, say-to, broadcast, gossip,
//! auction) and the speech tail of `commands::input` (say, yell).

pub mod allowance;
pub mod forget;

use silvermere_types::ids::SessionId;

use crate::items::inventory::matches_any_word;
use crate::world::World;

/// What `get_user_number` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMatch {
    /// One player.
    One(SessionId),
    /// Nobody.
    None,
    /// The abbreviation fits several players and none in full; the names, in channel
    /// order, for the "Please be more specific" list.
    Several(Vec<String>),
}

/// The player in the game whose first name `name` names. Without `exact` an abbreviation of
/// any word of the name counts (`matches_any_word`); a name equal to a player's in full ends the
/// search at once, ambiguity or not.
pub fn get_user_number(world: &World, name: &str, exact: bool) -> UserMatch {
    let mut found = None;
    let mut names = Vec::new();
    for id in world.session_ids() {
        if !world.is_in_game(id) {
            continue;
        }
        let Some(plr) = world.session(id).and_then(|s| s.player()) else {
            continue;
        };
        let full = name.eq_ignore_ascii_case(&plr.firstname);
        if (!exact && matches_any_word(name, &plr.firstname)) || full {
            names.push(plr.firstname.clone());
            found = Some(id);
            if full {
                return UserMatch::One(id);
            }
        }
    }
    match (found, names.len()) {
        (_, n) if n > 1 => UserMatch::Several(names),
        (Some(id), _) => UserMatch::One(id),
        _ => UserMatch::None,
    }
}

/// [`get_user_number`] in the command of `id`: the search, then `clear_output` on a full name or
/// when at most one abbreviation fits, which takes the dispatcher's `ANSI_NORMAL` out of
/// the buffer. Several matches leave the buffer alone.
pub fn find_user_number(world: &mut World, id: SessionId, name: &str, exact: bool) -> UserMatch {
    let found = get_user_number(world, name, exact);
    if !matches!(found, UserMatch::Several(_)) {
        crate::output::clear_output(world, id);
    }
    found
}
