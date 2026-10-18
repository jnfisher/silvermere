//! Gangs: the gang commands and the gang shop.
//!
//! CREATE, BROADGANG, PROMOTE and DEMOTE are this module's commands; STOCK, UNSTOCK and
//! MARKUP are registered in `shop` and come here. INVITE, UNINVITE, JOIN, LEAVE and
//! DISBAND are shared with groups (T-043): their handlers in `group` call the gang forms
//! below first ([`invite_member`], [`uninvite_member`], [`join_gang`], [`leave_gang`],
//! [`disband`]), which answer `None` for a line that is not the gang form.
//!
//! The record keeping (invitations, membership, gang exp, the shop takings) is
//! [`crate::gangs`].

use silvermere_rules::abilities::item_has_ability;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Currency, DebugLevel, RoomType, SessionState, ShopType};
use silvermere_types::flags::{GangFlags, PlayerFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ItemNo, RoomKey, SessionId, ShopNo};
use silvermere_types::names::CURRENCY_NAMES;
use silvermere_types::records::{Character, Gang};
use silvermere_types::sizes::NINVITEMS;

use super::dispatch::Handled;
use super::inventory::leading_int32;
use super::{Cmd, CommandLine, eq_ignore_case, starts_with_ignore_case};
use crate::comms::forget::is_user_forgotten;
use crate::display::gang::{display_gang_members, display_online_gang_members};
use crate::gangs::{
    GANG_MINEXP, GANGNAMELEN, GangQuery, OfflineOp, Target, ask, find_any_action_target,
    find_room_user, gang_key, gang_shop_item_value, get_gang, invite_to_gang, remove_from_gang,
    save_gang, save_gang_shop, set_gang_shop_owner,
};
use crate::items::inventory::{
    Search, add_item_to_inventory, find_item_in_inventory, remove_item_from_inventory,
    show_options, user_is_wearing,
};
use crate::items::room_items::add_item_to_room;
use crate::output::clear_output;
use crate::shops::{GANG_SLOTS, get_shop_item_from_name};
use crate::tell::{Noise, TellBlind, tell_gang, tell_room, tell_user};
use crate::world::World;

/// Most copies of an item in one gang shop slot.
const GANGMAXQTY: u16 = 20;
/// The highest asking price.
const GANGMAXPRICE: u32 = 9999;
/// The highest markup percentage.
const MAXMARKUP: i32 = 1000;
/// The highest broadcast channel JOIN takes.
const MAXCHAN: i32 = 255;

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

/// The player's gang record, when the player is in a gang that exists.
fn my_gang(world: &World, id: SessionId) -> Option<Gang> {
    let plr = player(world, id)?;
    if plr.gang.is_empty() {
        return None;
    }
    get_gang(world, &plr.gang).cloned()
}

fn room_of(plr: &Character) -> RoomKey {
    RoomKey {
        map: plr.map,
        room: plr.room,
    }
}

/// A message to everyone else in the room, always heard, by the blind too, with a prompt
/// and no colour.
fn tell_others(world: &mut World, id: SessionId, msg: &Msg) {
    let Some(room) = player(world, id).map(room_of) else {
        return;
    };
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        msg,
    );
}

/// A message to another player followed by his prompt.
fn tell_with_prompt(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
        s.push_prompt();
    }
}

fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// `Lieutenant ` or `Gang leader `.
fn rank_word(plr: &Character) -> String {
    if plr.flags.contains(PlayerFlags::LIEUTENANT) {
        "Lieutenant ".into()
    } else {
        "Gang leader ".into()
    }
}

/// CREATE GANG (or GUILD) {name} founds a gang with the player as its leader; CREATE ROOM
/// {direction} only shows the gang house lease line; any other word ends quietly. Fewer than two
/// words after CREATE is not a command.
pub fn cmd_create(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    if line.argc() < 3 {
        return Handled::NotCommand;
    }
    if eq_ignore_case(line.arg(1), "gang") || eq_ignore_case(line.arg(1), "guild") {
        if plr.exprem < GANG_MINEXP && plr.expbil == 0 {
            tell_user(world, id, Msg::GangCreateInexperienced);
        } else if !plr.gang.is_empty() {
            tell_user(world, id, Msg::GangCreateAlready);
        } else {
            create_gang(world, id, &plr, line.rest_from(2));
        }
    } else if eq_ignore_case(line.arg(1), "room") {
        // the owned-room checks are made and their result is not used
        tell_user(world, id, Msg::ShopBuyRoom);
    }
    Handled::Done
}

