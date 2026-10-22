//! The daily cleanup, run in the process with the players connected (T-056).
//!
//! The original's BBS ran the cleanup at its cleanup hour with nobody on line. Here
//! [`crate::scheduler::Routine::DailyCleanup`] fires at `game.cleanup.at` local time
//! ([`on_due`]); the engine asks the store for the stored characters
//! ([`EngineOutput::CleanupScan`]) and runs the four passes in the one input that carries
//! the answer ([`on_scan`]), between ticks, so the job is atomic with respect to commands
//! and combat rounds. Nobody is disconnected, refused or told; the original's console and
//! log lines go to the audit log ([`GameEvent::Audit`]) and `tracing`. Every change goes
//! out as write-behind persistence requests.
//!
//! | Pass | Here |
//! |---|---|
//! | 1a deeds | `find_deed_holders` |
//! | 1b tax | `collect_tax` |
//! | 2 rooms | `sweep_rooms` |
//! | 3 users | `users` |
//! | 4 gangs | `delete_gangs` |
//! | the restart | [`crate::monsters::cold_reset`], [`crate::shops::restock_untimed_all`], [`crate::monsters::refill_shops`], `rebuild_top_list` |
//!
//! The original's BBS went down at its cleanup hour and came back up, so its start-up
//! cycle's cold-start room reset ran every night after the passes: the rooms' recent-kill
//! and found-hidden-item markers cleared, the placed items put back, the exits locked,
//! re-armed and hidden again, and its shops' slots with no restock interval got their one
//! roll. Here the server stays up, so the run ends with that reset and that roll, then the
//! start-up cycle's Shop-room fill (the specific monster, then the room's pool up to its
//! cap) and the start-up build of the top list. The other rooms' specific monsters and the
//! lair fills are not run here: a specific monster that is not alive is made when a player
//! enters its room, and lairs fill on entry and in the spawn pass.
//!
//! A character in a session goes through the passes with its live record (`sesstime`
//! from the session so far; a house that closes under it moves it at once, with the
//! notice and the new room); its stored row is dropped. A character whose session ended
//! while the store was answering goes through the passes with the record its logoff
//! saved, which the stored row may not have caught up with ([`note_departure`]). A scan
//! the store could not answer is dropped ([`on_scan_failed`]) and the next run scans
//! again.

use std::collections::{HashMap, HashSet};

use silvermere_rules::abilities::{item_ability_value, item_has_ability};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Currency, RoomType, ShopType};
use silvermere_types::flags::{GangFlags, PlayerFlags, PlayerSettings, RoomFlags};
use silvermere_types::ids::{AccountId, ItemNo, MapId, RoomId, RoomKey, SessionId, ShopNo};
use silvermere_types::records::{Character, RoomItem};
use silvermere_types::sizes::{NHIDITEMS, NINVITEMS, NKEYS, NROOMITEMS, NSHOPITEMS};

use crate::commands::eq_ignore_case;
use crate::display::{RoomMode, display_room};
use crate::event_loop::{AuditValue, EngineOutput, GameEvent, PersistRequest};
use crate::items::inventory::remove_item_from_record;
use crate::items::room_items::{mark_changed, remove_item_from_room};
use crate::progression::TopTen;
use crate::world::World;

/// The last gang house number.
pub const GH_WHITE: usize = 10;

/// The bank the gang-house tax is taken from.
pub const TAX_BANK: ShopNo = ShopNo(8);

/// The walks of WCCUSERS and WCCGANGS stop after this many records. (sic) The deed walk of
/// pass 1 never counts.
pub const MAX_WALK: usize = 10_000;

/// Rooms with more than this many items on the floor are thinned out.
pub const CROWDED: usize = 8;

/// The house is held by a character whose tax went unpaid (`flags` bit 0).
pub const EVICTED: u8 = 1;
/// The tax was paid (`flags` bit 1).
pub const TAX_PAID: u8 = 2;

/// One gang house's ledger entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HouseLedger {
    /// Bit 0 evicted, bit 1 tax paid.
    pub flags: u8,
    /// The deed holder's user-id.
    pub leader: String,
    /// The holder's account (to find the bank book).
    pub leader_account: Option<AccountId>,
    /// The holder's gang.
    pub gang: String,
    /// The tax due, in copper (the deed's `ABL_GANGTAX` value is in silver).
    pub tax: u64,
    /// Visible items swept from the house.
    pub nitems: u32,
    /// Hidden items swept from the house.
    pub nhidden: u32,
    /// A deed holder was found and has not failed to pay.
    pub occupied: bool,
}

impl HouseLedger {
    /// Evicted, or no holder. (sic) The tax-paid bit is never looked at: as in the original,
    /// an unpaid tax never closes a house.
    pub fn closed(&self) -> bool {
        self.flags & EVICTED != 0 || !self.occupied
    }
}

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// Items moved to the hidden slots.
    pub items_hidden: u32,
    /// Characters who played since the last cleanup.
    pub users: u32,
    /// Characters looked at.
    pub examined: u32,
    /// Minutes played since the last cleanup, all characters.
    pub minutes: u64,
    /// Evil points forgiven, all characters.
    pub evil_forgiven: i32,
    /// Gangs deleted in pass 4.
    pub gangs_deleted: u32,
    /// Placed items put back by the room reset.
    pub placed_items: u32,
    /// Exits locked, re-armed or hidden again by the room reset.
    pub exits_reset: u32,
    /// Shop slots with no restock interval topped up by the restart's restock.
    pub shop_slots_restocked: u32,
    /// Monsters placed in Shop rooms by the refill after the room reset.
    pub shop_monsters: u32,
    /// Entries on the top list as rebuilt at the end of the run.
    pub top_list: u32,
    /// The gang houses, `GH_NONE..=GH_WHITE` (index 0 is never held).
    pub houses: [HouseLedger; GH_WHITE + 1],
}

/// The cleanup's state between [`begin`] and [`on_scan`].
#[derive(Debug, Clone, Default)]
pub struct CleanupState {
    /// Set while the store is answering: the record each account whose session ended
    /// since was saved with (`on_logoff`). The stored row may be older than that save, so
    /// the run uses this record instead.
    pub pending: Option<HashMap<AccountId, Character>>,
    /// The last run's report.
    pub last: Option<CleanupReport>,
}

