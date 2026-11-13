//! Social actions.
//!
//! [`Actions`] is stage 10 of `execute_input`: a line that no command, exit, room script or
//! spell name claimed is offered to the verb table ([`handle_social_actions`]). The verbs are
//! `World::actions` (WCCACTS), loaded once with the world.
//!
//! ASK and GREET (game commands, not social actions) are `ask_greet` (T-054).

use silvermere_text::colour::Palette;
use silvermere_text::{Msg, StatlineCtx, substitute};
use silvermere_types::flags::{ActionTargets, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, RoomKey, SessionId};
use silvermere_types::records::{Action, Character};
use silvermere_types::sizes::NACTTEXTS;

use super::CommandLine;
use super::cast::{Found, find_action_target as search};
use super::input::SocialAction;
use crate::display::actions::list_of_actions;
use crate::items::inventory::matches_any_word;
use crate::tell::{Noise, TellBlind, pronouns, tell_room};
use crate::world::World;
use silvermere_rules::spells::MatchKinds;

/// Text indices of an action record.
pub const TXT_SOLOSELF: usize = 0;
pub const TXT_SOLOROOM: usize = 1;
pub const TXT_USERSELF: usize = 2;
pub const TXT_USERTARGET: usize = 3;
pub const TXT_USERROOM: usize = 4;
pub const TXT_MONSELF: usize = 5;
pub const TXT_MONROOM: usize = 6;
pub const TXT_INVSELF: usize = 7;
pub const TXT_INVROOM: usize = 8;
pub const TXT_FLOORSELF: usize = 9;
pub const TXT_FLOORROOM: usize = 10;
const _: () = assert!(NACTTEXTS == 11);

/// The colour table row `tell_room` colours the room's line with.
const COL_ACTION: u8 = silvermere_text::colour::ColRow::Action as u8;

/// Stage 10's hook: [`handle_social_actions`].
pub struct Actions;

impl SocialAction for Actions {
    fn handle(&self, world: &mut World, id: SessionId, line: &str) -> bool {
        handle_social_actions(world, id, line)
    }
}

/// The action verb `word` names: the whole word, case not counting, no abbreviations. The record
/// and its target mask, or `None` for a word that is no verb or a verb whose mask is 0 (which
/// "callers read as not an action"). A stored name with white space in it is never found, as in the
/// original (the table's key drops the white space, the stored name keeps it).
pub fn find_mud_action<'a>(world: &'a World, word: &str) -> Option<&'a Action> {
    let act = world.actions.get(&word.to_ascii_uppercase())?;
    (!act.targets.is_empty()).then_some(act)
}

/// What the name after the verb referred to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    None,
    User(SessionId),
    Monster(ActiveMonsterId),
    Floor(ItemNo),
    Inventory(ItemNo),
    /// Several matches: the list is discarded when the match lookup ends.
    Multiple,
}

/// The action target search as social actions ask it: the kinds the verb's mask allows
/// (the unused search bit is not carried).
fn find_action_target(
    world: &mut World,
    id: SessionId,
    name: &str,
    allowed: ActionTargets,
) -> Target {
    let kinds = MatchKinds {
        users: allowed.contains(ActionTargets::USER),
        monsters: allowed.contains(ActionTargets::MONSTER),
        inventory: allowed.contains(ActionTargets::INVENTORY),
        floor: allowed.contains(ActionTargets::FLOOR),
        ..MatchKinds::default()
    };
    match search(world, id, name, kinds) {
        Found::None | Found::Spell(_) => Target::None,
        Found::User(n) => Target::User(n),
        Found::Monster(n) => Target::Monster(n),
        Found::Floor(n) => Target::Floor(n),
        Found::Inventory(n) => Target::Inventory(n),
        Found::Multiple(..) => Target::Multiple,
    }
}