/// The name rules and the insert of CREATE GANG.
fn create_gang(world: &mut World, id: SessionId, plr: &Character, name: &str) {
    if name.len() > GANGNAMELEN {
        tell_user(world, id, Msg::GangCreateTooLong { name: name.into() });
        return;
    }
    if eq_ignore_case(name, "None") {
        tell_user(world, id, Msg::GangCreateNone);
        return;
    }
    // every character printable, space to '~'
    if name.bytes().any(|b| !(b' '..=b'~').contains(&b)) {
        tell_user(world, id, Msg::GangCreateInvalid);
        return;
    }
    let key = gang_key(name);
    if let Some(g) = world.gangs.get(&key).cloned() {
        tell_user(world, id, Msg::GangCreateTaken);
        if !g.flags.contains(GangFlags::DISBANDED) {
            tell_user(
                world,
                id,
                Msg::GangCreateLeaderOf {
                    leader: g.leader,
                    gang: g.name,
                },
            );
        }
        return;
    }
    let gang = Gang {
        key: key.clone(),
        name: name.to_string(),
        exp: 0,
        leader: plr.firstname.clone(),
        created: world.today,
        members: 1,
        flags: GangFlags::empty(),
        overexp: 0,
        overflows: 0,
    };
    world.gangs.insert(key.clone(), gang);
    save_gang(world, &key);
    if let Some(p) = player_mut(world, id) {
        p.gang = name.to_string();
    }
    tell_user(world, id, Msg::GangCreated);
}

/// BROADGANG {text} (or `'text`/`-text`) to every member in the game, the speaker included, each
/// with a prompt (and the speaker the command's prompt as well). The word alone lists the members:
/// only those in the game under SET GANG ONLINE, all of them otherwise.
pub fn cmd_broadgang(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    if line.argc() > 1 && !plr.gang.is_empty() {
        let first = line.first();
        let text = if matches!(first.as_bytes().first(), Some(b'\'' | b'-')) {
            &line.rest_from(0)[1..]
        } else {
            line.rest_from(1)
        };
        let msg = Msg::GangPaths {
            who: plr.firstname.clone(),
            msg: text.to_string(),
        };
        tell_gang(world, &plr.gang, &msg);
        Handled::Done
    } else if line.argc() == 1 && !plr.gang.is_empty() {
        // no command prompt: the list brings the prompt
        if plr.flags.contains(PlayerFlags::GANGONLINE) {
            display_online_gang_members(world, id, &plr.gang);
        } else {
            display_gang_members(world, id, &plr.gang);
        }
        Handled::Wait
    } else {
        tell_user(world, id, Msg::GangNotAtPresent);
        Handled::Done
    }
}

/// The gang form of INVITE: INVITE MEMBER {name} by the leader or a lieutenant. `None` when the
/// line is not that form (fewer than three words, no gang, or a second word that does not
/// abbreviate MEMBER).
pub fn invite_member(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let plr = player(world, id)?.clone();
    if line.argc() < 3 || plr.gang.is_empty() {
        return None;
    }
    let g = get_gang(world, &plr.gang)?.clone();
    if !starts_with_ignore_case(line.arg(1), "member") {
        return None;
    }
    // a visible act
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
    if !(eq_ignore_case(&g.leader, &plr.firstname) || plr.flags.contains(PlayerFlags::LIEUTENANT)) {
        tell_user(world, id, Msg::GangInviteNotRank);
        return Some(Handled::Done);
    }
    let name = line.rest_from(2);
    // (sic) the name shown is the rest of the line from the second word: "member <name>"
    let shown = line.rest_from(1).to_string();
    let mut target = find_room_user(world, id, name);
    if target == Target::None {
        target = find_any_action_target(world, name);
    }
    let tgt = match target {
        Target::Multiple(names) => {
            show_options(world, id, &names);
            return Some(Handled::Done);
        }
        Target::None => {
            clear_output(world, id);
            tell_user(world, id, Msg::GangNotHere { who: shown });
            return Some(Handled::Done);
        }
        Target::Monster => {
            clear_output(world, id);
            tell_user(world, id, Msg::GangInviteSyntax);
            return Some(Handled::Done);
        }
        Target::User(t) => t,
    };
    clear_output(world, id);
    if tgt == id {
        tell_user(world, id, Msg::GangInviteSelf);
        return Some(Handled::Done);
    }
    let Some(tplr) = player(world, tgt).cloned() else {
        return Some(Handled::Done);
    };
    if tplr.hidden != 0 && !plr.status.contains(PlayerStatus::SEEHIDDEN) {
        tell_user(world, id, Msg::GangNotHere { who: shown });
    } else if is_user_forgotten(world, tgt, id) {
        tell_user(world, id, Msg::GangInviteForgotten { who: shown });
    } else {
        if invite_to_gang(world, tgt, &plr.gang) {
            tell_with_prompt(
                world,
                tgt,
                Msg::GangInvitedYou {
                    rank: rank_word(&plr),
                    who: plr.firstname.clone(),
                    gang: plr.gang.clone(),
                },
            );
        }
        tell_user(
            world,
            id,
            Msg::GangYouInvited {
                who: tplr.firstname.clone(),
            },
        );
    }
    Some(Handled::Done)
}