/// A stored character that is not in a session, as the store read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offline {
    pub account: AccountId,
    pub character: Character,
}

/// Seconds from `now` (Unix seconds) to the next `at` local time in `tz`, at least 1.
pub fn seconds_until(now: i64, at: (u8, u8), tz: &jiff::tz::TimeZone) -> u64 {
    let Ok(ts) = jiff::Timestamp::from_second(now) else {
        return 86_400;
    };
    let zoned = ts.to_zoned(tz.clone());
    let mut date = zoned.date();
    for _ in 0..3 {
        let target = date
            .at(
                i8::try_from(at.0).unwrap_or(3),
                i8::try_from(at.1).unwrap_or(0),
                0,
                0,
            )
            .to_zoned(tz.clone())
            .map(|z| z.timestamp().as_second());
        if let Ok(t) = target
            && t > now
        {
            return u64::try_from(t - now).unwrap_or(1).max(1);
        }
        let Ok(next) = date.tomorrow() else {
            break;
        };
        date = next;
    }
    86_400
}

/// Arms the next run at `game.cleanup.at` from [`World::wall`].
pub fn arm(world: &mut World) {
    use crate::scheduler::Routine;
    world.scheduler.disarm(Routine::DailyCleanup);
    let at = world.config.cleanup.hour_minute().unwrap_or((3, 0));
    // `game.cleanup.tz` is checked when the config loads; UTC if it somehow is not known
    let tz = jiff::tz::TimeZone::get(&world.config.cleanup.tz).unwrap_or(jiff::tz::TimeZone::UTC);
    let delay = seconds_until(world.wall, at, &tz);
    world.scheduler.arm(delay, Routine::DailyCleanup);
}

/// The scheduled run is due: start it and arm the next one.
pub fn on_due(world: &mut World) {
    begin(world);
    arm(world);
}

/// Starts a run (the routine, or `EngineInput::DailyCleanup`): the store is asked for the
/// stored characters and the passes run when it answers ([`on_scan`]). If an earlier
/// scan is still unanswered (the store lost it), the store is asked again, keeping the
/// departures recorded so far; whichever answer comes first runs, the others are dropped.
/// The cleanup is never left waiting for good (the card: never abort).
pub fn begin(world: &mut World) {
    if world.cleanup.pending.is_some() {
        tracing::warn!("daily cleanup: the last character scan was never answered; asking again");
    } else {
        world.cleanup.pending = Some(HashMap::new());
    }
    world.outbox.push(EngineOutput::CleanupScan);
}

/// A session leaves while the store is answering (`on_logoff`): `saved` is the record
/// its last save queued, which the stored row may not have caught up with yet. The run
/// uses it as the account's stored record.
pub fn note_departure(world: &mut World, account: Option<AccountId>, saved: Option<Character>) {
    if let (Some(map), Some(acct), Some(c)) = (world.cleanup.pending.as_mut(), account, saved) {
        map.insert(acct, c);
    }
}

/// The store could not answer the scan (a flush or read error): the run is dropped with a
/// warning and the next one ([`on_due`] or `EngineInput::DailyCleanup`) scans again.
pub fn on_scan_failed(world: &mut World) {
    if world.cleanup.pending.take().is_some() {
        tracing::warn!("daily cleanup: the character scan failed; skipped until the next run");
    }
}

/// The store's answer: every stored character with its account. A character in a session
/// goes through the passes with its live record, so its stored row is dropped; a session
/// that ended since [`begin`] stands for its account with the record it saved last. Then
/// the passes run. `None` when no run is waiting (an answer to a scan already served).
pub fn on_scan(world: &mut World, rows: Vec<(AccountId, Character)>) -> Option<CleanupReport> {
    let Some(mut departed) = world.cleanup.pending.take() else {
        tracing::warn!("daily cleanup: a character scan answer with no run waiting; dropped");
        return None;
    };
    let online: HashSet<AccountId> = world
        .sessions
        .values()
        .filter_map(|s| s.account.as_ref().map(|a| a.id))
        .collect();
    // T-088: a write queued since the store read (a hang-up's save, a gang offline edit)
    // stands for the row, as the latest state of that character; a departure's record is
    // only used for an account whose queued record is gone (taken by a login since)
    let rows = crate::player::with_queued(world, rows);
    departed.retain(|a, _| !world.queued_characters.contains_key(a));
    let mut offline: Vec<Offline> = rows
        .into_iter()
        .filter(|(a, _)| !online.contains(a))
        .map(|(account, character)| {
            let character = departed.remove(&account).unwrap_or(character);
            Offline { account, character }
        })
        .collect();
    // a character first saved during the scan has no stored row yet
    let mut rest: Vec<(AccountId, Character)> = departed
        .into_iter()
        .filter(|(a, _)| !online.contains(a))
        .collect();
    rest.sort_by_key(|(a, _)| *a);
    offline.extend(
        rest.into_iter()
            .map(|(account, character)| Offline { account, character }),
    );
    Some(run_cleanup(world, &mut offline))
}

/// One record of the character walks: in a session, or stored.
#[derive(Debug, Clone, Copy)]
enum Rec {
    Online(SessionId),
    Offline(usize),
}

/// The characters in WCCUSERS key order (the user-id, upper case).
fn records(world: &World, offline: &[Offline]) -> Vec<Rec> {
    let mut recs: Vec<(String, Rec)> = Vec::new();
    for id in world.session_ids() {
        if let Some(s) = world.session(id)
            && s.created
            && s.account.is_some()
            && let Some(c) = s.character.as_ref()
        {
            recs.push((c.userid.to_ascii_uppercase(), Rec::Online(id)));
        }
    }
    for (i, o) in offline.iter().enumerate() {
        recs.push((o.character.userid.to_ascii_uppercase(), Rec::Offline(i)));
    }
    recs.sort_by(|a, b| a.0.cmp(&b.0));
    recs.into_iter().map(|(_, r)| r).collect()
}

fn account_of(world: &World, offline: &[Offline], rec: Rec) -> Option<AccountId> {
    match rec {
        Rec::Online(id) => world.session(id)?.account.as_ref().map(|a| a.id),
        Rec::Offline(i) => offline.get(i).map(|o| o.account),
    }
}

