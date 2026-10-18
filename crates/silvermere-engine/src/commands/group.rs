//! Groups: follow, join, leave, invite, uninvite, disband party, the ranks, party, aid
//! and share. The party state and the routines that change it are [`crate::party`]; the PARTY
//! screen is [`crate::display::party`].
//!
//! The gang forms (INVITE/UNINVITE MEMBER, JOIN GANG, LEAVE GANG, DISBAND GANG) are
//! T-044's: each handler tries `gang`'s form first.

use silvermere_text::Msg;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;

use super::combat::{Found, find_action_target};
use super::dispatch::Handled;
use super::inventory::{leading_int32, match_currency};
use super::{Cmd, CommandLine, eq_ignore_case, gang, starts_with_ignore_case};
use crate::combat::{object_pronoun, possessive_pronoun};
use crate::comms::forget::is_user_forgotten;
use crate::gangs::{Target, find_any_action_target, find_room_user};
use crate::items::inventory::show_options;
use crate::movement::exits::prompt;
use crate::output::clear_output;
use crate::party::{
    RANK_FRONT, Rank, add_to_group, share_currency, stop_following, stop_following_all,
};
use crate::tell::{tell_group, tell_user};
use crate::world::World;

/// The highest broadcast channel JOIN takes.
const MAXCHAN: i32 = 255;

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

/// Ends resting and meditating.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// Ends sneaking and hiding: a visible act.
fn unhide(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
}

fn leader_of(world: &World, id: SessionId) -> Option<SessionId> {
    world.session(id).and_then(|s| s.party.leader)
}

fn name_of(world: &World, id: SessionId) -> String {
    player(world, id).map_or_else(String::new, |p| p.firstname.clone())
}

/// LEAVE, but for LEAVE GANG (T-044): a follower stops following; a player on a broadcast channel
/// tells it, leaves it and forgets his inviter; anything else, or more than one word after LEAVE,
/// is not a command.
pub fn cmd_leave(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if let Some(h) = gang::leave_gang(world, id, line) {
        return h;
    }
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if line.argc() > 2 {
        return Handled::NotCommand;
    }
    let channel = plr.bchannel;
    let who = plr.firstname.clone();
    if let Some(l) = leader_of(world, id) {
        stop_following(world, Some(l), id);
    } else if channel != 0 {
        let n = i32::from(channel);
        tell_group(world, channel, Some(id), &Msg::GroupLeftOther { who, n });
        tell_user(world, id, Msg::GroupLeft { n });
        if let Some(s) = world.session_mut(id) {
            s.ust.inviter.clear();
        }
        if let Some(p) = player_mut(world, id) {
            p.bchannel = 0;
        }
    } else {
        return Handled::NotCommand;
    }
    Handled::Done
}

/// JOIN, but for JOIN GANG (T-044): a number from 1 to 255, or `0` written out, moves the player to
/// that broadcast channel (the old one told he left, the new one that he joined); anything else is
/// FOLLOW.
pub fn cmd_join(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    if let Some(h) = gang::join_gang(world, id, line) {
        return h;
    }
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if line.argc() < 2 {
        tell_user(world, id, Msg::JoinSyntax);
        return Handled::Done;
    }
    let chan = leading_int32(line.arg(1));
    if !(1..=MAXCHAN).contains(&chan) && (chan != 0 || !eq_ignore_case(line.arg(1), "0")) {
        return cmd_follow(world, id, cmd, line);
    }
    let who = plr.firstname.clone();
    let old = plr.bchannel;
    if old != 0 {
        let msg = Msg::ChannelLeftOther {
            who: who.clone(),
            n: i32::from(old),
        };
        tell_group(world, old, Some(id), &msg);
    }
    // the channel is stored in a char
    let chan = chan as i16;
    if let Some(p) = player_mut(world, id) {
        p.bchannel = chan;
    }
    let n = i32::from(chan);
    tell_group(world, chan, Some(id), &Msg::ChannelJoinedOther { who, n });
    tell_user(world, id, Msg::ChannelJoined { n });
    Handled::Done
}