/// The gang form of UNINVITE: UNINVITE MEMBER {name} by the leader or a lieutenant. The leader
/// cannot be removed, a lieutenant only by the leader; a name that is no player in the game (or a
/// monster's) is looked for among the stored characters. `None` when the line is not that form.
pub fn uninvite_member(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let plr = player(world, id)?.clone();
    if line.argc() < 3 || plr.gang.is_empty() {
        return None;
    }
    let g = get_gang(world, &plr.gang)?.clone();
    if !starts_with_ignore_case(line.arg(1), "member") {
        return None;
    }
    let leader = eq_ignore_case(&g.leader, &plr.firstname);
    if !(leader || plr.flags.contains(PlayerFlags::LIEUTENANT)) {
        tell_user(world, id, Msg::GangUninviteNotRank);
        return Some(Handled::Done);
    }
    if eq_ignore_case(&g.leader, line.arg(2)) {
        tell_user(world, id, Msg::GangUninviteLeader);
        return Some(Handled::Done);
    }
    let name = line.rest_from(2).to_string();
    let mut target = find_room_user(world, id, &name);
    if let Target::Multiple(names) = target {
        show_options(world, id, &names);
        return Some(Handled::Done);
    }
    clear_output(world, id);
    if target == Target::None {
        target = find_any_action_target(world, &name);
    }
    let tgt = match target {
        Target::None | Target::Monster => {
            // a member not in the game: the answer comes from the store
            ask(
                world,
                id,
                GangQuery::Offline {
                    name,
                    gang: plr.gang.clone(),
                    op: OfflineOp::Uninvite,
                },
            );
            return Some(Handled::Wait);
        }
        Target::Multiple(_) => {
            tell_user(world, id, Msg::GangUninviteSyntax);
            return Some(Handled::Done);
        }
        Target::User(t) => t,
    };
    if tgt == id {
        tell_user(world, id, Msg::GangUninviteSelf);
        return Some(Handled::Done);
    }
    let Some(tplr) = player(world, tgt).cloned() else {
        return Some(Handled::Done);
    };
    if tplr.flags.contains(PlayerFlags::LIEUTENANT) && !leader {
        tell_user(world, id, Msg::GangUninviteLieutenant);
    } else if eq_ignore_case(&tplr.firstname, &g.leader)
        && plr.flags.contains(PlayerFlags::LIEUTENANT)
    {
        tell_user(world, id, Msg::GangUninviteInsolence);
    } else if remove_from_gang(world, tgt, &plr.gang) {
        tell_with_prompt(
            world,
            tgt,
            Msg::GangExiledYou {
                rank: rank_word(&plr),
                who: plr.firstname.clone(),
                gang: plr.gang.clone(),
            },
        );
        tell_user(
            world,
            id,
            Msg::GangYouRemoved {
                who: tplr.firstname.clone(),
            },
        );
    } else {
        tell_user(
            world,
            id,
            Msg::GangNotInYourGang {
                who: tplr.firstname.clone(),
            },
        );
    }
    Some(Handled::Done)
}