fn character_of<'a>(world: &'a World, offline: &'a [Offline], rec: Rec) -> Option<&'a Character> {
    match rec {
        Rec::Online(id) => world.session(id)?.character.as_ref(),
        Rec::Offline(i) => offline.get(i).map(|o| &o.character),
    }
}

fn audit(world: &mut World, kind: &'static str, fields: Vec<(&'static str, AuditValue)>) {
    world
        .outbox
        .push(EngineOutput::Event(GameEvent::Audit { kind, fields }));
}

fn int(v: impl Into<i64>) -> AuditValue {
    AuditValue::Int(v.into())
}

/// The cleanup: the four passes in order over the characters in sessions and `offline`,
/// and every room and gang, then the cold-start room reset.
pub fn run_cleanup(world: &mut World, offline: &mut [Offline]) -> CleanupReport {
    let mut report = CleanupReport::default();
    // the original's console notice "Starting Cleanup"
    audit(world, "cleanup.start", Vec::new());
    tracing::info!(target: "mmlog", "Entering cleanup");
    let recs = records(world, offline);
    find_deed_holders(world, offline, &recs, &mut report.houses);
    collect_tax(world, &mut report.houses);
    report.items_hidden = sweep_rooms(world, &mut report.houses);
    // the original's console line "Hid %d items."
    audit(
        world,
        "cleanup.items",
        vec![("hidden", int(report.items_hidden))],
    );
    log_houses(&report.houses);
    users(world, offline, &recs, &mut report);
    report.gangs_deleted = delete_gangs(world);
    // the room reset the original's restart after the cleanup gave
    let reset = crate::monsters::cold_reset(world);
    report.placed_items = reset.placed;
    report.exits_reset = reset.exits;
    // the restart's restock of the shop slots with no interval
    let mut rng = world.rng.clone();
    report.shop_slots_restocked = crate::shops::restock_untimed_all(world, &mut rng);
    world.rng = rng;
    // the Shop rooms' monsters and the top list, which the restart also rebuilt. The fill
    // comes after the reset clears the recent kills: at the restart a kill's minute stamp,
    // counted from the old start-up's day, lay ahead of the new start-up's minute and so
    // held no monster back, though the start-up fills a room before clearing its kill
    report.shop_monsters = crate::monsters::refill_shops(world).monsters;
    report.top_list = rebuild_top_list(world, offline);
    // the original's console notice "Cleanup Complete"
    audit(
        world,
        "cleanup.complete",
        vec![
            ("gangs_deleted", int(report.gangs_deleted)),
            ("placed_items", int(report.placed_items)),
            ("exits_reset", int(report.exits_reset)),
            ("shop_slots_restocked", int(report.shop_slots_restocked)),
            ("shop_monsters", int(report.shop_monsters)),
            ("top_list", int(report.top_list)),
        ],
    );
    tracing::info!(
        target: "mmlog",
        "Cleanup complete: {} placed items put back, {} exits reset, {} shop slots restocked, {} shop monsters placed, top list rebuilt ({} entries)",
        report.placed_items,
        report.exits_reset,
        report.shop_slots_restocked,
        report.shop_monsters,
        report.top_list
    );
    world.cleanup.last = Some(report.clone());
    report
}

/// The top list built again as at start-up ([`TopTen::rebuild`]), over every character,
/// online ones in their live state, leaving out the ones taken off it. They come in
/// descending user-id order, so of two with equal experience the later by user-id lists
/// above the earlier, as in the start-up build. Returns the entries on it.
fn rebuild_top_list(world: &mut World, offline: &[Offline]) -> u32 {
    let mut recs = records(world, offline);
    recs.reverse();
    let list = TopTen::rebuild(recs.iter().filter_map(|r| character_of(world, offline, *r)));
    let n = list.len() as u32;
    world.topten = list;
    n
}

/// Pass 1a: every character in a gang holding a Gang House Deed in its inventory holds
/// that house (the deed's value); the deed's `ABL_GANGTAX` value is the daily tax in
/// silver, kept in copper. A later holder of the same house in key order wins.
fn find_deed_holders(
    world: &World,
    offline: &[Offline],
    recs: &[Rec],
    gh: &mut [HouseLedger; GH_WHITE + 1],
) {
    for &rec in recs {
        let Some(c) = character_of(world, offline, rec) else {
            continue;
        };
        if c.gang.is_empty() {
            continue;
        }
        for &n in c.items.iter().take(NINVITEMS) {
            if n == ItemNo(0) {
                continue;
            }
            let Some(itm) = world.items.get(&n) else {
                continue;
            };
            if !item_has_ability(itm, Ability::GangDeed) {
                continue;
            }
            // `unsigned idx`: a negative value is out of range
            let Ok(idx) = usize::try_from(item_ability_value(itm, Ability::GangDeed)) else {
                continue;
            };
            if idx > GH_WHITE {
                continue;
            }
            let tax = item_ability_value(itm, Ability::GangTax);
            let h = &mut gh[idx];
            h.leader.clone_from(&c.userid);
            h.leader_account = account_of(world, offline, rec);
            h.gang.clone_from(&c.gang);
            h.tax = (i64::from(tax) * 100) as u64;
            h.flags = 0;
            h.occupied = true;
            tracing::info!(target: "mmlog",
                "GANGHOUSE : NMBR {idx} - LEADER {} - GANG {} - HOUSE TAX {}. ",
                h.leader, h.gang, h.tax);
        }
    }
}

