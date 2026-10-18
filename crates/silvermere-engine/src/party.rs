//! Parties: the session-only group state and the routines that change it.
//!
//! `leader`, `group[NGROUP]` and `dragging` are [`SessionId`]s in [`Party`], held by the
//! session and never saved. The battle rank stays in the
//! character (`Character::rank`, reset on load) because combat reads it there.
//!
//! One list does the work of members and invitations: a leader's
//! `group` holds everyone he invited, and an invitee becomes a follower when he follows
//! (his `leader` is set and every member is linked to every other, [`add_to_group`]). A
//! slot whose player does not follow the same leader is an invitation ([`same_party`]).

use silvermere_text::Msg;
use silvermere_types::enums::Currency;
use silvermere_types::flags::{PlayerSettings, PlayerStatus};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;
use silvermere_types::sizes::NGROUP;

use crate::commands::inventory::{coin_mut, proper_currency_name};
use crate::cycles::MoveKind;
use crate::movement::exits::prompt;
use crate::output::clear_output;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// `RANK_FRONT`: the front battle rank.
pub const RANK_FRONT: u8 = 1;
/// `RANK_BACK`: the back battle rank.
pub const RANK_BACK: u8 = 2;
pub use crate::player::RANK_MIDDLE;

/// The session-only party state (`leader`, `group[NGROUP]`, `dragging`), by session id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Party {
    /// Who this player follows (`leader`, -1 none).
    pub leader: Option<SessionId>,
    /// The party list (`group[NGROUP]`, -1 a free slot): for a leader the players he
    /// invited, following or not; for a follower his leader and the other members.
    pub group: [Option<SessionId>; NGROUP],
    /// Who this player drags (`dragging`).
    pub dragging: Option<SessionId>,
    /// Whether `dragging` was started from the character's record (`load_player`): only
    /// then does a save write it back. A record saved before that (character creation)
    /// keeps the value it has.
    pub loaded: bool,
}

impl Party {
    /// The party state a character enters the game with. `load_player` reads the whole
    /// record, `dragging` included (`save_player` writes it back), and sets `leader`,
    /// `group[]`, `rank` and `movedir` but not `dragging`. A new character's record is
    /// zeroed by `clear_player`, so it "drags" channel 0 until something sets -1: its first
    /// marched step is refused with "You may not drag someone while in a party!", which
    /// also resets it, and the -1 is saved from then on.
    pub fn from_record(dragging: i16) -> Self {
        Self {
            dragging: u16::try_from(dragging).ok().map(SessionId),
            loaded: true,
            ..Self::default()
        }
    }

    /// `dragging` as `save_player` writes it: -1 for nobody.
    pub fn record_dragging(&self) -> i16 {
        self.dragging
            .map_or(-1, |s| i16::try_from(s.0).unwrap_or(i16::MAX))
    }

    /// The filled slots, in slot order.
    pub fn members(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.group.iter().flatten().copied()
    }
}

/// The rank a FRONTRANK, BACKRANK or MIDRANK asks for: the argument of [`change_user_rank`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rank {
    /// 0, any value but 1 and 2.
    Middle,
    /// 1.
    Front,
    /// 2.
    Back,
}

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

fn party(world: &World, id: SessionId) -> Party {
    world
        .session(id)
        .map(|s| s.party.clone())
        .unwrap_or_default()
}

fn party_mut(world: &mut World, id: SessionId) -> Option<&mut Party> {
    world.session_mut(id).map(|s| &mut s.party)
}

fn leader_of(world: &World, id: SessionId) -> Option<SessionId> {
    world.session(id).and_then(|s| s.party.leader)
}

/// The player's name on a channel: the first name, empty for an empty channel.
fn name_of(world: &World, id: SessionId) -> String {
    player(world, id).map_or_else(String::new, |p| p.firstname.clone())
}

/// Whether the session has a character loaded and is in the game.
fn here(world: &World, id: SessionId) -> bool {
    player(world, id).is_some() && world.is_in_game(id)
}

/// The test the rank change, PARTY and SHARE make: `member`, listed by `unum`, really
/// travels with him: he follows
/// the same leader, follows `unum`, or is `unum`'s leader; and at least one of the two
/// follows someone. Otherwise the slot is only an invitation.
pub fn same_party(world: &World, unum: SessionId, member: SessionId) -> bool {
    let pl = leader_of(world, unum);
    let ml = leader_of(world, member);
    (ml == pl || ml == Some(unum) || pl == Some(member)) && (pl.is_some() || ml.is_some())
}