/// The gang form of JOIN: JOIN GANG {name} for a player in no gang (`join_gang` checks the
/// invitation), then the record is saved. `None` when the line is not that form (a channel number,
/// or fewer than three words).
pub fn join_gang(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let plr = player(world, id)?.clone();
    if line.argc() < 2 {
        return None;
    }
    let chan = leading_int32(line.arg(1));
    let channel = (1..=MAXCHAN).contains(&chan) || (chan == 0 && eq_ignore_case(line.arg(1), "0"));
    if channel || line.argc() <= 2 || !starts_with_ignore_case(line.arg(1), "gang") {
        return None;
    }
    if plr.gang.is_empty() {
        crate::gangs::join_gang(world, id, line.rest_from(2));
        crate::player::save_player(world, id);
    } else {
        tell_user(world, id, Msg::GangJoinAlready);
    }
    Some(Handled::Done)
}

/// The gang form of LEAVE: LEAVE GANG. The room is told, the player leaves and is saved; the leader
/// may not leave. `None` for any other line.
pub fn leave_gang(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let plr = player(world, id)?.clone();
    if line.argc() != 2 || !eq_ignore_case(line.arg(1), "gang") {
        return None;
    }
    if plr.gang.is_empty() {
        tell_user(world, id, Msg::GangNotCurrently);
        return Some(Handled::Done);
    }
    match get_gang(world, &plr.gang) {
        Some(g) if !eq_ignore_case(&g.leader, &plr.firstname) => {
            tell_others(
                world,
                id,
                &Msg::GangLeftRoom {
                    who: plr.firstname.clone(),
                    gang: plr.gang.clone(),
                },
            );
            tell_user(
                world,
                id,
                Msg::GangYouLeft {
                    gang: plr.gang.clone(),
                },
            );
            remove_from_gang(world, id, &plr.gang);
            crate::player::save_player(world, id);
        }
        // the leader, or (sic) a gang record that cannot be found
        _ => tell_user(world, id, Msg::GangLeaderCannotLeave),
    }
    Some(Handled::Done)
}

/// DISBAND, but for DISBAND PARTY, which is T-043's (`None`). DISBAND GANG asks the leader to
/// confirm (`SUBST_DISBAND`, answered in [`on_disband_answer`]); anything but one word after
/// DISBAND gets the syntax line.
pub fn disband(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let plr = player(world, id)?.clone();
    if line.argc() == 2 && eq_ignore_case(line.arg(1), "party") {
        return None;
    }
    if line.argc() != 2 || !eq_ignore_case(line.arg(1), "gang") {
        tell_user(world, id, Msg::GangDisbandSyntax);
        return Some(Handled::Done);
    }
    if plr.gang.is_empty() {
        tell_user(world, id, Msg::GangDisbandNoGang);
        return Some(Handled::Done);
    }
    let Some(g) = get_gang(world, &plr.gang).cloned() else {
        // an internal error ("cant find gang"): the player is told, the gang name is forgotten
        tracing::error!(?id, gang = %plr.gang, "cant find gang");
        tell_user(
            world,
            id,
            Msg::MudInternalError {
                msg: "cant find gang".into(),
            },
        );
        if let Some(p) = player_mut(world, id) {
            p.gang.clear();
        }
        return Some(Handled::Done);
    };
    if !eq_ignore_case(&plr.firstname, &g.leader) {
        tell_user(world, id, Msg::GangDisbandNotLeader);
        return Some(Handled::Done);
    }
    tell_user(world, id, Msg::GangDisbandConfirm { gang: g.name });
    if let Some(s) = world.session_mut(id) {
        s.state = SessionState::Disband;
    }
    Some(Handled::Wait)
}

/// The answer to the DISBAND question (`SUBST_DISBAND`): a one-word line starting with Y disbands
/// the gang (the gang is told, the leader removed, which disbands it); anything else keeps it.
/// Either way the player is back in the realm with a prompt.
pub fn on_disband_answer(world: &mut World, id: SessionId, line: &str) {
    let mut words = line.split_whitespace();
    let yes = matches!(
        (words.next(), words.next()),
        (Some(w), None) if w.starts_with(['y', 'Y'])
    );
    if yes {
        let gang = player(world, id).map(|p| p.gang.clone());
        if let Some(gang) = gang
            && let Some(g) = get_gang(world, &gang).cloned()
        {
            tell_gang(world, &gang, &Msg::GangDisbanded { gang: g.name });
            remove_from_gang(world, id, &gang);
            if let Some(s) = world.session_mut(id) {
                s.push_prompt();
            }
        }
    } else if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::GangNotDisbanded);
        s.push_prompt();
    }
    if let Some(s) = world.session_mut(id) {
        s.state = SessionState::Playing;
    }
}