/// Pass 1b: each held house's tax comes out of its holder's bank book at bank 8. Without
/// a book, or with less in it than the tax, the house is evicted (bit 0, no longer
/// occupied); otherwise the tax is taken (bit 1).
pub fn collect_tax(world: &mut World, gh: &mut [HouseLedger; GH_WHITE + 1]) {
    for (i, h) in gh.iter_mut().enumerate().skip(1) {
        if h.tax == 0 {
            continue;
        }
        let book = h
            .leader_account
            .and_then(|a| world.bank_books.get_mut(&(a, TAX_BANK)).map(|b| (a, b)));
        let paid = match book {
            Some((_, b)) if h.tax > u64::from(b.balance) => {
                tracing::info!(target: "mmlog",
                    "GANGHOUSE : Gang leader {} couldn't pay his gang house tax. BALANCE {}. ",
                    b.userid, b.balance);
                false
            }
            Some((acct, b)) => {
                let old = b.balance;
                b.balance -= h.tax as u32;
                tracing::info!(target: "mmlog",
                    "GANGHOUSE : Leader {} paid house tax. OLD SILVERMERE BALANCE {old} NEW BALANCE {}. ",
                    b.userid, b.balance);
                let book = b.clone();
                world
                    .outbox
                    .push(EngineOutput::Persist(PersistRequest::BankBook {
                        account: acct,
                        book,
                    }));
                true
            }
            None => false,
        };
        if paid {
            h.flags &= !EVICTED;
            h.flags |= TAX_PAID;
            h.occupied = true;
        } else {
            h.flags |= EVICTED;
            h.flags &= !TAX_PAID;
            h.occupied = false;
        }
        let kind = if paid {
            "ganghouse.tax"
        } else {
            "ganghouse.evicted"
        };
        let fields = vec![
            ("house", int(i as i64)),
            ("leader", AuditValue::Text(h.leader.clone())),
            ("gang", AuditValue::Text(h.gang.clone())),
            ("tax", int(h.tax as i64)),
        ];
        world
            .outbox
            .push(EngineOutput::Event(GameEvent::Audit { kind, fields }));
    }
}

/// The house table entry for a house number, `None` past `GH_WHITE`. Entry 0 is never
/// filled; it reads as closed.
fn house(gh: &[HouseLedger; GH_WHITE + 1], n: i64) -> Option<&HouseLedger> {
    usize::try_from(n).ok().and_then(|i| gh.get(i))
}

fn empty_slot() -> RoomItem {
    RoomItem {
        item: ItemNo(0),
        uses: -1,
        count: 0,
    }
}

/// The 30-character item name `remove_item_from_room` is given (cut at 30).
fn name30(name: &str) -> String {
    name.chars().take(30).collect()
}

/// The rooms the cleanup sweeps, in order, as the original sweeps them: maps from 1 until
/// 100 maps had no room 1 before their first gap, rooms from 1 until a map had 50 missing
/// numbers. (sic) A map's walk stops at its 50th missing room number however far apart the
/// gaps are, so rooms numbered past that are never swept.
pub fn walk_order(world: &World) -> Vec<RoomKey> {
    let mut maps: Vec<u32> = world.rooms.keys().map(|k| k.map.0).collect();
    maps.sort_unstable();
    maps.dedup();
    let last_map = maps.last().copied().unwrap_or(0);
    let mut order = Vec::new();
    let mut nemptymaps = 0;
    let mut map = 1u32;
    while nemptymaps < 100 && map <= last_map.saturating_add(100) {
        let (mut nemptyrooms, mut seen, mut counted) = (0, false, false);
        let mut room = 1u32;
        while nemptyrooms < 50 {
            let key = RoomKey::new(map, room);
            if world.rooms.contains_key(&key) {
                seen = true;
                order.push(key);
            } else {
                nemptyrooms += 1;
                if !seen && !counted {
                    nemptymaps += 1;
                    counted = true;
                }
            }
            room += 1;
        }
        map += 1;
    }
    order
}

/// Pass 2: every room. Returns the number of items hidden.
pub fn sweep_rooms(world: &mut World, gh: &mut [HouseLedger; GH_WHITE + 1]) -> u32 {
    let mut nswept = 0;
    for key in walk_order(world) {
        nswept += sweep_room(world, key, gh);
    }
    nswept
}