/// The player back to the middle rank,
/// crossed off the list of everyone he lists (a member left listing nobody goes back to
/// the middle too), his own list cleared, and crossed off every other list in the game.
pub fn remove_from_group(world: &mut World, unum: SessionId) {
    if player(world, unum).is_none() {
        return;
    }
    if let Some(p) = player_mut(world, unum) {
        p.rank = RANK_MIDDLE;
    }
    for i in 0..NGROUP {
        let Some(m) = party(world, unum).group[i] else {
            continue;
        };
        if player(world, m).is_some() {
            let mut others = false;
            if let Some(mp) = party_mut(world, m) {
                for slot in &mut mp.group {
                    if *slot == Some(unum) {
                        *slot = None;
                    } else if slot.is_some() {
                        others = true;
                    }
                }
            }
            if !others && let Some(mplr) = player_mut(world, m) {
                mplr.rank = RANK_MIDDLE;
            }
        }
        if let Some(p) = party_mut(world, unum) {
            p.group[i] = None;
        }
    }
    for i in world.session_ids() {
        if world.is_in_game(i)
            && player(world, i).is_some()
            && let Some(p) = party_mut(world, i)
        {
            for slot in &mut p.group {
                if *slot == Some(unum) {
                    *slot = None;
                }
            }
        }
    }
}

/// Adds `member` to `unum`'s party. Without
/// `link_all` a member not yet listed goes into `unum`'s first free slot (`true` when he
/// did). With `link_all` and `member` already listed (he was invited), every member of
/// `unum`'s list gets `unum` and every other member into his own list, and the answer
/// is `true`; any other case answers `false`.
pub fn add_to_group(world: &mut World, unum: SessionId, member: SessionId, link_all: bool) -> bool {
    if member == unum || player(world, unum).is_none() {
        return false;
    }
    let group = party(world, unum).group;
    let already = group.contains(&Some(member));
    if !already && !link_all {
        if let Some(p) = party_mut(world, unum)
            && let Some(slot) = p.group.iter_mut().find(|s| s.is_none())
        {
            *slot = Some(member);
            return true;
        }
        return false;
    }
    if already && link_all {
        for m in group.iter().flatten() {
            add_to_group(world, *m, unum, false);
            for o in group.iter().flatten() {
                add_to_group(world, *m, *o, false);
            }
        }
        return true;
    }
    false
}

/// Stops one follower following:
/// when `follower` follows `leader`, he leaves the party (`remove_from_group`) and is
/// told, with his prompt. The leader is never told. Nothing when he follows someone else
/// (or, here, when there is no leader: every caller passes the follower's own leader).
pub fn stop_following(world: &mut World, leader: Option<SessionId>, follower: SessionId) {
    if leader.is_none() || player(world, follower).is_none() || leader_of(world, follower) != leader
    {
        return;
    }
    remove_from_group(world, follower);
    if let Some(p) = party_mut(world, follower) {
        p.leader = None;
    }
    let who = leader.map(|l| name_of(world, l)).unwrap_or_default();
    tell_user(world, follower, Msg::NoLongerFollowing { who });
    prompt(world, follower);
}

/// [`stop_following`] for every follower: everyone in the game who
/// follows `leader` leaves the party and is told (the leader himself, were he listed as
/// his own follower, silently), in channel order; then the leader leaves his party.
pub fn stop_following_all(world: &mut World, leader: SessionId) {
    for i in world.session_ids() {
        if !world.is_in_game(i) || leader_of(world, i) != Some(leader) {
            continue;
        }
        if let Some(p) = party_mut(world, i) {
            p.leader = None;
        }
        remove_from_group(world, i);
        if i != leader {
            let who = name_of(world, leader);
            tell_user(world, i, Msg::NoLongerFollowing { who });
            prompt(world, i);
        }
    }
    remove_from_group(world, leader);
}

