//! The gang member lists of BROADGANG (T-044): [`display_online_gang_members`],
//! [`display_gang_members`] and [`show_gang_members`].
//!
//! The full list walks every character record (WCCUSERS, key 2, lowest experience first),
//! one record per poll of the channel. Here [`display_gang_members`] shows the heading and
//! the leader, sets the paging state (`SUBST_LISTING`, `USF_GANGLIST`) and asks the store
//! for the gang's characters ([`crate::gangs::GangQuery::Members`]);
//! [`show_gang_members`] shows the rest when the answer comes back.
//!
//! (sic) X during the list clears only `USF_TOPTEN`, so it returns the player to the realm
//! but the gang list still runs to its end.

use silvermere_text::Msg;
use silvermere_types::enums::SessionState;
use silvermere_types::flags::{GangFlags, PlayerFlags, SessionFlags};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;

use crate::commands::eq_ignore_case;
use crate::comms::{UserMatch, get_user_number};
use crate::gangs::{GangQuery, ask, get_gang};
use crate::world::World;

/// `%-29.29s`.
fn col29(s: &str) -> String {
    format!("{:<29.29}", s)
}

fn full_name(c: &Character) -> String {
    format!("{} {}", c.firstname, c.lastname)
}

/// `get_user_number(name, 1)` as a player record.
fn online(world: &World, name: &str) -> Option<Character> {
    match get_user_number(world, name, true) {
        UserMatch::One(s) => world.session(s).and_then(|s| s.player()).cloned(),
        _ => None,
    }
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// The leader (`[Leader]`, or `[Leader - Offline]`), the lieutenants in the game, the other
/// members in the game, then the prompt.
pub fn display_online_gang_members(world: &mut World, id: SessionId, gangname: &str) {
    let Some(g) = get_gang(world, gangname).cloned() else {
        return;
    };
    push(
        world,
        id,
        Msg::GangMembersOnlineHeader {
            gang: g.name.clone(),
        },
    );
    let leader = match online(world, &g.leader) {
        Some(p) => Msg::GangLeaderRow {
            name: col29(&full_name(&p)),
        },
        None => Msg::GangLeaderOfflineRow {
            name: col29(&g.leader),
        },
    };
    push(world, id, leader);
    let members: Vec<Character> = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .filter_map(|i| world.session(i).and_then(|s| s.player()).cloned())
        .filter(|p| eq_ignore_case(&p.gang, &g.name) && !eq_ignore_case(&p.firstname, &g.leader))
        .collect();
    for p in members
        .iter()
        .filter(|p| p.flags.contains(PlayerFlags::LIEUTENANT))
    {
        push(
            world,
            id,
            Msg::GangLieutenantRow {
                name: col29(&full_name(p)),
            },
        );
    }
    for p in members
        .iter()
        .filter(|p| !p.flags.contains(PlayerFlags::LIEUTENANT))
    {
        push(
            world,
            id,
            Msg::GangMemberRow {
                name: col29(&full_name(p)),
            },
        );
    }
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
}

/// The heading, the disbanded note, the leader, then the paging state; the members follow
/// when the store answers.
pub fn display_gang_members(world: &mut World, id: SessionId, gangname: &str) {
    let Some(g) = get_gang(world, gangname).cloned() else {
        return;
    };
    push(
        world,
        id,
        Msg::GangMembersHeader {
            gang: g.name.clone(),
            count: i32::from(g.members),
        },
    );
    if g.flags.contains(GangFlags::DISBANDED) {
        push(world, id, Msg::GangMembersDisbanded);
    }
    let leader = match online(world, &g.leader) {
        Some(p) => Msg::GangLeaderOnlineRow {
            name: col29(&full_name(&p)),
        },
        None => Msg::GangLeaderListRow {
            name: col29(&g.leader),
        },
    };
    push(world, id, leader);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.ust.listidx = 1;
    s.ust.listmax = g.members;
    s.ust.flags.insert(SessionFlags::GANGLIST);
    s.ust.listkey = 0;
    s.ust.rptname = g.name.clone();
    s.state = SessionState::Listing;
    ask(world, id, GangQuery::Members { gang: g.name });
}

/// The rest of the gang list over `rows`, the gang's stored characters lowest experience
/// first: each member but the leader counts and is shown (`- Online ` when in the game,
/// `[Lieutenant]`); after each record the list ends once as many have been counted as the
/// gang has members, or at the last record. Then the paging state is cleared, the player is
/// back in the realm and gets the prompt.
///
/// Every line comes from the stored record; only the `- Online` test looks at who is in the game
/// (an exact name match). A member whose rank or gang changed in the game shows the stored state
/// until his record is saved: neither `remove_from_gang` nor PROMOTE/DEMOTE saves the target.
pub fn show_gang_members(world: &mut World, id: SessionId, rows: &[Character]) {
    let paging = world
        .session(id)
        .is_some_and(|s| s.ust.flags.contains(SessionFlags::GANGLIST));
    if !paging {
        return;
    }
    let (gangname, listmax) = {
        let s = world.session(id).expect("checked above");
        (s.ust.rptname.clone(), s.ust.listmax)
    };
    let mut idx = world.session(id).map_or(1, |s| s.ust.listidx);
    for stored in rows {
        let live = online(world, &stored.firstname);
        let c = stored;
        if eq_ignore_case(&c.gang, &gangname) {
            // (sic) a gang gone in the meantime stops the walk where it is
            let Some(leader) = get_gang(world, &gangname).map(|g| g.leader.clone()) else {
                return;
            };
            if !eq_ignore_case(&c.firstname, &leader) {
                idx = idx.wrapping_add(1);
                push(
                    world,
                    id,
                    Msg::GangMemberListRow {
                        name: col29(&full_name(c)),
                        online: if live.is_some() { "- Online " } else { "" }.into(),
                        rank: if c.flags.contains(PlayerFlags::LIEUTENANT) {
                            "[Lieutenant]"
                        } else {
                            ""
                        }
                        .into(),
                    },
                );
            }
        }
        if idx >= listmax {
            break;
        }
    }
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.ust.listidx = idx;
    s.ust.flags.remove(SessionFlags::GANGLIST);
    s.state = SessionState::Playing;
    s.push_prompt();
}