/// FOLLOW alone stops following (or shows the syntax); FOLLOW {name} joins the party of the player
/// named in the room when he has invited us: our own list keeps only him, the party is linked, both
/// are told, and the invitation is used up.
pub fn cmd_follow(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        match leader_of(world, id) {
            Some(l) => stop_following(world, Some(l), id),
            None => tell_user(world, id, Msg::FollowSyntax),
        }
        return Handled::Done;
    }
    // players and monsters, not the searcher; charmed monsters do not come last
    let found = find_action_target(world, id, line.rest_from(1), true);
    if !matches!(found, Found::Multiple(_)) {
        clear_output(world, id);
    }
    match found {
        Found::Multiple(names) => show_options(world, id, &names),
        Found::User(tgt) if tgt == id => tell_user(world, id, Msg::FollowSelf),
        Found::User(tgt) => {
            if let Some(s) = world.session_mut(id) {
                for slot in &mut s.party.group {
                    if slot.is_some() && *slot != Some(tgt) {
                        *slot = None;
                    }
                }
            }
            if add_to_group(world, tgt, id, true) {
                if let Some(s) = world.session_mut(id) {
                    s.party.leader = Some(tgt);
                }
                let leader = name_of(world, tgt);
                tell_user(world, id, Msg::NowFollowing { who: leader });
                let me = name_of(world, id);
                tell_user(world, tgt, Msg::StartedFollow { who: me });
                prompt(world, tgt);
                if let Some(s) = world.session_mut(id) {
                    s.ust.inviter.clear();
                }
            } else {
                tell_user(world, id, Msg::InviteFirst);
            }
        }
        Found::Monster(_) => tell_user(world, id, Msg::FollowMonster),
        Found::None => tell_user(world, id, Msg::FollowNothing),
    }
    Handled::Done
}

/// INVITE, but for INVITE MEMBER (T-044): the player named in the room is written into the
/// inviter's list (when there is a free slot and he is not listed yet; only then is he told), the
/// inviter goes to the front rank, and the invitee's inviter is noted. Not someone hidden from the
/// inviter, nor someone who has the inviter on his forget list.
pub fn cmd_invite(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if let Some(h) = gang::invite_member(world, id, line) {
        return h;
    }
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        tell_user(world, id, Msg::InviteSyntax);
        return Handled::Done;
    }
    let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    let me = plr.firstname.clone();
    unhide(world, id);
    let name = line.rest_from(1).to_string();
    let found = find_room_user(world, id, &name);
    if !matches!(found, Target::Multiple(_)) {
        clear_output(world, id);
    }
    let tgt = match found {
        Target::Multiple(names) => {
            show_options(world, id, &names);
            return Handled::Done;
        }
        Target::None => {
            tell_user(world, id, Msg::GangNotHere { who: name });
            return Handled::Done;
        }
        Target::Monster => {
            tell_user(world, id, Msg::InviteSyntax);
            return Handled::Done;
        }
        Target::User(t) => t,
    };
    if tgt == id {
        tell_user(world, id, Msg::GangInviteSelf);
        return Handled::Done;
    }
    let hidden = player(world, tgt).is_some_and(|t| t.hidden != 0);
    if hidden && !sees_hidden {
        tell_user(world, id, Msg::GangNotHere { who: name });
    } else if is_user_forgotten(world, tgt, id) {
        tell_user(world, id, Msg::GangInviteForgotten { who: name });
    } else {
        if let Some(p) = player_mut(world, id) {
            p.rank = RANK_FRONT;
        }
        if let Some(s) = world.session_mut(tgt) {
            s.ust.inviter = me.clone();
        }
        if add_to_group(world, id, tgt, false) {
            let whom = object_pronoun(player(world, id)).to_owned();
            tell_user(world, tgt, Msg::InvitedYouFollow { who: me, whom });
            prompt(world, tgt);
        }
        let who = name_of(world, tgt);
        tell_user(world, id, Msg::YouInvitedFollow { who });
    }
    Handled::Done
}