/// What PROMOTE or DEMOTE found for the name (`find_any_action_target`).
enum RankTarget {
    /// The commander himself.
    Myself,
    /// A player in the game.
    Online(SessionId),
    /// Nobody in the game: the stored characters are asked.
    Offline,
    /// A monster's name: nothing is done here. (sic) In the original it can rank whoever is
    /// on the channel numbered as the monster.
    Monster,
}

/// The common start of PROMOTE and DEMOTE: one name after the word, from the gang's leader. `None`
/// when the command ends quietly.
fn rank_command(world: &World, id: SessionId, line: &CommandLine) -> Option<RankTarget> {
    let plr = player(world, id)?;
    if line.argc() != 2 || plr.gang.is_empty() {
        return None;
    }
    let g = get_gang(world, &plr.gang)?;
    if !eq_ignore_case(&g.leader, &plr.firstname) {
        return None;
    }
    Some(match find_any_action_target(world, line.arg(1)) {
        Target::User(t) if t == id => RankTarget::Myself,
        Target::User(t) => RankTarget::Online(t),
        Target::Monster => RankTarget::Monster,
        Target::None | Target::Multiple(_) => RankTarget::Offline,
    })
}

/// PROMOTE {name} makes a member a lieutenant. Only the leader; a member in the game is promoted at
/// once and told, one off line gets `PF_PROMOTED` and the notice at his next logon.
pub fn cmd_promote(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        tell_user(world, id, Msg::GangPromoteSyntax);
        return Handled::Done;
    }
    match rank_command(world, id, line) {
        None | Some(RankTarget::Monster) => Handled::Done,
        Some(RankTarget::Myself) => {
            tell_user(world, id, Msg::GangPromoteSelf);
            Handled::Done
        }
        Some(RankTarget::Online(t)) => {
            let Some(tplr) = player(world, t).cloned() else {
                return Handled::Done;
            };
            let leader = my_gang(world, id).map(|g| g.leader).unwrap_or_default();
            if !eq_ignore_case(&tplr.gang, &plr.gang) {
                tell_user(world, id, Msg::GangPromoteOutsider);
            } else if eq_ignore_case(&tplr.firstname, &leader) {
                tell_user(world, id, Msg::GangPromoteLeader);
            } else {
                if let Some(p) = player_mut(world, t) {
                    p.flags.insert(PlayerFlags::LIEUTENANT);
                }
                tell_user(world, t, Msg::GangPromotedYou);
                tell_user(
                    world,
                    id,
                    Msg::GangPromoteNotified {
                        who: tplr.firstname,
                    },
                );
            }
            Handled::Done
        }
        Some(RankTarget::Offline) => {
            ask(
                world,
                id,
                GangQuery::Offline {
                    name: line.arg(1).to_string(),
                    gang: plr.gang,
                    op: OfflineOp::Promote,
                },
            );
            Handled::Wait
        }
    }
}

/// DEMOTE {name} takes the lieutenant rank away, as PROMOTE gives it.
pub fn cmd_demote(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        tell_user(world, id, Msg::GangDemoteSyntax);
        return Handled::Done;
    }
    match rank_command(world, id, line) {
        None | Some(RankTarget::Monster) => Handled::Done,
        Some(RankTarget::Myself) => {
            tell_user(world, id, Msg::GangDemoteSelf);
            Handled::Done
        }
        Some(RankTarget::Online(t)) => {
            let Some(tplr) = player(world, t).cloned() else {
                return Handled::Done;
            };
            if !eq_ignore_case(&tplr.gang, &plr.gang) {
                tell_user(world, id, Msg::GangDemoteOutsider);
            } else {
                if let Some(p) = player_mut(world, t) {
                    p.flags.remove(PlayerFlags::LIEUTENANT);
                }
                tell_user(world, t, Msg::GangDemotedYou);
                tell_user(
                    world,
                    id,
                    Msg::GangDemoteNotified {
                        who: tplr.firstname,
                    },
                );
            }
            Handled::Done
        }
        Some(RankTarget::Offline) => {
            ask(
                world,
                id,
                GangQuery::Offline {
                    name: line.arg(1).to_string(),
                    gang: plr.gang,
                    op: OfflineOp::Demote,
                },
            );
            Handled::Wait
        }
    }
}