fn sweep_room(world: &mut World, key: RoomKey, gh: &mut [HouseLedger; GH_WHITE + 1]) -> u32 {
    let Some(rm) = world.rooms.get(&key) else {
        return 0;
    };
    let (flags, ganghouse, room_type, shopnum, placed) = (
        rm.flags,
        i64::from(rm.ganghouse.code()),
        rm.room_type,
        rm.shopnum,
        rm.placed,
    );
    if !world.room_state.contains_key(&key) {
        return 0;
    }
    let mut changed = false;
    let mut nswept = 0;
    if flags.contains(RoomFlags::GANGHOUSE) {
        // (sic) a house number past GH_WHITE skips the rest of the room
        if ganghouse > GH_WHITE as i64 {
            return 0;
        }
        let closed = house(gh, ganghouse).is_none_or(|h| ganghouse == 0 || h.closed());
        if closed {
            if ganghouse != 0 {
                changed |=
                    empty_closed_house(world, key, gh, ganghouse as usize, room_type, shopnum);
            }
        } else {
            changed |= take_closed_houses_items(world, key, gh);
        }
    } else if room_type == RoomType::Shop {
        reopen_deed_shop(world, shopnum, gh);
    }
    let World {
        items, room_state, ..
    } = &mut *world;
    let Some(state) = room_state.get_mut(&key) else {
        return 0;
    };
    // loose coins of an ordinary room go to the hidden coins
    if ganghouse == 0 {
        let c = &mut state.coins;
        let h = &mut state.hidcoins;
        for (v, hv) in [
            (&mut c.runic, &mut h.runic),
            (&mut c.platinum, &mut h.platinum),
            (&mut c.gold, &mut h.gold),
            (&mut c.silver, &mut h.silver),
            (&mut c.copper, &mut h.copper),
        ] {
            if *v != 0 {
                changed = true;
                *hv = hv.wrapping_add(*v);
                *v = 0;
            }
        }
    }
    // the floor of an ordinary room
    let mut nitems = 0;
    let mut removals: Vec<(String, usize, bool)> = Vec::new();
    let mut deeds: Vec<(String, i32)> = Vec::new();
    if ganghouse == 0 {
        for i in 0..NROOMITEMS {
            let slot = state.items[i];
            if slot.item == ItemNo(0) {
                continue;
            }
            let Some(itm) = items.get(&slot.item) else {
                continue;
            };
            nitems += 1;
            if itm.gettable == 0 {
                // only a Gang House Deed is taken off the floor
                if item_has_ability(itm, Ability::GangDeed) {
                    deeds.push((
                        name30(&itm.name),
                        item_ability_value(itm, Ability::GangDeed),
                    ));
                }
                continue;
            }
            let delete = itm.cost == 0 || item_has_ability(itm, Ability::DelCleanup);
            if flags.contains(RoomFlags::CLEANUP) && itm.limit != 0 {
                // a limited item copy by copy (remove_item_from_room keeps the owner list
                // right); (sic) while extra copies remain, so the slot's last copy stays
                removals.push((name30(&itm.name), i, false));
            } else if flags.contains(RoomFlags::CLEANUP) || (delete && itm.limit == 0) {
                // the slot is cleared
                state.items[i] = empty_slot();
                changed = true;
            } else {
                let keep =
                    !placed.contains(&slot.item) && !item_has_ability(itm, Ability::KeepVisible);
                if !keep {
                    continue;
                }
                // join a hidden stack of the same uncharged item; (sic) only the extra
                // copies are added, the slot's own copy is lost
                if slot.uses == -1
                    && let Some(k) = state.hiditems.iter().position(|h| h.item == slot.item)
                {
                    state.hiditems[k].uses = slot.uses;
                    state.hiditems[k].count = state.hiditems[k].count.wrapping_add(slot.count);
                    state.items[i] = empty_slot();
                    changed = true;
                    nswept += 1;
                } else if let Some(k) = state.hiditems.iter().position(|h| h.item == ItemNo(0)) {
                    // hide it in the first free hidden slot
                    state.hiditems[k] = slot;
                    state.items[i] = empty_slot();
                    changed = true;
                    nswept += 1;
                }
            }
        }
    }
    // a crowded room loses its cheap unlimited items
    if nitems > CROWDED && ganghouse == 0 {
        for i in 0..NROOMITEMS {
            let slot = state.items[i];
            if slot.item == ItemNo(0) {
                continue;
            }
            let Some(itm) = items.get(&slot.item) else {
                continue;
            };
            if itm.gettable == 0 || placed.contains(&slot.item) {
                continue;
            }
            let dear = itm.currency.code() > Currency::Gold.code()
                || (itm.currency == Currency::Gold && itm.cost > 14);
            if !dear && itm.limit == 0 {
                state.items[i] = empty_slot();
                changed = true;
            }
        }
    }
    // the hidden items of a RF_CLEANUP room
    if flags.contains(RoomFlags::CLEANUP) && ganghouse == 0 {
        for i in 0..NHIDITEMS {
            let slot = state.hiditems[i];
            if slot.item == ItemNo(0) {
                continue;
            }
            let Some(itm) = items.get(&slot.item) else {
                continue;
            };
            if itm.gettable == 0 {
                continue;
            }
            if itm.limit != 0 {
                removals.push((name30(&itm.name), i, true));
            } else {
                state.hiditems[i] = empty_slot();
                changed = true;
            }
        }
    } else if ganghouse != 0 {
        tracing::info!(target: "mmlog",
            "GANGHOUSE : DELETE_AT_CLEANUP : Skipping room cleanup. Room {} Map {} Gang house Number {}.",
            key.room, key.map, ganghouse);
    }
    // free or "Delete at cleanup" unlimited hidden items of an ordinary room
    if ganghouse == 0 {
        for i in 0..NHIDITEMS {
            let slot = state.hiditems[i];
            if slot.item == ItemNo(0) {
                continue;
            }
            let Some(itm) = items.get(&slot.item) else {
                continue;
            };
            if itm.gettable != 0
                && (itm.cost == 0 || item_has_ability(itm, Ability::DelCleanup))
                && itm.limit == 0
            {
                state.hiditems[i] = empty_slot();
                changed = true;
            }
        }
    }
    // The limited items of a RF_CLEANUP room and the deeds on the floor go through
    // `remove_item_from_room`, which releases the copies (WCCITOWN) and marks the room.
    // These removals come last here, after the other steps, rather than at the walk's slot.
    // `remove_item_from_room` takes the first fitting item, not the slot the walk was at:
    // the first visible item whose name the 30-character name abbreviates, or (sic, as in
    // the original) any Gang House Deed in an earlier slot, then a found hidden one. So
    // when another slot of the room fits the name or holds a deed, a removal made last can
    // take a different copy than one made at the walk's slot would. Neither happens in the
    // stock world's RF_CLEANUP rooms; accepted.
    for (name, i, hidden) in removals {
        let mut n = 0;
        while n < 500 && extra_copies(world, key, i, hidden) > 0 {
            n += 1;
            remove_item_from_room(world, key, &name, false);
        }
    }
    for (name, idx) in deeds {
        remove_item_from_room(world, key, &name, false);
        tracing::info!(target: "mmlog",
            "GANGHOUSE : Removing GANGHOUSE DEED {idx} from Room {} Map {} floor.",
            key.room, key.map);
    }
    if changed {
        mark_changed(world, key);
    }
    nswept
}

fn extra_copies(world: &World, key: RoomKey, i: usize, hidden: bool) -> i16 {
    world.room_state.get(&key).map_or(0, |s| {
        if hidden {
            s.hiditems[i].count
        } else {
            s.items[i].count
        }
    })
}

/// A closed house's room loses its coins (not the runic ones), its floor and hidden items
/// (counted in the house's entry) and, for its gang shop, the first 10 slots of stock and
/// the owner.
fn empty_closed_house(
    world: &mut World,
    key: RoomKey,
    gh: &mut [HouseLedger; GH_WHITE + 1],
    n: usize,
    room_type: RoomType,
    shopnum: ShopNo,
) -> bool {
    let Some(state) = world.room_state.get_mut(&key) else {
        return false;
    };
    let mut changed = false;
    gh[n].nitems = 0;
    gh[n].nhidden = 0;
    let (c, h) = (&mut state.coins, &mut state.hidcoins);
    for (v, hv) in [
        (&mut c.platinum, &mut h.platinum),
        (&mut c.gold, &mut h.gold),
        (&mut c.silver, &mut h.silver),
        (&mut c.copper, &mut h.copper),
    ] {
        if *v != 0 {
            changed = true;
            *v = 0;
            *hv = 0;
        }
    }
    for slot in state.items.iter_mut() {
        if slot.item != ItemNo(0) {
            gh[n].nitems += 1;
            *slot = empty_slot();
            changed = true;
        }
    }
    for slot in state.hiditems.iter_mut() {
        if slot.item != ItemNo(0) {
            gh[n].nhidden += 1;
            *slot = empty_slot();
            changed = true;
        }
    }
    if room_type == RoomType::Shop
        && let Some(shp) = world.shops.get_mut(&shopnum)
        && shp.shop_type == ShopType::GangShop
    {
        for slot in shp.stock.iter_mut().take(10) {
            slot.item = ItemNo(0);
            slot.qty = 0;
            slot.regentime = 0;
            slot.regennum = 0;
        }
        // the owner's user-id, kept in the first stock slot's `maxqty`, is cut to nothing
        let le = shp.stock[0].maxqty.to_le_bytes();
        shp.stock[0].maxqty = u16::from_le_bytes([0, le[1]]);
        crate::gangs::save_gang_shop(world, shopnum);
    }
    changed
}