/// UNINVITE, but for UNINVITE MEMBER (T-044): the player named (in the room, else anywhere in the
/// game) stops following and leaves the list; an empty list puts the player back in the middle
/// rank.
pub fn cmd_uninvite(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if let Some(h) = gang::uninvite_member(world, id, line) {
        return h;
    }
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    if line.argc() == 1 {
        tell_user(world, id, Msg::UninviteSyntax);
        return Handled::Done;
    }
    let name = line.rest_from(1).to_string();
    let mut target = find_room_user(world, id, &name);
    if let Target::Multiple(names) = target {
        show_options(world, id, &names);
        return Handled::Done;
    }
    clear_output(world, id);
    if target == Target::None {
        target = find_any_action_target(world, &name);
    }
    let Target::User(tgt) = target else {
        tell_user(world, id, Msg::UninviteSyntax);
        return Handled::Done;
    };
    if tgt == id {
        tell_user(world, id, Msg::GangUninviteSelf);
        return Handled::Done;
    }
    stop_following(world, Some(id), tgt);
    let mut found = false;
    if let Some(s) = world.session_mut(id) {
        for slot in &mut s.party.group {
            if *slot == Some(tgt) {
                *slot = None;
            }
            found |= slot.is_some();
        }
    }
    if !found && let Some(p) = player_mut(world, id) {
        p.rank = 0;
    }
    let who = name_of(world, tgt);
    tell_user(world, id, Msg::FollowerRemoved { who });
    Handled::Done
}

/// DISBAND, but for DISBAND GANG (T-044): DISBAND PARTY makes all the player's followers stop
/// following him.
pub fn cmd_disband(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if let Some(h) = gang::disband(world, id, line) {
        return h;
    }
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    stop_following_all(world, id);
    Handled::Done
}

/// SHARE {amount} {currency} [WITH ROOM/PARTY], the party by default ([`share_currency`]).
pub fn cmd_share(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() < 3 {
        tell_user(world, id, Msg::ShareSyntaxDefault);
        return Handled::Done;
    }
    unhide(world, id);
    let amount = leading_int32(line.arg(1));
    if amount <= 0 {
        tell_user(world, id, Msg::ShareSyntax);
        return Handled::Done;
    }
    match match_currency(world, line.arg(2)) {
        Some(cur) if line.argc() == 3 || starts_with_ignore_case(line.arg(3), "with") => {
            let in_room = line.argc() > 4 && starts_with_ignore_case(line.arg(4), "room");
            share_currency(world, id, amount, cur, in_room);
        }
        _ => tell_user(world, id, Msg::ShareSyntax),
    }
    Handled::Done
}

/// AID: a player in the room who is down (no hit points) is marked as aided (he heals one point a
/// slow tick instead of bleeding, T-037), and both are told.
pub fn cmd_aid(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let me = plr.firstname.clone();
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::AidSyntax);
        return Handled::Done;
    }
    unhide(world, id);
    let found = find_room_user(world, id, line.rest_from(1));
    // the match lookup ends (the colour reset is dropped) unless the name was ambiguous
    if !matches!(found, Target::Multiple(_)) {
        clear_output(world, id);
    }
    let tgt = match found {
        Target::Multiple(names) => {
            show_options(world, id, &names);
            return Handled::Done;
        }
        Target::User(t) => t,
        Target::None | Target::Monster => {
            tell_user(world, id, Msg::AidSyntax);
            return Handled::Done;
        }
    };
    let Some(tplr) = player(world, tgt).cloned() else {
        return Handled::Done;
    };
    if tplr.hp > 0 {
        let who = tplr.firstname;
        tell_user(world, id, Msg::AidNotNeeded { who });
        return Handled::Done;
    }
    if let Some(t) = player_mut(world, tgt) {
        t.aided = 1;
    }
    let his = possessive_pronoun(Some(&tplr)).to_owned();
    tell_user(
        world,
        id,
        Msg::Aided {
            who: tplr.firstname.clone(),
            his,
        },
    );
    tell_user(world, tgt, Msg::AidedYou { who: me });
    prompt(world, tgt);
    Handled::Done
}

/// The rank change for FRONTRANK 1, MIDRANK 0, BACKRANK 2 ([`crate::party::change_user_rank`]).
pub fn change_user_rank(world: &mut World, id: SessionId, cmd: Cmd) {
    let rank = match cmd {
        Cmd::FrontRank => Rank::Front,
        Cmd::BackRank => Rank::Back,
        _ => Rank::Middle,
    };
    crate::party::change_user_rank(world, id, rank);
}

/// PARTY ([`crate::display::party::display_party`]).
pub fn display_party(world: &mut World, id: SessionId) {
    crate::display::party::display_party(world, id);
}