/// Doing anything ends resting and meditating.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// Using the controller gives the player away.
fn stop_hiding(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
}

/// Where a gang shop command stands: the shop of the player's room and its gang house.
enum ShopCheck {
    /// Not in a shop room.
    NotHere,
    /// A shop, but not a gang shop.
    NotGang,
    /// A gang shop: its number and the room's gang house number.
    Gang(ShopNo, i32),
}

fn gang_shop_here(world: &World, plr: &Character) -> ShopCheck {
    let Some(rm) = world.room(room_of(plr)) else {
        return ShopCheck::NotHere;
    };
    if rm.room_type != RoomType::Shop {
        return ShopCheck::NotHere;
    }
    match world.shops.get(&rm.shopnum) {
        Some(shp) if shp.shop_type == ShopType::GangShop => {
            ShopCheck::Gang(rm.shopnum, i32::from(rm.ganghouse.code()))
        }
        _ => ShopCheck::NotGang,
    }
}

/// The controller test of UNSTOCK and MARKUP: some carried slot's `ABL_GANGSHOPITEM` value (0 for
/// none) equals the room's gang house.
fn carries_controller(world: &World, plr: &Character, house: i32) -> bool {
    plr.items
        .iter()
        .take(NINVITEMS)
        .any(|&n| gang_shop_item_value(world, n) == house)
}

/// STOCK {item} [{price}] [{currency}] puts a carried item up for sale in a gang shop, for a player
/// carrying the gang house's controller. Limited, worn cursed (without a second copy), loyal and
/// no-drop items are refused. The item joins a slot that sells it with fewer than 20 copies, else
/// the first empty slot of the ten; the price (capped at 9999) and currency are the last words or
/// the item's own; the slots' `maxqty[]` take the stocker's user-id.
pub fn cmd_stock(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    stop_resting(world, id);
    if line.argc() < 2 {
        tell_user(world, id, Msg::GangStockSyntax);
        return Handled::Done;
    }
    let (shopnum, house) = match gang_shop_here(world, &plr) {
        ShopCheck::NotHere => {
            tell_user(world, id, Msg::GangStockNotHere);
            return Handled::Done;
        }
        ShopCheck::NotGang => {
            tell_user(world, id, Msg::GangShopNotGang);
            return Handled::Done;
        }
        ShopCheck::Gang(s, h) => (s, h),
    };
    // a slot carrying the controller of this house
    let haskey = plr.items.iter().take(NINVITEMS).any(|&n| {
        world
            .items
            .get(&n)
            .is_some_and(|i| item_has_ability(i, Ability::GangShopItem))
            && gang_shop_item_value(world, n) == house
    });
    if !haskey {
        tell_user(world, id, Msg::GangStockNoKey);
        return Handled::Done;
    }
    stop_hiding(world, id);
    let found = find_item_in_inventory(world, &plr, line.rest_from(1), Search::Any);
    // the match lookup ends: the candidate list is thrown away and the item found is stocked
    clear_output(world, id);
    let Some(itm) = found.item.and_then(|n| world.items.get(&n)).cloned() else {
        return Handled::Done;
    };
    if itm.limit != 0 {
        tell_user(world, id, Msg::GangStockLimited);
        return Handled::Done;
    }
    let cursed =
        item_has_ability(&itm, Ability::Cursed) || item_has_ability(&itm, Ability::MajorCurse);
    if cursed && user_is_wearing(&plr, itm.number) {
        let copies = plr
            .items
            .iter()
            .take(NINVITEMS)
            .filter(|&&n| n == itm.number)
            .count();
        if copies < 2 {
            tell_user(world, id, Msg::GangStockRefused);
            return Handled::Done;
        }
    }
    if item_has_ability(&itm, Ability::LoyalItem) || itm.nodrop != 0 {
        tell_user(world, id, Msg::GangStockRefused);
        return Handled::Done;
    }
    let debug = debug_on(world, id);
    let Some(shp) = world.shops.get(&shopnum).cloned() else {
        return Handled::Done;
    };
    // a slot already selling it with room, else the first empty one
    let mut slot = None;
    for i in 0..GANG_SLOTS {
        if debug {
            tell_user(
                world,
                id,
                Msg::GangStockDebugSlot {
                    slot: i as i32,
                    item: shp.stock[i].item.0 as u32,
                },
            );
        }
        if slot.is_none() && shp.stock[i].item == itm.number && shp.stock[i].qty < GANGMAXQTY {
            slot = Some((i, false));
        }
    }
    if slot.is_none() {
        slot = (0..GANG_SLOTS)
            .find(|&i| shp.stock[i].item == ItemNo(0))
            .map(|i| (i, true));
    }
    let Some((slot, fresh)) = slot else {
        tell_user(world, id, Msg::GangStockFull);
        return Handled::Done;
    };
    let (price, currency) = stock_price(world, line);
    let shp = world.shops.get_mut(&shopnum).expect("looked up above");
    let s = &mut shp.stock[slot];
    if fresh {
        s.item = itm.number;
        s.qty = 1;
    } else {
        s.qty += 1;
    }
    // the asking price, unsigned, goes in the slot's `regentime`
    s.regentime = price.map_or(itm.cost, |p| p as u16);
    s.regennum = currency.map_or(itm.currency.code() as i16, |c| c.code() as i16);
    set_gang_shop_owner(shp, &plr.userid);
    save_gang_shop(world, shopnum);
    tell_user(
        world,
        id,
        Msg::GangStockAdded {
            item: itm.name.clone(),
        },
    );
    remove_item_from_inventory(world, id, itm.number);
    tell_others(
        world,
        id,
        &Msg::GangStockSeen {
            who: plr.firstname.clone(),
            item: itm.name,
        },
    );
    Handled::Done
}