/// `fill_names` as the social actions use it: each `%s` takes the next name, a `%s` too many prints
/// nothing (every call ends with an extra `""`), `%%` is one percent sign.
pub fn fill_names(template: &str, names: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = names.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('s') => {
                chars.next();
                out.push_str(next.next().copied().unwrap_or(""));
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

/// The prompt-style substitution over the actor, then [`fill_names`] with `names`. `target` is the
/// text `%t` prints (`None`: `nothing`).
pub fn render(
    world: &World,
    id: SessionId,
    template: &str,
    target: Option<&str>,
    names: &[&str],
) -> String {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return String::new();
    };
    fill_names(&substitute_for(plr, template, target), names)
}

fn substitute_for(plr: &Character, template: &str, target: Option<&str>) -> String {
    let (possessive, object) = pronouns(plr.sex);
    let ctx = StatlineCtx {
        hp: i32::from(plr.hp),
        maxhp: i32::from(plr.maxhp),
        mana: i32::from(plr.mana),
        maxmana: i32::from(plr.maxmana),
        exp: plr.exp,
        // as tell::prompt_bytes: the exp needed and the wealth are other cards'
        exp_needed: None,
        wealth_copper: 0,
        first_name: &plr.firstname,
        evil_warn: plr.settings.contains(PlayerSettings::EVILWARN),
        resting: plr.status.contains(PlayerStatus::RESTING),
        meditating: plr.status2.contains(PlayerStatus2::MEDITATING),
        pronoun_possessive: possessive,
        pronoun_object: object,
        title: target,
        palette: Palette::repaired(plr.palette),
    };
    substitute(template, &ctx)
}

/// A social action. `true` when the line was performed (every line and prompt has been queued),
/// `false` when it was no action and nothing was shown to anybody.
pub fn handle_social_actions(world: &mut World, id: SessionId, line: &str) -> bool {
    let cl = CommandLine::parse(line);
    if cl.argc() == 0 {
        return false;
    }
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    if plr.settings.contains(PlayerSettings::NOACTIONS) {
        return false;
    }
    let Some(act) = find_mud_action(world, cl.arg(0)).cloned() else {
        return false;
    };
    let allowed = act.targets;
    let me = plr.firstname.clone();
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };

    if cl.argc() < 2 {
        // the verb alone
        if !allowed.contains(ActionTargets::SINGLE) {
            return false;
        }
        reveal(world, id);
        let poss = pronouns(plr_sex(world, id)).0;
        let own = render(world, id, &act.texts[TXT_SOLOSELF], None, &[]);
        say_self(world, id, own);
        let room = render(world, id, &act.texts[TXT_SOLOROOM], None, &[&me, poss]);
        say_room(world, key, [Some(id), None], room);
        return true;
    }

    let name = cl.rest_from(1);
    let mut target = find_action_target(world, id, name, allowed);
    match target {
        Target::Multiple => return false,
        Target::None => {
            // the player named himself
            if !allowed.contains(ActionTargets::USER) || !matches_any_word(name, &me) {
                return false;
            }
            target = Target::User(id);
        }
        _ => {}
    }
    match target {
        Target::User(tgt) => {
            if !allowed.contains(ActionTargets::USER) {
                return false;
            }
            let Some(tplr) = world.session(tgt).and_then(|s| s.player()) else {
                return false;
            };
            // another player who is hidden or sneaking can't be seen
            if tgt != id && (tplr.hidden != 0 || tplr.status.contains(PlayerStatus::SNEAKING)) {
                return false;
            }
            let them = tplr.firstname.clone();
            reveal(world, id);
            let own = render(world, id, &act.texts[TXT_USERSELF], Some(&them), &[&them]);
            say_self(world, id, own);
            let to_target = render(world, id, &act.texts[TXT_USERTARGET], Some(&them), &[&me]);
            if let Some(s) = world.session_mut(tgt) {
                s.output.push(Msg::ActionTarget { text: to_target });
                s.push_prompt();
            }
            let room = render(
                world,
                id,
                &act.texts[TXT_USERROOM],
                Some(&them),
                &[&me, &them],
            );
            say_room(world, key, [Some(id), Some(tgt)], room);
        }
        Target::Monster(m) => {
            if !allowed.contains(ActionTargets::MONSTER) {
                return false;
            }
            let Some(mon) = world.active_monsters.by_number(m) else {
                return false;
            };
            let mname = mon.name.clone();
            reveal(world, id);
            let own = render(world, id, &act.texts[TXT_MONSELF], Some(&mname), &[&mname]);
            say_self(world, id, own);
            let room = render(
                world,
                id,
                &act.texts[TXT_MONROOM],
                Some(&mname),
                &[&me, &mname],
            );
            say_room(world, key, [Some(id), None], room);
        }
        Target::Floor(n) => {
            if !allowed.contains(ActionTargets::FLOOR) {
                return false;
            }
            let Some(iname) = world.items.get(&n).map(|i| i.name.clone()) else {
                return false;
            };
            reveal(world, id);
            let own = render(
                world,
                id,
                &act.texts[TXT_FLOORSELF],
                Some(&iname),
                &[&iname],
            );
            say_self(world, id, own);
            let room = render(
                world,
                id,
                &act.texts[TXT_FLOORROOM],
                Some(&iname),
                &[&me, &iname],
            );
            say_room(world, key, [Some(id), None], room);
        }
        Target::Inventory(n) => {
            if !allowed.contains(ActionTargets::INVENTORY) {
                return false;
            }
            let Some(iname) = world.items.get(&n).map(|i| i.name.clone()) else {
                return false;
            };
            reveal(world, id);
            let poss = pronouns(plr_sex(world, id)).0;
            let own = render(world, id, &act.texts[TXT_INVSELF], Some(&iname), &[&iname]);
            say_self(world, id, own);
            let room = render(
                world,
                id,
                &act.texts[TXT_INVROOM],
                Some(&iname),
                &[&me, poss, &iname],
            );
            say_room(world, key, [Some(id), None], room);
        }
        Target::None | Target::Multiple => return false,
    }
    true
}

