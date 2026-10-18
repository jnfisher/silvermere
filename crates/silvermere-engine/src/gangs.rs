//! Gangs: the gang records, the pending invitations, membership, gang
//! experience and the gang shop takings.
//!
//! Membership is only the gang's name in the character (`player.gang`); the gang record
//! (kept in [`World::gangs`] by upper-case key) counts the members. The
//! invitations are the engine's own list ([`World::gang_invites`]): they are never
//! stored, so a restart loses them, and they go when the invited player leaves the game
//! ([`clear_gang_invitations`] with no gang, and `cleanup_when_user_leaves`, at every
//! logoff and hang-up).
//!
//! Every change to a gang record is queued as [`PersistRequest::Gang`].
//!
//! The member list, UNINVITE MEMBER, PROMOTE and DEMOTE of a member off line need
//! character records that are not in the game. The engine never touches the database, so
//! those ask with [`EngineOutput::GangQuery`] and carry on in [`on_query_answer`] when the
//! store answers.
//!
//! The gang commands are `commands::gang`, the member lists `display::gang`, the disband
//! confirmation `state::listing`.

use silvermere_rules::abilities::item_ability_value;
use silvermere_text::Msg;
use silvermere_types::enums::DebugLevel;
use silvermere_types::flags::{GangFlags, PlayerFlags};
use silvermere_types::ids::{AccountId, ActiveMonsterId, SessionId, ShopNo};
use silvermere_types::records::{Character, Gang, Shop};
use silvermere_types::sizes::NSHOPITEMS;

use crate::commands::eq_ignore_case;
use crate::comms::{UserMatch, get_user_number};
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::items::inventory::matches_any_word;
use crate::tell::{tell_gang, tell_user};
use crate::world::{GangInvite, World};

/// `GANGNAMELEN`: the longest gang name.
pub const GANGNAMELEN: usize = 19;
/// `GANG_MINEXP`: experience needed to found a gang.
pub const GANG_MINEXP: u32 = 100_000;
/// `GANGBANK`: the bank (shop number) gang shop takings are paid into.
pub const GANGBANK: ShopNo = ShopNo(8);

/// The gang record's key: the name in upper case (`gang.key`, WCCGANGS key 0).
pub fn gang_key(name: &str) -> String {
    name.to_ascii_uppercase()
}

/// The gang whose key is `name` in upper case.
pub fn get_gang<'a>(world: &'a World, name: &str) -> Option<&'a Gang> {
    world.gangs.get(&gang_key(name))
}

/// [`get_gang`], mutable.
pub fn get_gang_mut<'a>(world: &'a mut World, name: &str) -> Option<&'a mut Gang> {
    world.gangs.get_mut(&gang_key(name))
}

/// Marks the gang changed: queues the gang record named `name` for the store.
pub fn save_gang(world: &mut World, name: &str) {
    if let Some(g) = get_gang(world, name).cloned() {
        world
            .outbox
            .push(EngineOutput::Persist(PersistRequest::Gang(g)));
    }
}

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

/// Records an invitation for `unum` to join
/// `gangname`. `false` when the gang does not exist or the invitation is a duplicate.
///
/// (sic) the duplicate test starts at the newest invitation and walks on from there, so
/// only the newest one is compared; an older identical invitation is not noticed. At most
/// 19 characters of the name are kept.
pub fn invite_to_gang(world: &mut World, unum: SessionId, gangname: &str) -> bool {
    if get_gang(world, gangname).is_none() {
        return false;
    }
    if world
        .gang_invites
        .last()
        .is_some_and(|l| l.session == unum && eq_ignore_case(&l.gang, gangname))
    {
        return false;
    }
    let gang: String = gangname.chars().take(GANGNAMELEN).collect();
    world.gang_invites.push(GangInvite {
        session: unum,
        gang,
    });
    true
}

/// Drops `unum`'s pending
/// invitations, all of them (`None`) or only those to `gangname`.
pub fn clear_gang_invitations(world: &mut World, unum: SessionId, gangname: Option<&str>) {
    world
        .gang_invites
        .retain(|g| !(g.session == unum && gangname.is_none_or(|n| eq_ignore_case(n, &g.gang))));
}