/// The asking price and currency typed after STOCK: when the last word names a currency (the first
/// word of its name) the price is the word before it, else the last word. A price reading 0 counts
/// only as `free` or `0`; it is capped at 9999. `None` for either means the item's own.
fn stock_price(world: &World, line: &CommandLine) -> (Option<u32>, Option<Currency>) {
    let n = line.argc();
    let last = line.arg(n - 1);
    // the fifth currency name is the sysop's USRCURNM at run time
    let currency = CURRENCY_NAMES
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            if i == 4 {
                world.config.usr_cur_name.as_str()
            } else {
                name
            }
        })
        .enumerate()
        .filter(|(_, name)| eq_ignore_case(last, name.split(' ').next().unwrap_or("")))
        .map(|(i, _)| Currency::from_code(i as u16))
        .next_back();
    let word = if currency.is_some() {
        line.arg(n - 2)
    } else {
        last
    };
    // the price is `leading_int32` cut to an unsigned 16 bits
    let price = u32::from(leading_int32(word) as u16);
    let price = if price == 0 {
        (eq_ignore_case(word, "free") || eq_ignore_case(word, "0")).then_some(0)
    } else {
        Some(price)
    };
    (price.map(|p| p.min(GANGMAXPRICE)), currency)
}

/// UNSTOCK ALL drops every stocked copy on the floor and clears the slots; UNSTOCK {item} hands one
/// copy back from the slot with the fewest (the last of equals), clearing a slot that runs out.
pub fn cmd_unstock(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    stop_resting(world, id);
    if line.argc() < 2 {
        tell_user(world, id, Msg::GangUnstockSyntax);
        return Handled::Done;
    }
    let (shopnum, house) = match gang_shop_here(world, &plr) {
        ShopCheck::NotHere => {
            tell_user(world, id, Msg::GangUnstockNotHere);
            return Handled::Done;
        }
        ShopCheck::NotGang => {
            tell_user(world, id, Msg::GangShopNotGang);
            return Handled::Done;
        }
        ShopCheck::Gang(s, h) => (s, h),
    };
    if !carries_controller(world, &plr, house) {
        tell_user(world, id, Msg::GangUnstockNoKey);
        return Handled::Done;
    }
    stop_hiding(world, id);
    let room = room_of(&plr);
    if starts_with_ignore_case(line.arg(1), "all") {
        let mut emptied = false;
        for i in 0..GANG_SLOTS {
            let (item, qty) = {
                let s = &world.shops[&shopnum].stock[i];
                (s.item, s.qty)
            };
            // (sic) a full floor loses the copy
            for _ in 0..qty {
                add_item_to_room(world, room, item, false, 1);
                emptied = true;
            }
            let s = &mut world.shops.get_mut(&shopnum).expect("gang shop").stock[i];
            s.qty = 0;
            s.item = ItemNo(0);
            s.regentime = 0;
        }
        if emptied {
            tell_user(world, id, Msg::GangUnstockAll);
            tell_others(
                world,
                id,
                &Msg::GangUnstockAllSeen {
                    who: plr.firstname.clone(),
                },
            );
        } else {
            tell_user(world, id, Msg::GangUnstockEmpty);
        }
        save_gang_shop(world, shopnum);
        return Handled::Done;
    }
    let debug = debug_on(world, id);
    let shp = world.shops[&shopnum].clone();
    let found = get_shop_item_from_name(world, &shp, line.rest_from(1));
    if found.cleared {
        clear_output(world, id);
    }
    if found.item.is_none() && found.count > 0 {
        // several candidates: the list is sent
        show_options(world, id, &found.options);
    }
    let mut removed = false;
    if let Some(itm) = found.item.and_then(|n| world.items.get(&n)).cloned() {
        let mut pick = None;
        let mut lowqty = GANGMAXQTY;
        for i in 0..GANG_SLOTS {
            let s = &shp.stock[i];
            if s.item == itm.number && s.qty > 0 {
                if s.qty <= lowqty {
                    pick = Some(i);
                    lowqty = s.qty;
                }
            } else if debug && s.item == itm.number {
                tell_user(world, id, Msg::GangUnstockDebugFound);
                if s.qty == 0 {
                    tell_user(world, id, Msg::GangUnstockDebugZero);
                }
            }
        }
        if let Some(slot) = pick
            && add_item_to_inventory(world, id, itm.number, -2)
        {
            removed = true;
            tell_others(
                world,
                id,
                &Msg::GangUnstockSeen {
                    who: plr.firstname.clone(),
                    item: itm.name.clone(),
                },
            );
            tell_user(world, id, Msg::GangUnstockDone { item: itm.name });
            let s = &mut world.shops.get_mut(&shopnum).expect("gang shop").stock[slot];
            s.qty -= 1;
            if s.qty == 0 {
                s.item = ItemNo(0);
                s.regentime = 0;
                if debug {
                    tell_user(world, id, Msg::GangUnstockDebugRemoving);
                }
            }
            save_gang_shop(world, shopnum);
        }
    }
    if !removed {
        tell_user(world, id, Msg::GangUnstockNotStocked);
    }
    Handled::Done
}