fn plr_sex(world: &World, id: SessionId) -> u8 {
    world
        .session(id)
        .and_then(|s| s.player())
        .map_or(b' ', |p| p.sex)
}

/// "Acting gives the player away": hiding and sneaking end.
fn reveal(world: &mut World, id: SessionId) {
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.hidden = 0;
        p.status.remove(PlayerStatus::SNEAKING);
    }
}

/// The actor's line in the actor's `COL_ACTION` colour, then the actor's prompt.
fn say_self(world: &mut World, id: SessionId, text: String) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::ActionSelf { text });
        s.push_prompt();
    }
}

/// A line to the room but `except`, always heard, seen only by those who can see, with a
/// prompt, in the action colour.
fn say_room(world: &mut World, key: RoomKey, except: [Option<SessionId>; 2], text: String) {
    tell_room(
        world,
        key,
        except,
        Noise::Always,
        TellBlind::Sight,
        true,
        Some(COL_ACTION),
        &Msg::ActionRoom { text },
    );
}

/// `ACTION` alone or `ACTION LIST`: the list of actions.
pub fn display_list_of_actions(world: &mut World, id: SessionId) {
    let msg = list_of_actions(&world.action_order);
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::fill_names;

    // each `%s` takes the next name; a `%s` too many prints nothing.
    #[test]
    fn fill_names_fills_in_order_and_blanks_the_surplus() {
        assert_eq!(
            fill_names("%s hugs %s %s!", &["Al", "her", "sword"]),
            "Al hugs her sword!"
        );
        assert_eq!(fill_names("%s waves %s.", &["Al"]), "Al waves .");
        assert_eq!(fill_names("%s waves!", &["Al", "her"]), "Al waves!");
        assert_eq!(fill_names("100%% sure", &[]), "100% sure");
        assert_eq!(fill_names("a % b", &[]), "a % b");
    }
}