/// Takes `unum`, who is in the game, out
/// of `gangname`. Its invitations to that gang are dropped in any case. A member loses
/// the gang name and the lieutenant rank and the gang a member; when the member was the
/// leader the gang is flagged disbanded and every other member in the game is taken out
/// too (members off line are cleaned at their next `load_player`).
/// The character is not saved here. `false` when `unum` is not a member of a gang that
/// exists.
pub fn remove_from_gang(world: &mut World, unum: SessionId, gangname: &str) -> bool {
    if player(world, unum).is_none() {
        return false;
    }
    clear_gang_invitations(world, unum, Some(gangname));
    let Some(plr) = player(world, unum) else {
        return false;
    };
    if !eq_ignore_case(&plr.gang, gangname) {
        return false;
    }
    let key = gang_key(&plr.gang);
    let firstname = plr.firstname.clone();
    let Some(g) = world.gangs.get(&key) else {
        return false;
    };
    let gname = g.name.clone();
    let leader = eq_ignore_case(&firstname, &g.leader);
    if let Some(plr) = player_mut(world, unum) {
        plr.gang.clear();
        plr.flags.remove(PlayerFlags::LIEUTENANT);
    }
    let mut members = 1i16;
    if leader {
        // Every other member in the game (the leader's name is already gone)
        for i in world.session_ids() {
            if !world.is_in_game(i) {
                continue;
            }
            if let Some(p) = player_mut(world, i)
                && !p.gang.is_empty()
                && eq_ignore_case(&p.gang, &gname)
            {
                p.gang.clear();
                p.flags.remove(PlayerFlags::LIEUTENANT);
                members += 1;
            }
        }
    }
    if let Some(g) = world.gangs.get_mut(&key) {
        g.members = g.members.wrapping_sub(members);
        if leader {
            g.flags.insert(GangFlags::DISBANDED);
        }
    }
    save_gang(world, &key);
    true
}

/// `unum` joins `gangname` if invited to it.
/// The joiner is told, then the gang (`tell_gang`, the joiner included). Invited or not,
/// the record is saved and every invitation `unum` still has is dropped.
pub fn join_gang(world: &mut World, unum: SessionId, gangname: &str) {
    let Some(gname) = get_gang(world, gangname).map(|g| g.name.clone()) else {
        tell_user(world, unum, Msg::GangJoinNoGang);
        return;
    };
    let Some(firstname) = player(world, unum).map(|p| p.firstname.clone()) else {
        tell_user(world, unum, Msg::GangJoinNoGang);
        return;
    };
    let invited = world
        .gang_invites
        .iter()
        .any(|inv| inv.session == unum && eq_ignore_case(gangname, &inv.gang));
    if invited {
        if let Some(g) = get_gang_mut(world, gangname) {
            g.members = g.members.wrapping_add(1);
        }
        save_gang(world, &gname);
        if let Some(plr) = player_mut(world, unum) {
            plr.gang = gname.clone();
        }
        tell_user(
            world,
            unum,
            Msg::GangJoined {
                gang: gname.clone(),
            },
        );
        tell_gang(world, &gname, &Msg::GangMemberJoined { who: firstname });
    } else {
        tell_user(world, unum, Msg::GangJoinNotInvited);
    }
    crate::player::save_player(world, unum);
    clear_gang_invitations(world, unum, None);
}

/// The gang part of `add_experience`: the gang's total grows by
/// the kill's experience. Past `u32::MAX` the total stays at the cap, `GF_OVERFLOW` is
/// set and the rest goes to `overexp`; `overexp` wrapping in turn counts in `overflows`.
/// Every sum wraps at 2^32.
pub fn add_kill_exp(g: &mut Gang, exp: u32) {
    if g.flags.contains(GangFlags::OVERFLOW) {
        let newtot = g.overexp.wrapping_add(exp);
        if g.overexp > newtot {
            let room = u32::MAX - g.overexp;
            g.overexp = exp.wrapping_sub(room);
            g.overflows = g.overflows.wrapping_add(1);
        } else {
            g.overexp = newtot;
        }
    } else {
        g.overexp = 0;
        g.overflows = 0;
        let newtot = g.exp.wrapping_add(exp);
        if g.exp > newtot {
            let room = u32::MAX - g.exp;
            g.exp = u32::MAX;
            g.overexp = exp.wrapping_sub(room);
            g.flags.insert(GangFlags::OVERFLOW);
        } else {
            g.exp = newtot;
        }
    }
}