/// MARKUP {percent} sets a gang shop's markup, kept within 0 and 1000; a word that is no number
/// counts as 0.
pub fn cmd_markup(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    stop_resting(world, id);
    if line.argc() < 2 {
        tell_user(world, id, Msg::GangMarkupSyntax);
        return Handled::Done;
    }
    let (shopnum, house) = match gang_shop_here(world, &plr) {
        ShopCheck::NotHere => {
            tell_user(world, id, Msg::GangMarkupNotHere);
            return Handled::Done;
        }
        ShopCheck::NotGang => {
            tell_user(world, id, Msg::GangShopNotGang);
            return Handled::Done;
        }
        ShopCheck::Gang(s, h) => (s, h),
    };
    if !carries_controller(world, &plr, house) {
        tell_user(world, id, Msg::GangMarkupNoKey);
        return Handled::Done;
    }
    stop_hiding(world, id);
    // `leading_int32` cut to a 16-bit int
    let pct = i32::from(leading_int32(line.arg(1)) as i16).clamp(0, MAXMARKUP) as u16;
    if let Some(shp) = world.shops.get_mut(&shopnum) {
        shp.markup = pct;
    }
    save_gang_shop(world, shopnum);
    tell_user(world, id, Msg::GangMarkupSet { pct });
    Handled::Done
}