/// A downed player who leaves
/// the game or dies is let go by whoever drags him.
pub fn stop_dragging_user(world: &mut World, unum: SessionId) {
    if !player(world, unum).is_some_and(|p| p.hp <= 0) {
        return;
    }
    for i in world.session_ids() {
        if !world.is_in_game(i) {
            continue;
        }
        let drags = world.session(i).is_some_and(|s| {
            s.party.dragging == Some(unum)
                && s.player()
                    .is_some_and(|p| p.status.contains(PlayerStatus::DRAGGING))
        });
        if drags && let Some(s) = world.session_mut(i) {
            s.party.dragging = None;
            if let Some(p) = s.character.as_mut() {
                p.status.remove(PlayerStatus::DRAGGING);
            }
        }
    }
}

/// FRONTRANK, BACKRANK,
/// MIDRANK. The members no longer in the game are crossed off and the others counted by
/// rank; alone, the player is not in a party. The front is always open; the back is
/// refused to a leader, to the last of the front, and when the front (the others in it)
/// would have fewer than the back; the middle to the last of the front or back and to a
/// leader. On a move the "just moved" line goes to the members of the same party, then
/// the player gets his own line.
///
/// Each member is sent the line with [`tell_user`], which leaves the text in the buffer, so
/// every member of the same party reads it;
/// when nobody is told, the player gets it before his own line.
pub fn change_user_rank(world: &mut World, id: SessionId, rank: Rank) {
    let Some(plr) = player(world, id) else {
        return;
    };
    let name = plr.firstname.clone();
    let myrank = plr.rank;
    let mut cnt = 1;
    let (mut front, mut back) = (0, 0);
    for i in 0..NGROUP {
        let Some(m) = party(world, id).group[i] else {
            continue;
        };
        if here(world, m) {
            cnt += 1;
            match player(world, m).map_or(RANK_MIDDLE, |p| p.rank) {
                RANK_FRONT => front += 1,
                RANK_BACK => back += 1,
                _ => {}
            }
        } else if let Some(p) = party_mut(world, id) {
            p.group[i] = None;
        }
    }
    if cnt < 2 {
        tell_user(world, id, Msg::RankNotInParty);
        if let Some(p) = player_mut(world, id) {
            p.rank = RANK_MIDDLE;
        }
        return;
    }
    let leads = leader_of(world, id).is_none();
    let (newrank, other, mine) = match rank {
        Rank::Front => (
            RANK_FRONT,
            Msg::RankFrontOther { who: name },
            Msg::RankFrontSelf,
        ),
        Rank::Back => {
            let refusal = if leads {
                Some(Msg::RankBackLeader)
            } else if front == 0 && myrank == RANK_FRONT {
                Some(Msg::RankFrontOnly)
            } else if front < back {
                Some(Msg::RankNotEnoughFront)
            } else {
                None
            };
            if let Some(msg) = refusal {
                tell_user(world, id, msg);
                return;
            }
            (
                RANK_BACK,
                Msg::RankBackOther { who: name },
                Msg::RankBackSelf,
            )
        }
        Rank::Middle => {
            let refusal = if front == 0 && myrank == RANK_FRONT {
                Some(Msg::RankFrontOnly)
            } else if back == 0 && myrank == RANK_BACK {
                Some(Msg::RankBackOnly)
            } else if leads {
                Some(Msg::RankMidLeader)
            } else {
                None
            };
            if let Some(msg) = refusal {
                tell_user(world, id, msg);
                return;
            }
            (
                RANK_MIDDLE,
                Msg::RankMidOther { who: name },
                Msg::RankMidSelf,
            )
        }
    };
    if let Some(p) = player_mut(world, id) {
        p.rank = newrank;
    }
    let told: Vec<SessionId> = party(world, id)
        .members()
        .filter(|&m| player(world, m).is_none() || same_party(world, id, m))
        .collect();
    if told.is_empty() {
        tell_user(world, id, other);
    } else {
        // Each send only rewinds the output buffer, so the text stays: with nothing
        // written between the calls each member gets the same bytes, the dispatcher's
        // ANSI_NORMAL included, and the player's own line is written over them
        let pending = clear_output(world, id);
        for m in told {
            if let Some(s) = world.session_mut(m) {
                if pending {
                    s.output.push(Msg::CmdNormal);
                }
                s.output.push(other.clone());
            }
        }
    }
    tell_user(world, id, mine);
}