/// The gang part of deleting a character off line, for the sysop's character deletion
/// (T-055): the gang loses a member and, if `c` led it, is flagged
/// disbanded.
pub fn on_character_deleted(world: &mut World, c: &Character) {
    if c.gang.is_empty() {
        return;
    }
    let Some(g) = get_gang_mut(world, &c.gang) else {
        return;
    };
    g.members = g.members.wrapping_sub(1);
    if eq_ignore_case(&c.firstname, &g.leader) {
        g.flags.insert(GangFlags::DISBANDED);
    }
    let key = g.key.clone();
    save_gang(world, &key);
}

/// The user-id a gang shop keeps in its slots' `maxqty[]`, read as a string: the
/// little-endian bytes of the twenty `u16`s up to the first NUL.
pub fn gang_shop_owner(shop: &Shop) -> String {
    let bytes: Vec<u8> = shop
        .stock
        .iter()
        .flat_map(|s| s.maxqty.to_le_bytes())
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The owner's user-id and its NUL copied over the slots' `maxqty[]`; the bytes after the
/// NUL are left as they were.
pub fn set_gang_shop_owner(shop: &mut Shop, userid: &str) {
    let mut bytes: Vec<u8> = userid.bytes().collect();
    bytes.push(0);
    bytes.truncate(NSHOPITEMS * 2);
    for (i, b) in bytes.into_iter().enumerate() {
        let slot = &mut shop.stock[i / 2].maxqty;
        let mut le = slot.to_le_bytes();
        le[i % 2] = b;
        *slot = u16::from_le_bytes(le);
    }
}

/// Queues the whole gang shop record (after STOCK, UNSTOCK, MARKUP or a
/// sale that cleared a slot).
pub fn save_gang_shop(world: &mut World, shop: ShopNo) {
    if let Some(shp) = world.shops.get(&shop).cloned() {
        world
            .outbox
            .push(EngineOutput::Persist(PersistRequest::GangShop(Box::new(
                shp,
            ))));
    }
}

/// The takings of a sale in
/// gang shop `shop` go to its owner's bank book at the gang bank. Nothing is paid without
/// an owner or money; without a book, or when the balance would wrap, the money is lost.
/// A sysop in debug mode is told. The buyer `id` is saved afterwards.
pub fn deposit_gangleaders_account(world: &mut World, id: SessionId, shop: ShopNo, amount: u32) {
    let Some(owner) = world.shops.get(&shop).map(gang_shop_owner) else {
        return;
    };
    // the owner name is always present: an empty owner is looked up like any other
    if amount == 0 {
        return;
    }
    let debug = world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off);
    let book = world
        .bank_books
        .iter()
        .filter(|((_, bank), b)| *bank == GANGBANK && eq_ignore_case(&b.userid, &owner))
        .map(|((account, _), _)| *account)
        .min();
    match book {
        Some(account) => {
            let b = world
                .bank_books
                .get_mut(&(account, GANGBANK))
                .expect("found above");
            if b.balance.wrapping_add(amount) > b.balance {
                b.balance += amount;
                let book = b.clone();
                world
                    .outbox
                    .push(EngineOutput::Persist(PersistRequest::BankBook {
                        account,
                        book,
                    }));
                if debug {
                    tell_user(
                        world,
                        id,
                        Msg::GangShopDepositDebug {
                            leader: owner,
                            amount: amount.to_string(),
                        },
                    );
                }
            } else if debug {
                let balance = b.balance as u16;
                tell_user(
                    world,
                    id,
                    Msg::GangShopDepositFullDebug {
                        leader: owner,
                        balance,
                    },
                );
            }
        }
        None if debug => {
            tell_user(world, id, Msg::GangShopDepositNobookDebug { leader: owner });
        }
        None => {}
    }
    crate::player::save_player(world, id);
}