/// In an open house, the Gang House Items that belong to a closed house go (counted in
/// that house's `nitems`, hidden ones too, sic).
fn take_closed_houses_items(
    world: &mut World,
    key: RoomKey,
    gh: &mut [HouseLedger; GH_WHITE + 1],
) -> bool {
    let items = &world.items;
    let Some(state) = world.room_state.get_mut(&key) else {
        return false;
    };
    let mut changed = false;
    for slot in state.items.iter_mut().chain(state.hiditems.iter_mut()) {
        if slot.item == ItemNo(0) {
            continue;
        }
        let Some(itm) = items.get(&slot.item) else {
            continue;
        };
        if !item_has_ability(itm, Ability::GangItem) {
            continue;
        }
        let n = i64::from(item_ability_value(itm, Ability::GangItem));
        if n > GH_WHITE as i64 {
            continue;
        }
        if house(gh, n).is_none_or(|h| n == 0 || h.closed()) {
            if let Ok(n) = usize::try_from(n) {
                gh[n].nitems += 1;
            }
            *slot = empty_slot();
            changed = true;
        }
    }
    changed
}

/// In the deed shop, the deed of a closed house goes back on sale (one in stock) and the
/// house is marked evicted.
fn reopen_deed_shop(world: &mut World, shopnum: ShopNo, gh: &mut [HouseLedger; GH_WHITE + 1]) {
    let items = &world.items;
    let Some(shp) = world.shops.get_mut(&shopnum) else {
        return;
    };
    if shp.shop_type != ShopType::DeedShop {
        return;
    }
    let mut dirty = false;
    for slot in shp.stock.iter_mut().take(NSHOPITEMS) {
        if slot.item == ItemNo(0) {
            continue;
        }
        let Some(itm) = items.get(&slot.item) else {
            continue;
        };
        if !item_has_ability(itm, Ability::GangDeed) {
            continue;
        }
        let n = i64::from(item_ability_value(itm, Ability::GangDeed));
        if n > GH_WHITE as i64 {
            continue;
        }
        let Ok(i) = usize::try_from(n) else {
            continue;
        };
        if (i == 0 || gh[i].closed()) && slot.qty != 1 {
            tracing::info!(target: "mmlog",
                "GANGHOUSE : Adding gang house deed for gang house {i} back into deed shop.");
            slot.qty = 1;
            slot.regentime = 0;
            slot.regenpct = 0;
            dirty = true;
            gh[i].flags |= EVICTED;
        }
    }
    if dirty {
        crate::gangs::save_gang_shop(world, shopnum);
    }
}

/// What became of each house, to the activity log. (sic) The tax-paid line is never
/// written (the same always-false test as [`HouseLedger::closed`]).
fn log_houses(gh: &[HouseLedger; GH_WHITE + 1]) {
    for (i, h) in gh.iter().enumerate().skip(1) {
        if h.flags & EVICTED != 0 {
            tracing::info!(target: "mmlog", "GANGHOUSE : HOUSE {i} : Effective eviction flag set.");
        }
        if !h.occupied {
            tracing::info!(target: "mmlog", "GANGHOUSE : HOUSE {i} : Not Occupied.");
        }
    }
}

/// Evil points after one day's forgiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forgiven {
    /// The new `evil`.
    pub evil: i16,
    /// What counts as forgiven (the share of the total; can be negative, sic).
    pub forgiven: i32,
}

/// The forgiveness arithmetic of pass 3, pure. `eps` is a flat 2 when evil points were
/// charged since (`PS_EVILADDED`), else `(mins/epcycle) * epamount + 3`, at most
/// `maxepday`. A character below level 4 is forgiven only while evil and never past 0;
/// from level 4 the points may go negative down to -200, a negative result being pulled
/// back up by 2 (and 2 less counted).
pub fn forgiveness(
    level: u16,
    evil: i16,
    mins: u32,
    evil_added: bool,
    cfg: &silvermere_types::config::GameConfig,
) -> Forgiven {
    let mut eps = if evil_added {
        2
    } else {
        (mins / cfg.ep_cycle.max(1)) as i32 * cfg.ep_amount as i32 + 3
    };
    if (cfg.max_ep_day as i32) < eps {
        eps = cfg.max_ep_day as i32;
    }
    let mut evil = i32::from(evil);
    let mut forgiven = 0;
    if (level < 4 && evil > 0) || level > 3 {
        if evil - eps < -200 {
            if evil > -200 {
                forgiven += evil + 200;
                evil = -200;
            }
        } else {
            evil -= eps;
            forgiven += eps;
            if evil < 0 {
                if level > 3 {
                    evil += 2;
                    forgiven -= 2;
                    evil = evil.clamp(-220, 0);
                } else {
                    evil = 0;
                }
            }
        }
    }
    Forgiven {
        evil: evil as i16,
        forgiven,
    }
}

/// What pass 3 did to one character that its session must see.
#[derive(Debug, Default)]
struct UserOutcome {
    moved: bool,
}