/// The followers on lower session ids walk after their leader once his queued move
/// worked (move type 3): their own fast update has already run
/// this cycle; those on higher ids follow in theirs (`cycles::fast`). One who is down or
/// cannot make the move leaves the party line (`leader = -1` only, sic: he stays in the
/// lists, and the leader is not told).
pub fn follow_step(world: &mut World, leader: SessionId, dir: i16) {
    let Ok(dir) = u8::try_from(dir) else {
        return;
    };
    for i in world.session_ids() {
        if i >= leader {
            break;
        }
        if !world.is_in_game(i) || leader_of(world, i) != Some(leader) {
            continue;
        }
        let down = player(world, i).is_none_or(|p| p.hp <= 0);
        if (down || !(world.cycles.hooks.move_user)(world, i, dir, MoveKind::Follow))
            && let Some(p) = party_mut(world, i)
        {
            p.leader = None;
        }
    }
}

/// Whether `vid` gets a share from `unum`: in the room
/// or in the same party, visible to the sharer (hidden only with see-hidden), and taking
/// things (`PS_NORECEIVE` off).
fn takes_share(world: &World, unum: SessionId, vid: SessionId, in_room: bool) -> bool {
    let (Some(plr), Some(v)) = (player(world, unum), player(world, vid)) else {
        return false;
    };
    if in_room && (v.room != plr.room || v.map != plr.map) {
        return false;
    }
    if !in_room && !same_party(world, unum, vid) {
        return false;
    }
    (v.hidden == 0 || plr.status.contains(PlayerStatus::SEEHIDDEN))
        && !v.settings.contains(PlayerSettings::NORECEIVE)
}

/// The players `unum` shares with, in the original's order: for the room every other
/// player in the game there in channel order; for the party the list in slot order (a
/// slot whose player has left the game is freed on the way).
fn sharers(world: &mut World, unum: SessionId, in_room: bool) -> Vec<SessionId> {
    let mut out = Vec::new();
    if in_room {
        for i in world.session_ids() {
            if i != unum && world.is_in_game(i) && takes_share(world, unum, i, true) {
                out.push(i);
            }
        }
    } else {
        for i in 0..NGROUP {
            let Some(m) = party(world, unum).group[i] else {
                continue;
            };
            if m == unum {
                continue;
            }
            if !here(world, m) {
                if let Some(p) = party_mut(world, unum) {
                    p.group[i] = None;
                }
            } else if takes_share(world, unum, m, false) {
                out.push(m);
            }
        }
    }
    out
}

/// SHARE: the pot
/// (cut to what the player carries) is split evenly between him and everyone in the room
/// or in his party who takes a share; each receiver gets `amount / count` with his
/// prompt, the sharer hears each gift, the sharer's room sees the coins change hands,
/// and the sharer keeps his share and the remainder. Every record touched is saved.
pub fn share_currency(
    world: &mut World,
    unum: SessionId,
    amount: i32,
    cur: Currency,
    in_room: bool,
) {
    let Some(plr) = player_mut(world, unum) else {
        return;
    };
    let purse = coin_mut(&mut plr.coins, cur);
    let amount = (amount.max(0) as u32).min(*purse);
    *purse -= amount;
    if amount == 0 {
        tell_user(world, unum, Msg::ShareNotEnough);
        return;
    }
    let takers = sharers(world, unum, in_room);
    let cnt = takers.len() as u32 + 1;
    let share = amount / cnt;
    let left = amount - share * cnt;
    if let Some(p) = player_mut(world, unum) {
        *coin_mut(&mut p.coins, cur) += left + share;
    }
    let (me, room) = match player(world, unum) {
        Some(p) => (p.firstname.clone(), crate::combat::room_of(p)),
        None => return,
    };
    let coin = proper_currency_name(world, i64::from(share), cur);
    for v in takers {
        let vname = name_of(world, v);
        if let Some(p) = player_mut(world, v) {
            *coin_mut(&mut p.coins, cur) += share;
        }
        tell_user(
            world,
            unum,
            Msg::GaveCoins {
                who: vname.clone(),
                amount: share.to_string(),
                coin: coin.clone(),
            },
        );
        tell_user(
            world,
            v,
            Msg::GotCoins {
                who: me.clone(),
                amount: share.to_string(),
                coin: coin.clone(),
            },
        );
        prompt(world, v);
        tell_room(
            world,
            room,
            [Some(unum), Some(v)],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::GaveSomeCoins {
                who: me.clone(),
                whom: vname,
            },
        );
        crate::player::save_player(world, v);
    }
    crate::player::save_player(world, unum);
}