/// What a gang command asks the store about characters that are not in the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GangQuery {
    /// The stored characters whose gang is `gang` (any case), lowest experience first:
    /// the member list's WCCUSERS walk on key 2.
    Members { gang: String },
    /// The stored character whose first name is `name` (any case): the key 1 lookups of
    /// removing an offline member and of PROMOTE/DEMOTE. `gang` is the commander's gang.
    Offline {
        name: String,
        gang: String,
        op: OfflineOp,
    },
}

/// What is done to a member found off line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineOp {
    /// UNINVITE MEMBER of a member off line.
    Uninvite,
    /// PROMOTE: `PF_PROMOTED`, shown at the next logon.
    Promote,
    /// DEMOTE: `PF_DEMOTED`, shown at the next logon.
    Demote,
}

/// The store's answer to a [`GangQuery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GangAnswer {
    /// For [`GangQuery::Members`], in experience order.
    Members(Vec<Character>),
    /// For [`GangQuery::Offline`]: the character and its account, if found.
    Offline(Option<(AccountId, Box<Character>)>),
}

/// Asks the store (after a flush, so the rows are current).
pub fn ask(world: &mut World, id: SessionId, query: GangQuery) {
    world.outbox.push(EngineOutput::GangQuery { id, query });
}

/// The store answered `query` for session `id`.
pub fn on_query_answer(world: &mut World, id: SessionId, query: GangQuery, answer: GangAnswer) {
    match (query, answer) {
        (GangQuery::Members { .. }, GangAnswer::Members(rows)) => {
            crate::display::gang::show_gang_members(world, id, &rows);
        }
        (GangQuery::Offline { name, gang, op }, GangAnswer::Offline(found)) => {
            offline_edit(world, id, &name, &gang, op, found);
        }
        _ => {}
    }
}

/// The part of UNINVITE MEMBER, PROMOTE and DEMOTE that works on a character record off
/// line. A character who entered the game while the store looked is changed in memory
/// instead, so the change is not lost.
fn offline_edit(
    world: &mut World,
    id: SessionId,
    name: &str,
    gang: &str,
    op: OfflineOp,
    found: Option<(AccountId, Box<Character>)>,
) {
    // T-088: the record queued for the account and not yet acknowledged is the one
    // on file
    let found = match found {
        Some((account, c)) => crate::player::queued_or_stored(world, account, Some(*c))
            .map(|c| (account, Box::new(c))),
        None => queued_by_first_name(world, name),
    };
    let found = found.filter(|(_, c)| eq_ignore_case(&c.firstname, name));
    let online =
        found
            .as_ref()
            .and_then(|(_, c)| match get_user_number(world, &c.firstname, true) {
                UserMatch::One(s) => Some(s),
                _ => None,
            });
    let mut record = match (&found, online) {
        (Some(_), Some(s)) => player(world, s).cloned().map(|c| (None, c)),
        (Some((account, c)), None) => Some((Some(*account), (**c).clone())),
        (None, _) => None,
    };
    let member = record
        .as_ref()
        .is_some_and(|(_, c)| eq_ignore_case(&c.gang, gang) && get_gang(world, &c.gang).is_some());
    // `name` is rewritten in place and then printed by `cmd_uninvite`
    let shown = typed_name_as_rewritten(name, found.as_ref().map(|(_, c)| c.firstname.as_str()));
    let msg = match op {
        OfflineOp::Uninvite => {
            if member && let Some((_, c)) = record.as_mut() {
                c.gang.clear();
                c.flags.remove(PlayerFlags::LIEUTENANT);
                if let Some(g) = get_gang_mut(world, gang) {
                    g.members = g.members.wrapping_sub(1);
                }
                save_gang(world, gang);
                Some(Msg::GangRemovedOffline { who: shown })
            } else {
                record = None;
                Some(Msg::GangNotHere { who: shown })
            }
        }
        // The Syntax line of an unknown name goes out with the prompt `execute_input`
        // sends after a handled command
        OfflineOp::Promote => match record.as_mut() {
            None => Some(Msg::GangPromoteSyntax),
            Some((_, c)) if eq_ignore_case(&c.gang, gang) => {
                if c.flags.contains(PlayerFlags::LIEUTENANT) {
                    let who = c.firstname.clone();
                    record = None;
                    Some(Msg::GangPromoteAlready { who })
                } else {
                    c.flags.insert(PlayerFlags::PROMOTED);
                    Some(Msg::GangPromoteLater {
                        who: c.firstname.clone(),
                    })
                }
            }
            Some((_, c)) => {
                let who = c.firstname.clone();
                record = None;
                Some(Msg::GangPromoteInviteFirst { who })
            }
        },
        // (sic) the lieutenant test never holds, so every member found gets the notice
        OfflineOp::Demote => match record.as_mut() {
            None => Some(Msg::GangDemoteSyntax),
            Some((_, c)) if eq_ignore_case(&c.gang, gang) => {
                c.flags.insert(PlayerFlags::DEMOTED);
                Some(Msg::GangDemoteLater {
                    who: c.firstname.clone(),
                })
            }
            Some(_) => {
                record = None;
                Some(Msg::GangDemoteOutsiderOffline)
            }
        },
    };
    match (record, online) {
        (Some((Some(account), character)), _) => {
            crate::player::queue_character(world, account, Box::new(character));
        }
        (Some((None, character)), Some(s)) => {
            if let Some(p) = player_mut(world, s) {
                *p = character;
            }
        }
        _ => {}
    }
    if let Some(s) = world.session_mut(id) {
        if let Some(m) = msg {
            s.output.push(m);
        }
        s.push_prompt();
    }
}

