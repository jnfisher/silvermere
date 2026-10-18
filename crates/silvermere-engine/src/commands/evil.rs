//! FORGIVE: the victim forgives a player in the room who attacked or robbed them
//! ([`crate::evil::attempt_to_forgive`]).

use silvermere_text::Msg;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::SessionId;

use super::dispatch::Handled;
use super::{CommandLine, eq_ignore_case};
use crate::combat::{object_pronoun, player, possessive_pronoun};
use crate::evil::attempt_to_forgive;
use crate::items::inventory::{matches_any_word, show_options};
use crate::output::clear_output;
use crate::tell::tell_user;
use crate::world::World;

/// What the search for another player in the room found.
enum Found {
    None,
    User(SessionId),
    Multiple(Vec<String>),
}

/// The player steps of the action target search, not the searcher: the other players in
/// the room whose first name the word begins (hidden ones only for a player who sees hidden); an
/// exact name ends the search, several candidates are listed, else the last one found.
fn find_user(world: &World, id: SessionId, word: &str) -> Found {
    let Some(plr) = player(world, id) else {
        return Found::None;
    };
    let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    let (room, map) = (plr.room, plr.map);
    let mut found = Found::None;
    let mut options = Vec::new();
    for other in world.session_ids() {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some(v) = player(world, other) else {
            continue;
        };
        if v.room != room || v.map != map || !matches_any_word(word, &v.firstname) {
            continue;
        }
        if v.hidden != 0 && !sees_hidden {
            continue;
        }
        if eq_ignore_case(word, &v.firstname) {
            return Found::User(other);
        }
        found = Found::User(other);
        options.push(v.firstname.clone());
    }
    if options.len() > 1 {
        return Found::Multiple(options);
    }
    found
}

/// `FORGIVE <name>`. The named player is the attacker, the one typing the victim. Without
/// a name the line is left to the next stage; a name that matches no player here is "You do not see
/// <name> here!".
pub fn cmd_forgive(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    if line.argc() < 2 {
        return Handled::NotCommand;
    }
    let word = line.arg(1);
    let found = find_user(world, id, word);
    // the match lookup ends (the colour reset is dropped) unless the name was ambiguous
    if !matches!(found, Found::Multiple(_)) {
        clear_output(world, id);
    }
    match found {
        Found::Multiple(options) => {
            show_options(world, id, &options);
        }
        Found::None => {
            tell_user(world, id, Msg::CombatNotHere { who: word.into() });
        }
        Found::User(tgt) => {
            let him = object_pronoun(player(world, tgt)).to_owned();
            let his = possessive_pronoun(player(world, tgt)).to_owned();
            if attempt_to_forgive(world, tgt, id) {
                // the attacker (with a prompt), then the victim
                if let Some(s) = world.session_mut(tgt) {
                    s.output.push(Msg::ForgiveForgiven);
                    s.push_prompt();
                }
                tell_user(world, id, Msg::ForgiveDone { him, his });
            } else {
                tell_user(world, id, Msg::ForgiveRefused { him, his });
            }
        }
    }
    Handled::Done
}