/// Pass 3: every character who played since the last cleanup.
fn users(world: &mut World, offline: &mut [Offline], recs: &[Rec], report: &mut CleanupReport) {
    let gh = report.houses.clone();
    for &rec in recs.iter().take(MAX_WALK) {
        report.examined += 1;
        match rec {
            Rec::Online(id) => {
                let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) else {
                    continue;
                };
                let out = user(world, &mut c, &gh, report);
                if let Some(s) = world.session_mut(id) {
                    s.character = Some(c);
                }
                if let Some(out) = out {
                    online_after(world, id, out);
                }
            }
            Rec::Offline(i) => {
                let mut c = offline[i].character.clone();
                if user(world, &mut c, &gh, report).is_some() {
                    let account = offline[i].account;
                    crate::player::queue_character(world, account, Box::new(c.clone()));
                    offline[i].character = c;
                }
            }
        }
    }
    // the original's console line "%d (%d) users, %s mins, %d EPs forgiven"
    let fields = vec![
        ("users", int(report.users)),
        ("examined", int(report.examined)),
        ("minutes", int(report.minutes as i64)),
        ("evil_forgiven", int(report.evil_forgiven)),
    ];
    audit(world, "cleanup.users", fields);
}

/// One character of pass 3; `None` when it has not played since the last cleanup.
fn user(
    world: &mut World,
    c: &mut Character,
    gh: &[HouseLedger; GH_WHITE + 1],
    report: &mut CleanupReport,
) -> Option<UserOutcome> {
    if c.sesstime == 0 {
        return None;
    }
    report.users += 1;
    // sesstime counts slow cycles, two to the minute
    let mins = c.sesstime / 2;
    report.minutes += u64::from(mins);
    c.sesstime = 0;
    c.lifemins = c.lifemins.wrapping_add(mins);
    let f = forgiveness(
        c.level,
        c.evil,
        mins,
        c.settings.contains(PlayerSettings::EVILADDED),
        &world.config,
    );
    c.evil = f.evil;
    report.evil_forgiven += f.forgiven;
    c.settings.remove(PlayerSettings::EVILADDED);
    // the inventory, then the key ring
    for keys in [false, true] {
        let n = if keys { NKEYS } else { NINVITEMS };
        for i in 0..n {
            let slot = |c: &Character| {
                if keys {
                    c.keys.get(i).copied()
                } else {
                    c.items.get(i).copied()
                }
                .unwrap_or(ItemNo(0))
            };
            let item = slot(c);
            if item == ItemNo(0) {
                continue;
            }
            // "Recharge at cleanup": the copy gets the ability's value as its uses
            let uses = if keys {
                &mut c.keyuses
            } else {
                &mut c.itemuses
            };
            if uses.get(i).is_some_and(|&u| u >= 0) {
                let k = world
                    .items
                    .get(&item)
                    .map_or(0, |itm| item_ability_value(itm, Ability::RechargeCleanup));
                if k != 0 {
                    uses[i] = k as i16;
                }
            }
            // "Delete from inventory at cleanup"
            if world
                .items
                .get(&item)
                .is_some_and(|itm| item_has_ability(itm, Ability::DelInvCleanup))
            {
                remove_item_from_record(world, c, item);
            }
            // the slot is read again: the ability's value is 0 now
            let item = slot(c);
            let k = world.items.get(&item).map_or(0, |itm| {
                i64::from(item_ability_value(itm, Ability::GangItem))
            });
            if k == 0 || k > GH_WHITE as i64 {
                continue;
            }
            let Some(h) = house(gh, k) else {
                continue;
            };
            if h.closed() {
                remove_item_from_record(world, c, item);
                if !h.leader.is_empty() && eq_ignore_case(&h.leader, &c.userid) {
                    c.flags |= PlayerFlags::HOUSECLOSED;
                }
                if !h.gang.is_empty() && eq_ignore_case(&h.gang, &c.gang) {
                    c.flags |= PlayerFlags::HOUSEITEMS;
                }
            }
            // (sic) a second removal of the same item number, which finds a copy only
            // when the slot still holds one
            if !eq_ignore_case(&c.gang, &h.gang) {
                let item = slot(c);
                if item != ItemNo(0) {
                    remove_item_from_record(world, c, item);
                }
            }
        }
    }
    c.lastmon = 0;
    c.flags.remove(PlayerFlags::SOLDDEED);
    // out of a closed gang house, to map 1
    let mut out = UserOutcome::default();
    let here = RoomKey {
        map: c.map,
        room: c.room,
    };
    if let Some(rm) = world.rooms.get(&here)
        && rm.flags.contains(RoomFlags::GANGHOUSE)
    {
        let n = i64::from(rm.ganghouse.code());
        if n != 0
            && let Some(h) = house(gh, n)
            && h.closed()
        {
            c.map = MapId(1);
            c.room = RoomId(if c.evil >= 40 {
                world.config.evil_reinc_room
            } else {
                world.config.reinc_room
            });
            out.moved = true;
        }
    }
    Some(out)
}

/// What an online character sees of pass 3, and its save: the house notices that the
/// next entry would have shown (`load_player`) at once, and the room it was moved to. A
/// move ends dragging and following as a teleport does and breaks off combat.
fn online_after(world: &mut World, id: SessionId, out: UserOutcome) {
    // At the menu nothing is shown: the notices wait for the next entry, as offline.
    if world.is_in_game(id) {
        let mut notices = Vec::new();
        if let Some(c) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            if c.flags.contains(PlayerFlags::HOUSECLOSED) {
                c.flags -= PlayerFlags::HOUSECLOSED;
                notices.push(Msg::EnterHouseClosed);
            }
            if c.flags.contains(PlayerFlags::HOUSEITEMS) {
                c.flags -= PlayerFlags::HOUSEITEMS;
                notices.push(Msg::EnterHouseItems);
            }
        }
        if out.moved || !notices.is_empty() {
            for n in notices {
                crate::tell::tell_user(world, id, n);
            }
            if out.moved {
                end_party_and_combat(world, id);
                let verbose = world
                    .session(id)
                    .and_then(|s| s.player())
                    .map_or(0, |p| p.verbose);
                display_room(world, id, RoomMode::verbose(verbose));
            }
            if let Some(s) = world.session_mut(id) {
                s.push_prompt();
            }
            world.touch(id);
        }
    }
    crate::player::save_player(world, id);
}