/// A character first saved since the store's read has no row yet: its queued record,
/// found by first name as the store's read would (T-088).
fn queued_by_first_name(world: &World, name: &str) -> Option<(AccountId, Box<Character>)> {
    let mut found: Vec<_> = world
        .queued_characters
        .iter()
        .filter_map(|(a, q)| q.record.as_ref().map(|c| (*a, c)))
        .filter(|(_, c)| eq_ignore_case(&c.firstname, name))
        .collect();
    found.sort_by_key(|(a, _)| *a);
    found.first().map(|(a, c)| (*a, (*c).clone()))
}

/// The name removing an offline member leaves in the typed argument:
/// its first letter upper-cased for the exact lookup; when that lookup fails (the stored
/// first name is not exactly that), the rest lower-cased as well for the retry.
fn typed_name_as_rewritten(typed: &str, stored: Option<&str>) -> String {
    let mut up = typed.to_string();
    if let Some(first) = up.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    if stored == Some(up.as_str()) {
        return up;
    }
    let mut cap = typed.to_ascii_lowercase();
    if let Some(first) = cap.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    cap
}

/// What a name typed after INVITE MEMBER or UNINVITE MEMBER names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Nothing (`FTT_NONE`).
    None,
    /// A player (`FTT_USER`).
    User(SessionId),
    /// A monster (`FTT_MONSTER`).
    Monster,
    /// Several players fit and none in full (`FTT_MULTIPLE`); the names, for the
    /// "Please be more specific" list.
    Multiple(Vec<String>),
}

/// The player steps of `find_action_target` for players other than oneself: the players
/// in `id`'s room but `id` whose first name the first word of `name` abbreviates
/// ([`matches_any_word`]); a hidden player only for one who sees
/// hidden things. A full name ends the search.
pub fn find_room_user(world: &World, id: SessionId, name: &str) -> Target {
    let Some(me) = player(world, id) else {
        return Target::None;
    };
    let (map, room) = (me.map, me.room);
    let sees_hidden = me
        .status
        .contains(silvermere_types::flags::PlayerStatus::SEEHIDDEN);
    let first = name.split(' ').next().unwrap_or("");
    let mut found = None;
    let mut names = Vec::new();
    for other in world.session_ids() {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some(p) = player(world, other) else {
            continue;
        };
        if p.map != map || p.room != room || !matches_any_word(first, &p.firstname) {
            continue;
        }
        if p.hidden != 0 && !sees_hidden {
            continue;
        }
        if eq_ignore_case(first, &p.firstname) {
            return Target::User(other);
        }
        found = Some(other);
        names.push(p.firstname.clone());
    }
    match (found, names.len()) {
        (_, n) if n > 1 => Target::Multiple(names),
        (Some(t), _) => Target::User(t),
        _ => Target::None,
    }
}

/// The first player in the game, in channel order, whose first name `name` abbreviates
/// ([`matches_any_word`]), else [`Target::Monster`] when the name is part of the name of a monster
/// in the cache ([`find_monster_in_memory`], T-086).
pub fn find_any_action_target(world: &World, name: &str) -> Target {
    for other in world.session_ids() {
        if !world.is_in_game(other) {
            continue;
        }
        if player(world, other).is_some_and(|p| matches_any_word(name, &p.firstname)) {
            return Target::User(other);
        }
    }
    if find_monster_in_memory(world, name).is_some() {
        return Target::Monster;
    }
    Target::None
}

/// The first active monster
/// in the cache (slots 0..=monsbuf) whose name contains `name` (`contains_ignore_case`).
pub fn find_monster_in_memory(world: &World, name: &str) -> Option<ActiveMonsterId> {
    let resident: Vec<_> = world
        .active_monsters
        .cache()
        .resident()
        .map(|(_, n)| n)
        .collect();
    resident.into_iter().find(|&n| {
        world
            .active_monsters
            .peek(n)
            .is_some_and(|m| crate::display::sysop::contains_ignore_case(name, &m.name))
    })
}

/// The value of `ABL_GANGSHOPITEM` on carried item `n`, 0 for none or an unknown item.
pub fn gang_shop_item_value(world: &World, n: silvermere_types::ids::ItemNo) -> i32 {
    world.items.get(&n).map_or(0, |itm| {
        item_ability_value(itm, silvermere_types::enums::Ability::GangShopItem)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silvermere_types::records::ShopSlot;

    fn gang(exp: u32) -> Gang {
        Gang {
            key: "WOLVES".into(),
            name: "Wolves".into(),
            exp,
            leader: "Bob".into(),
            created: 0,
            members: 1,
            flags: GangFlags::empty(),
            overexp: 0,
            overflows: 0,
        }
    }

    #[test]
    fn gang_exp_adds_and_overflows() {
        let mut g = gang(10);
        add_kill_exp(&mut g, 5);
        assert_eq!((g.exp, g.overexp, g.flags), (15, 0, GangFlags::empty()));
        let mut g = gang(u32::MAX - 3);
        add_kill_exp(&mut g, 10);
        assert_eq!(g.exp, u32::MAX);
        assert_eq!(g.overexp, 7);
        assert!(g.flags.contains(GangFlags::OVERFLOW));
        add_kill_exp(&mut g, u32::MAX - 7);
        assert_eq!((g.overexp, g.overflows), (u32::MAX, 0));
        add_kill_exp(&mut g, 5);
        // room = 0, overexp = 5 - 0, one wrap
        assert_eq!((g.overexp, g.overflows), (5, 1));
    }

    #[test]
    fn shop_owner_round_trips_through_maxqty() {
        let mut shop = Shop {
            number: ShopNo(1),
            name: String::new(),
            desc: Default::default(),
            shop_type: silvermere_types::enums::ShopType::GangShop,
            minlevel: 0,
            maxlevel: 0,
            markup: 0,
            ownable: 0,
            expandable: 0,
            trainclass: silvermere_types::ids::ClassNo(0),
            stock: [ShopSlot {
                item: silvermere_types::ids::ItemNo(0),
                maxqty: 0xFFFF,
                qty: 0,
                regentime: 0,
                regennum: 0,
                regenpct: 0,
            }; NSHOPITEMS],
        };
        set_gang_shop_owner(&mut shop, "bob");
        assert_eq!(gang_shop_owner(&shop), "bob");
        assert_eq!(shop.stock[0].maxqty, u16::from_le_bytes(*b"bo"));
        assert_eq!(shop.stock[1].maxqty, u16::from_le_bytes([b'b', 0]));
        assert_eq!(shop.stock[2].maxqty, 0xFFFF);
    }
}