/// A move out of a closed house ends dragging and following as a teleport does and breaks
/// off combat.
fn end_party_and_combat(world: &mut World, id: SessionId) {
    let leader = world.session(id).and_then(|s| s.party.leader);
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
    }
    if leader.is_some() {
        crate::movement::move_user::stop_following(world, leader, id);
    }
    let followers: Vec<SessionId> = world
        .session_ids()
        .into_iter()
        .filter(|&f| world.session(f).is_some_and(|s| s.party.leader == Some(id)))
        .collect();
    for f in followers {
        crate::movement::move_user::stop_following(world, Some(id), f);
    }
    crate::combat::kill_autocombat(world, id);
    crate::combat::autocombat::kill_autocombat_against(world, id);
}

/// Pass 4: gangs with no members left, or marked for deletion, are deleted.
pub fn delete_gangs(world: &mut World) -> u32 {
    let mut keys: Vec<String> = world.gangs.keys().cloned().collect();
    keys.sort();
    let mut n = 0;
    for key in keys.into_iter().take(MAX_WALK) {
        let Some(g) = world.gangs.get(&key) else {
            continue;
        };
        if g.members == 0 || g.flags.contains(GangFlags::DELETE) {
            world.gangs.remove(&key);
            world
                .outbox
                .push(EngineOutput::Persist(PersistRequest::DeleteGang(key)));
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use silvermere_types::config::GameConfig;

    fn f(level: u16, evil: i16, mins: u32) -> (i16, i32) {
        let r = forgiveness(level, evil, mins, false, &GameConfig::default());
        (r.evil, r.forgiven)
    }

    // epcycle 60, epamount 1, maxepday 12
    #[test]
    fn forgiveness_table() {
        // (level, evil, minutes) -> (evil, forgiven)
        type Row = ((u16, i16, u32), (i16, i32));
        let table: &[Row] = &[
            // eps = mins/60 + 3, at most 12
            ((5, 50, 120), (45, 5)),
            ((20, 60, 0), (57, 3)),
            ((20, 60, 600), (48, 12)),
            ((4, 30, 539), (19, 11)),
            // below 4: only while evil, never past 0 (and the whole eps counts, sic)
            ((1, 60, 600), (48, 12)),
            ((3, 5, 600), (0, 12)),
            ((3, 0, 600), (0, 0)),
            ((1, -50, 600), (-50, 0)),
            // from 4: through 0, pulled back up by 2
            ((4, 1, 0), (0, 1)),
            ((4, 3, 600), (-7, 10)),
            ((20, -100, 600), (-110, 10)),
            // the -200 floor: what is left above it is forgiven
            ((4, -199, 0), (-200, 1)),
            ((20, -195, 600), (-200, 5)),
            ((20, -200, 600), (-200, 0)),
            // at or under the floor nothing changes, down to -220 and past it
            ((20, -210, 600), (-210, 0)),
            ((20, -230, 600), (-230, 0)),
        ];
        for &((level, evil, mins), want) in table {
            assert_eq!(
                f(level, evil, mins),
                want,
                "level {level} evil {evil} {mins} min"
            );
        }
    }

    // levels 1, 3, 4 and 20 over EP -230..=60 and 0..=600 minutes
    #[test]
    fn forgiveness_bounds() {
        let cfg = GameConfig::default();
        for level in [1u16, 3, 4, 20] {
            for evil in -230i16..=60 {
                for mins in (0u32..=600).step_by(30) {
                    let r = forgiveness(level, evil, mins, false, &cfg);
                    if level < 4 {
                        if evil > 0 {
                            assert!(r.evil >= 0 && r.evil <= evil);
                        } else {
                            assert_eq!(r.evil, evil);
                        }
                    } else if evil > -200 {
                        assert!(r.evil >= -200 && r.evil <= evil.max(0));
                    } else {
                        assert_eq!(r.evil, evil);
                    }
                    assert!(r.forgiven <= cfg.max_ep_day as i32);
                }
            }
        }
    }

    // evil points charged since: a flat 2
    #[test]
    fn evil_added_forgives_two() {
        let r = forgiveness(5, 50, 600, true, &GameConfig::default());
        assert_eq!((r.evil, r.forgiven), (48, 2));
    }

    // closed: evicted or not occupied; the tax-paid bit is never read
    #[test]
    fn house_closed() {
        let mut h = HouseLedger::default();
        assert!(h.closed());
        h.occupied = true;
        assert!(!h.closed());
        h.flags = TAX_PAID;
        assert!(!h.closed());
        h.flags = EVICTED | TAX_PAID;
        assert!(h.closed());
    }

    // `game.cleanup.at`, local time: the next one strictly after now
    #[test]
    fn next_run() {
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(seconds_until(0, (3, 0), &utc), 3 * 3600);
        assert_eq!(seconds_until(3 * 3600, (3, 0), &utc), 86_400);
        assert_eq!(seconds_until(3 * 3600 + 1, (3, 0), &utc), 86_399);
        assert_eq!(seconds_until(23 * 3600, (0, 30), &utc), 5400);
        // a fixed offset: 03:00 at UTC+2 is 01:00 UTC
        let plus2 = jiff::tz::TimeZone::fixed(jiff::tz::offset(2));
        assert_eq!(seconds_until(0, (3, 0), &plus2), 3600);
    }

    // the default zone, a day on each side of the 2026 DST change (Mar 8, 02:00 EST -> EDT)
    #[test]
    fn next_run_detroit_dst() {
        let det = jiff::tz::TimeZone::get("America/Detroit").unwrap();
        let ts = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_second();
        // Mar 7 12:00 EST (17:00Z): 03:00 on the 8th is already EDT (07:00Z), 14h
        assert_eq!(
            seconds_until(ts("2026-03-07T17:00:00Z"), (3, 0), &det),
            14 * 3600
        );
        // Mar 6 12:00 EST: 03:00 on the 7th is EST (08:00Z), 15h
        assert_eq!(
            seconds_until(ts("2026-03-06T17:00:00Z"), (3, 0), &det),
            15 * 3600
        );
        // Mar 8 12:00 EDT (16:00Z): 03:00 EDT next day, 15h
        assert_eq!(
            seconds_until(ts("2026-03-08T16:00:00Z"), (3, 0), &det),
            15 * 3600
        );
    }
}
