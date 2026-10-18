//! The sysop's reports and lists (T-055): the room status ([`display_debug_room_stats`]),
//! the map ([`print_map`]), the AV/DV line ([`display_attack_values`]), the limited-item and
//! database lists, the sysop's WHO ([`display_scan_of_users`] modes 1 and 2), the paged
//! user report ([`display_sysop_user_report`]), the abilities line and the restock list.

use silvermere_rules::combat::{
    AlignGroup, Blow, Move, PlayerAbilities, player_fighter, user_align_group,
};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, ExitType, OwnerKind, RoomType, SessionState};
use silvermere_types::flags::{PlayerSettings, RoomFlags};
use silvermere_types::ids::{
    AccountId, MapId, RoomId, RoomKey, SessionId, ShopNo, SpellNo, TextBlockNo,
};
use silvermere_types::names::MONSTER_TYPES;
use silvermere_types::records::Character;
use silvermere_types::sizes::NDIRS;

use crate::commands::CommandLine;
use crate::commands::parse::starts_with_ignore_case;
use crate::display::who::players_in_list_order;
use crate::items::inventory::ability_value;
use crate::movement::HOOKS;
use crate::tell::tell_user;
use crate::world::World;

fn push(world: &mut World, id: SessionId, msg: Msg) {
    tell_user(world, id, msg);
}

fn line(world: &mut World, id: SessionId, text: String) {
    push(world, id, Msg::SysopText { text });
}

/// `%-w.ws`: cut to `w`, padded on the right.
fn left(s: &str, w: usize) -> String {
    let cut: String = s.chars().take(w).collect();
    format!("{cut:<w$}")
}

/// `MM/DD/YY` of Unix seconds (UTC).
pub fn format_date(unix: i64) -> String {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{month:02}/{day:02}/{:02}", year.rem_euclid(100))
}

/// `HH:MM:SS` of Unix seconds (UTC).
pub fn format_time(unix: i64) -> String {
    let s = unix.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// The player's fighter figures for a normal blow (`atktype = atkmove = 5`) against the
/// player's own alignment group, shown to `to`.
pub fn display_attack_values(world: &mut World, shown: SessionId, to: SessionId) {
    let Some(plr) = world.session(shown).and_then(|s| s.player()) else {
        return;
    };
    let Some(class) = world.classes.get(&plr.class) else {
        return;
    };
    let weapon = world.items.get(&plr.weapon);
    let armour: Vec<_> = plr.worn.iter().filter_map(|n| world.items.get(n)).collect();
    let abilities = PlayerAbilities {
        dodge: ability_value(world, plr, Ability::Dodge),
        def_vs_good: ability_value(world, plr, Ability::DefVsGood),
        def_vs_evil: ability_value(world, plr, Ability::DefVsEvil),
        shadow_stealth: ability_value(world, plr, Ability::ShadowStealth) != 0,
        ..PlayerAbilities::default()
    };
    let style: AlignGroup = user_align_group(plr.evil);
    let f = player_fighter(
        plr,
        class,
        weapon,
        &armour,
        abilities,
        Blow::new(Move::Attack, style),
    );
    // "AV %-5d DV %-5d " "DR %-5d DG %-4d " "EU %-5d D %5d/%-5d CR %3d\r"
    let msg = Msg::SysopAttackValues {
        av: format!("{:<5} ", f.av),
        dv: format!("{:<5} ", f.dv),
        dr: format!("{:<5} ", f.dr),
        dg: format!("{:<4} ", f.dodge),
        eu: format!("{:<5} ", f.energy as u16 as i16),
        min: format!("{:>5}", f.mindmg),
        max: format!("{:<5} ", f.maxdmg),
        cr: format!("{:>3}", f.crit),
    };
    push(world, to, msg);
}

/// A room's numbers, area and generation settings, specific monster, flags, monsters,
/// items, hidden and placed items and a shop's pending restocks. Room 0 / map 0 is where
/// the sysop stands.
pub fn display_debug_room_stats(world: &mut World, id: SessionId, room: u32, map: u32) {
    let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let key = RoomKey {
        room: if room == 0 { here.room } else { RoomId(room) },
        map: if map == 0 { here.map } else { MapId(map) },
    };
    let (Some(rm), Some(st)) = (
        world.room(key).cloned(),
        world.room_state.get(&key).cloned(),
    ) else {
        push(world, id, Msg::SysopRoomError);
        return;
    };
    let mut out = vec![
        format!("Room {}  Map: {}", rm.key.room.0, rm.key.map.0),
        format!(
            "This room as Area: Max: {}  Current: {}",
            rm.areamax, st.areacount
        ),
        format!(
            "Min: {} Max: {} Group: {} by Number: {}",
            rm.minlevel,
            rm.maxlevel,
            usize::try_from(rm.mongroup)
                .ok()
                .and_then(|g| MONSTER_TYPES.get(g))
                .copied()
                .unwrap_or(""),
            rm.genmonster.0
        ),
        format!(
            "Room Max: {}  Current: {}  Last Killed: {} Delay: {}",
            rm.maxmonsters,
            st.nummonsters,
            format_time(i64::from(st.lastkill) * 60),
            rm.regendelay
        ),
    ];
    if rm.ctlroom.0 != 0 {
        out.push(format!("Controlling Room: {}", rm.ctlroom.0));
        let ctl = RoomKey {
            map: rm.key.map,
            room: rm.ctlroom,
        };
        if let (Some(c), Some(cs)) = (world.room(ctl), world.room_state.get(&ctl)) {
            out.push(format!(
                "Current Area: Max: {}  Current: {}",
                c.areamax, cs.areacount
            ));
        }
    } else {
        out.push("No controlling room.".into());
    }
    for text in out {
        line(world, id, text);
    }
    if rm.specmon.0 != 0
        && let Some(k) = world.monsters.get(&rm.specmon)
    {
        // (sic) the first part has no line end
        let text = format!(
            "Specific Monster: {}-{} [{}/{}]Last Killed: {} {} (RG: {})",
            rm.specmon.0,
            k.name,
            k.curalive,
            k.maxalive,
            format_date(k.killed_at),
            format_time(k.killed_at),
            k.regenhrs
        );
        line(world, id, text);
    }
    for (flag, text) in [
        (rm.flags.contains(RoomFlags::PROTECTED), "Protected"),
        (rm.flags.contains(RoomFlags::PATROL), "Patrollable"),
        (rm.flags.contains(RoomFlags::GANGHOUSE), "Ganghouse"),
        (st.specalive, "Specific Monster is Alive"),
    ] {
        if flag {
            line(world, id, text.into());
        }
    }
    let list = |head: &str, parts: Vec<String>, always: bool| -> Option<String> {
        if parts.is_empty() {
            always.then(|| format!("{head}None"))
        } else {
            Some(format!("{head}{} ", parts.join(" ")))
        }
    };
    let monsters: Vec<String> = st
        .monsters
        .iter()
        .filter(|m| m.0 != 0)
        .map(|m| m.0.to_string())
        .collect();
    let items: Vec<String> = st
        .items
        .iter()
        .filter(|i| i.item.0 != 0)
        .map(|i| format!("{}({})", i.item.0, i.count))
        .collect();
    let hidden: Vec<String> = st
        .hiditems
        .iter()
        .filter(|i| i.item.0 != 0)
        .map(|i| format!("{}({})", i.item.0, i.count))
        .collect();
    let placed: Vec<String> = rm
        .placed
        .iter()
        .filter(|i| i.0 != 0)
        .map(|i| i.0.to_string())
        .collect();
    for text in [
        list("Monsters: ", monsters, true),
        list("Items: ", items, true),
        list("Hidden items: ", hidden, true),
        list("Placed items: ", placed, false),
    ]
    .into_iter()
    .flatten()
    {
        line(world, id, text);
    }
    // A shop of the usual types: its pending restocks
    if rm.shopnum.0 != 0
        && world.shops.get(&rm.shopnum).is_some_and(|s| {
            use silvermere_types::enums::ShopType as T;
            matches!(
                s.shop_type,
                T::General
                    | T::Weapons
                    | T::Armour
                    | T::Items
                    | T::Spells
                    | T::Hospital
                    | T::Tavern
                    | T::Bank
                    | T::Training
                    | T::Inn
                    | T::Specific
            )
        })
    {
        display_all_restocking_required(world, id, i64::from(rm.shopnum.0));
    }
}

/// The heading and one line per pending restock of the shop (every shop for 0): slot, due
/// date and time, and the slot's item when one shop is listed; at most 2000 nodes looked
/// at, then `Total I2R`. The due tick is shown as wall time (now plus the ticks still to
/// go).
pub fn display_all_restocking_required(world: &mut World, id: SessionId, shop: i64) {
    push(world, id, Msg::SysopRestockHeading { shop });
    let filter = (shop != 0).then_some(ShopNo(shop as i32));
    let shp = filter.and_then(|s| world.shops.get(&s)).cloned();
    let now = world.scheduler.tick();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // MAXDISPLAY counts every node looked at, listed or not
    let nodes: Vec<_> = crate::shops::pending_restocks(world, None)
        .into_iter()
        .take(2000)
        .filter(|n| filter.is_none_or(|s| n.shop == s))
        .collect();
    let found = nodes.len();
    for n in nodes {
        let at = wall + (n.due.saturating_sub(now)) as i64;
        let item = shp
            .as_ref()
            .and_then(|s| s.stock.get(n.slot))
            .map_or_else(String::new, |slot| format!("({})", slot.item.0 as i16));
        line(
            world,
            id,
            format!(
                "     [{}] {} {} {item}",
                n.slot,
                format_date(at),
                format_time(at)
            ),
        );
    }
    if found > 0 {
        push(world, id, Msg::SysopRestockTotal { n: found as i64 });
    }
}

/// The record's ability slots as `number(value)` pairs.
pub fn display_user_abilities(world: &mut World, shown: SessionId, to: SessionId) {
    let Some(plr) = world.session(shown).and_then(|s| s.player()) else {
        return;
    };
    let list: String = plr
        .abils
        .iter()
        .filter(|(a, _)| *a != Ability::None)
        .map(|(a, v)| format!("{}({}) ", a.code(), v))
        .collect();
    let list = if list.is_empty() {
        "Nothing special".to_string()
    } else {
        list
    };
    push(world, to, Msg::SysopUserAbilities { list });
}

/// WCCTEXT block 1510 run as a special command for the player (the jail script), through
/// T-054's `run_text_block`.
pub fn perform_sysop_text_block(world: &mut World, t: SessionId) {
    crate::scripts::special::run_text_block(world, t, SYSOP_TEXT_BLOCK);
}

/// The sysop's text block.
pub const SYSOP_TEXT_BLOCK: TextBlockNo = TextBlockNo(1510);

/// Every copy of a limited item and where it is, at most 1000.
///
/// A shop-shelf copy (T-041) is an `OWN_ROOM` record of the shop room (user-id `#SHELF`)
/// and prints as the original printed it: `Item X Map m Room r`, like a copy on the floor.
pub fn display_list_of_limited_items(world: &mut World, id: SessionId) {
    let mut rows: Vec<_> = world.item_ownership.values().cloned().collect();
    rows.sort_by_key(|o| o.id);
    for own in rows.into_iter().take(1000) {
        let Some(item) = world.items.get(&own.itemnum).map(|i| i.name.clone()) else {
            push(world, id, Msg::SysopNoItemError);
            continue;
        };
        let msg = match own.owntype {
            OwnerKind::Room => Msg::SysopLimitedRoom {
                item,
                map: i64::from(own.mapnum.0 as u16 as i16),
                room: i64::from(own.roomnum.0 as u16 as i16),
            },
            OwnerKind::User => Msg::SysopLimitedUser {
                item,
                user: own.userid.clone(),
            },
            OwnerKind::Monster => Msg::SysopLimitedMonster {
                item,
                mon: i64::from(own.monnum.0 as u16 as i16),
            },
            _ => Msg::SysopError,
        };
        push(world, id, msg);
    }
}

/// The copies sorted by item name, filtered by the words after NEWLIMITED (MAP [n], GANG
/// [name], USER, MONSTER, NAME item), each selecting the rows the original lists. The item
/// name is looked up by the item number.
pub fn display_new_list_of_limited_items(world: &mut World, id: SessionId, cmd: &CommandLine) {
    let margc = cmd.argc();
    let (w3, w4) = (cmd.arg(3), cmd.arg(4));
    let mut rows: Vec<(String, silvermere_types::records::ItemOwnership)> = world
        .item_ownership
        .values()
        .filter_map(|o| {
            world
                .items
                .get(&o.itemnum)
                .map(|i| (i.name.clone(), o.clone()))
        })
        .collect();
    // the insertion sort keeps equal names in reading order
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()).then(a.1.id.cmp(&b.1.id)));
    let mut out = Vec::new();
    for (name, own) in rows {
        let kind = own.owntype;
        let pass = margc == 3
            || (margc == 4
                && kind == OwnerKind::Room
                && (starts_with_ignore_case(w3, "map") || starts_with_ignore_case(w3, "gang")))
            || (kind == OwnerKind::User && starts_with_ignore_case(w3, "user"))
            || (kind == OwnerKind::Monster && starts_with_ignore_case(w3, "monster"))
            || (margc == 5
                && kind == OwnerKind::Room
                && (starts_with_ignore_case(w3, "gang")
                    || (starts_with_ignore_case(w3, "map")
                        && super::super::commands::sysop::leading_int(w4) as i32 as i16
                            == own.mapnum.0 as i16)))
            || (starts_with_ignore_case(w3, "name") && starts_with_ignore_case(w4, &name));
        if !pass {
            continue;
        }
        match kind {
            OwnerKind::Room => {
                let key = RoomKey {
                    map: own.mapnum,
                    room: own.roomnum,
                };
                let gh = world.room(key).map(|r| r.ganghouse);
                let ghname = gh
                    .filter(|g| g.code() != 0)
                    .and_then(|g| g.name())
                    .map(str::to_string);
                let text = match &ghname {
                    Some(g) => format!(
                        "Item {name} Map {} Room {} ({g})",
                        own.mapnum.0, own.roomnum.0
                    ),
                    None => format!("Item {name} Map {} Room {}", own.mapnum.0, own.roomnum.0),
                };
                let again = margc == 3
                    || (margc == 4
                        && (starts_with_ignore_case(w3, "map")
                            || (starts_with_ignore_case(w3, "gang") && ghname.is_some())))
                    || (margc == 5
                        && ((starts_with_ignore_case(w3, "map")
                            && super::super::commands::sysop::leading_int(w4) as i32 as i16
                                == own.mapnum.0 as i16)
                            || (starts_with_ignore_case(w3, "gang")
                                && ghname
                                    .as_deref()
                                    .is_some_and(|g| starts_with_ignore_case(w4, g)))));
                if again {
                    out.push(text);
                }
            }
            OwnerKind::User => out.push(format!("Item {name} User {}", own.userid)),
            OwnerKind::Monster => {
                let mname = world
                    .active_monsters
                    .values()
                    .find(|m| m.number == own.monnum)
                    .map_or("Unknown Monster", |m| m.name.as_str())
                    .to_string();
                out.push(format!("Item {name} Monster {} ({mname})", own.monnum.0));
            }
            // OWN_UNKNOWN is never set; it prints nothing
            _ => {}
        }
    }
    for text in out {
        line(world, id, text);
    }
    line(world, id, String::new());
}

/// Number, type and name of every item, then the two trace lines the author left in. The
/// debug file `nrddebug.out` is not written. (The original's columns come out garbled,
/// sic; the numbers are printed as they are.)
pub fn display_list_of_all_items(world: &mut World, id: SessionId) {
    let mut items: Vec<_> = world
        .items
        .values()
        .map(|i| {
            (
                i.number.0,
                i.item_type.display_name().unwrap_or("").to_string(),
                i.name.clone(),
            )
        })
        .collect();
    items.sort_by_key(|i| i.0);
    for (n, t, name) in items {
        line(world, id, format!("{n}: {} - {name}", left(&t, 11)));
    }
    line(world, id, "Just before dfaRstBlk();".into());
    line(world, id, "Just after dfaRstBlk();".into());
}

/// The active monsters numbered from `first`, at most `max`.
pub fn display_list_of_all_active_monsters(world: &mut World, id: SessionId, first: i64, max: i64) {
    let mut mons: Vec<_> = world
        .active_monsters
        .values()
        .filter(|m| i64::from(m.number.0) >= first)
        .map(|m| {
            (
                m.number.0,
                m.name.clone(),
                m.room.0,
                m.map.0,
                m.following.clone(),
            )
        })
        .collect();
    mons.sort_by_key(|m| m.0);
    let mut cnt = 0i64;
    for (n, name, room, map, following) in mons {
        if cnt >= max {
            break;
        }
        cnt += 1;
        line(
            world,
            id,
            format!("{n:>5}) {name} [{room}/{map}] {{{following}}}"),
        );
    }
    push(
        world,
        id,
        Msg::SysopActiveMonstersTotal {
            atleast: if cnt >= max {
                "At least ".into()
            } else {
                String::new()
            },
            n: cnt,
        },
    );
}

/// Every known monster.
pub fn display_list_of_all_known_monsters(world: &mut World, id: SessionId) {
    let mut mons: Vec<_> = world
        .monsters
        .values()
        .map(|m| (m.number.0, m.name.clone(), m.level, m.genindex))
        .collect();
    mons.sort_by_key(|m| m.0);
    for (n, name, level, genindex) in mons {
        let num: String = n.to_string().chars().take(6).collect();
        let name: String = name.chars().take(30).collect();
        line(
            world,
            id,
            format!("{num:>6} {name:>30} - Lvl {} Gen# {genindex}", level as i16),
        );
    }
}

/// Every spell's name.
pub fn display_list_of_all_spells(world: &mut World, id: SessionId) {
    let mut spells: Vec<(SpellNo, String)> = world
        .spells
        .values()
        .map(|s| (s.number, s.name.clone()))
        .collect();
    spells.sort_by_key(|s| s.0.0);
    for (_, name) in spells {
        line(world, id, name);
    }
}

/// The player list in the sysop's modes: 1 every player with user-id, room/map, evil and
/// `A` in combat; 2 adds the evil timers.
pub fn display_scan_of_users(world: &mut World, id: SessionId, mode: u8) {
    let mut first = true;
    let mut cnt = 0;
    for pid in players_in_list_order(world) {
        let Some(p) = world.session(pid).and_then(|s| s.player()).cloned() else {
            continue;
        };
        if first {
            push(world, id, Msg::SysopScanHeader);
            first = false;
        }
        let combat = if HOOKS.combat.in_autocombat(world, pid) {
            'A'
        } else {
            ' '
        };
        let msg = Msg::SysopScanRow {
            userid: left(&p.userid, 16),
            name: left(&format!("{} {}", p.firstname, p.lastname), 20),
            bcst: format!("{:<4}", p.bchannel),
            roommap: format!(
                "{:>5}/{}",
                left(&p.room.0.to_string(), 5).trim_end(),
                left(&p.map.0.to_string(), 3)
            ),
            evil: left(&p.evil.to_string(), 6),
            combat,
            edited: left(if p.immortal != 0 { "EDITED" } else { " " }, 6),
        };
        push(world, id, msg);
        // mode 2: the evil timers. TODO(T-038): the evil timers.
        let _ = mode;
        cnt += 1;
    }
    push(
        world,
        id,
        if first {
            Msg::SysopScanNone
        } else {
            Msg::SysopScanCount { n: cnt }
        },
    );
}

/// The paging state of a REPORT USERS / NOTOPTEN (`listkey`, `rptname`, `USF_REPORT`): the
/// characters in experience order once `bin` has sent them, and the place reached.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// 0 every character (or those whose names hold the filter), 1 those left out of
    /// TOPTEN.
    pub mode: u8,
    /// At most 19 characters.
    pub filter: String,
    /// The rows still to show: (user-id, first name, last name, gang, settings, immortal).
    pub rows: Option<std::collections::VecDeque<Character>>,
}

/// The heading, the paging state, `SUBST_USER_REPORT`, and the stored characters asked for
/// (`EngineOutput::Roster`); the lines follow a tick at a time ([`next_reported_user`]).
pub fn display_sysop_user_report(
    world: &mut World,
    id: SessionId,
    mode: u8,
    filter: Option<String>,
) {
    push(
        world,
        id,
        if mode == 1 {
            Msg::SysopReportNotoptenHeading
        } else {
            Msg::SysopReportAllHeading
        },
    );
    let filter: String = filter.unwrap_or_default().chars().take(19).collect();
    world.sysop.reports.insert(
        id,
        Report {
            mode,
            filter,
            rows: None,
        },
    );
    if let Some(s) = world.session_mut(id) {
        s.state = SessionState::UserReport;
    }
    world
        .outbox
        .push(crate::event_loop::EngineOutput::Roster { id });
}

/// `contains_ignore_case(sub, s)`: `sub` appears in `s`, ignoring case.
pub(crate) fn contains_ignore_case(sub: &str, s: &str) -> bool {
    s.to_ascii_uppercase().contains(&sub.to_ascii_uppercase())
}

/// The next character in experience order, lowest first; at the end the report stops with
/// the prompt. `false` while the characters have not arrived, or when the report is over.
pub fn next_reported_user(world: &mut World, id: SessionId) -> bool {
    let Some(report) = world.sysop.reports.get_mut(&id) else {
        return false;
    };
    let Some(rows) = report.rows.as_mut() else {
        return false;
    };
    let Some(plr) = rows.pop_front() else {
        world.sysop.reports.remove(&id);
        if let Some(s) = world.session_mut(id) {
            s.state = SessionState::Playing;
            s.push_prompt();
        }
        return false;
    };
    let name = left(&format!("{} {}", plr.firstname, plr.lastname), 25);
    let userid = left(&plr.userid, 29);
    let msg = match report.mode {
        1 => (plr.settings.contains(PlayerSettings::NOTOPTEN) || plr.immortal != 0).then(|| {
            Msg::SysopReportRow {
                userid,
                name,
                extra: if plr.immortal != 0 {
                    "(Editted)".into()
                } else {
                    String::new()
                },
            }
        }),
        _ => (report.filter.is_empty()
            || contains_ignore_case(&report.filter, &plr.userid)
            || contains_ignore_case(&report.filter, &plr.firstname)
            || contains_ignore_case(&report.filter, &plr.lastname))
        .then(|| Msg::SysopReportRow {
            userid,
            name,
            extra: plr.gang.clone(),
        }),
    };
    if let Some(m) = msg {
        push(world, id, m);
    }
    true
}

/// The characters for a report, in the order key 2 of WCCUSERS gives (experience, lowest
/// first; equal keys in insertion order, here the account id).
pub fn report_order(
    mut chars: Vec<(AccountId, Character)>,
) -> std::collections::VecDeque<Character> {
    chars.sort_by(|a, b| {
        (a.1.expbil, a.1.exprem, a.1.exp)
            .cmp(&(b.1.expbil, b.1.exprem, b.1.exp))
            .then(a.0.0.cmp(&b.0.0))
    });
    chars.into_iter().map(|(_, c)| c).collect()
}

/// The map's width and height in squares, and its depth limit.
const MAPWIDTH: usize = 18;
const MAPHEIGHT: usize = 8;
const MAXMAPDEPTH: i32 = 8;

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    room: u32,
    exits: [u32; NDIRS],
    symbol: u8,
}

struct MapWalk<'a> {
    world: &'a World,
    map: MapId,
    cells: [[Cell; MAPHEIGHT]; MAPWIDTH],
    depth: i32,
}

impl MapWalk<'_> {
    /// The room into its square, then its eight compass neighbours, N NE E SE S SW W NW, up
    /// to the depth limit. Secret exits (spell, item, hidden, trap, timed) are drawn for a
    /// developer (`devmode`, the DEBUG key here); remote-action exits never.
    fn process(&mut self, room: u32, x: i32, y: i32, dev: bool) {
        let d = self.depth;
        self.depth += 1;
        if d <= MAXMAPDEPTH
            && x >= 0
            && y >= 0
            && (x as usize) < MAPWIDTH
            && (y as usize) < MAPHEIGHT
        {
            let (xu, yu) = (x as usize, y as usize);
            if self.cells[xu][yu].room == 0 && room != 0 {
                let key = RoomKey {
                    map: self.map,
                    room: RoomId(room),
                };
                if let Some(rm) = self.world.room(key) {
                    let mut cell = Cell {
                        room: rm.key.room.0,
                        ..Cell::default()
                    };
                    for dir in 0..NDIRS {
                        let e = &rm.exits[dir];
                        let hide = match e.kind {
                            ExitType::Spell
                            | ExitType::Item
                            | ExitType::Hidden
                            | ExitType::Trap
                            | ExitType::Timed => !dev,
                            ExitType::RemoteAction => true,
                            _ => false,
                        };
                        cell.exits[dir] = if hide { 0 } else { e.target.0 };
                    }
                    cell.symbol = match rm.room_type {
                        RoomType::Shop => b'S',
                        RoomType::Arena => b'A',
                        RoomType::Lair => b'L',
                        RoomType::Hotel => b'H',
                        RoomType::Colliseum => b'C',
                        _ => b'*',
                    };
                    self.cells[xu][yu] = cell;
                    // DIR_ order: N S E W NE NW SE SW U D
                    for (dir, dx, dy) in [
                        (0, 0, -1),
                        (4, 1, -1),
                        (2, 1, 0),
                        (6, 1, 1),
                        (1, 0, 1),
                        (7, -1, 1),
                        (3, -1, 0),
                        (5, -1, -1),
                    ] {
                        let next = self.cells[xu][yu].exits[dir];
                        self.process(next, x + dx, y + dy, dev);
                    }
                }
            }
        }
        self.depth -= 1;
    }
}

/// The rooms around `room` in a grid, three text lines per row. The file name was never
/// used; the map goes to the screen only.
pub fn print_map(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    numbers: bool,
    width: usize,
    height: usize,
) {
    let width = if width == 0 || width > MAPWIDTH {
        MAPWIDTH
    } else {
        width
    };
    let height = if height == 0 || height > MAPHEIGHT {
        MAPHEIGHT
    } else {
        height
    };
    let (cx, cy) = (width / 2, height / 2);
    let dev = world
        .session(id)
        .is_some_and(|s| s.sysop_keys.contains(crate::commands::sysop::Key::Debug));
    let mut walk = MapWalk {
        world,
        map: room.map,
        cells: [[Cell::default(); MAPHEIGHT]; MAPWIDTH],
        depth: 0,
    };
    walk.process(room.room.0, cx as i32, cy as i32, dev);
    let cells = walk.cells;
    let mark = |b: bool, c: char| if b { c } else { ' ' };
    let mut text = String::new();
    for y in 0..height {
        for col in cells.iter().take(width) {
            let c = &col[y];
            let (nw, n, ne) = (
                mark(c.exits[5] != 0, '\\'),
                mark(c.exits[0] != 0, '|'),
                mark(c.exits[4] != 0, '/'),
            );
            if numbers {
                text.push_str(&format!("{nw}   {n}   {ne} "));
            } else {
                text.push_str(&format!("{nw}{n}{ne}"));
            }
        }
        text.push('\n');
        for (x, col) in cells.iter().enumerate().take(width) {
            let c = &col[y];
            if c.room != 0 {
                let (w, e) = (mark(c.exits[3] != 0, '-'), mark(c.exits[2] != 0, '-'));
                if numbers {
                    text.push_str(&format!("{w}>{:<5}<{e} ", c.room as u16 as i16));
                } else {
                    let sym = if cx == x && cy == y {
                        'X'
                    } else {
                        c.symbol as char
                    };
                    text.push_str(&format!("{w}{sym}{e}"));
                }
            } else {
                text.push_str(if numbers { "          " } else { "   " });
            }
        }
        text.push('\n');
        for col in cells.iter().take(width) {
            let c = &col[y];
            let (sw, s, se) = (
                mark(c.exits[7] != 0, '/'),
                mark(c.exits[1] != 0, '|'),
                mark(c.exits[6] != 0, '\\'),
            );
            if numbers {
                text.push_str(&format!("{sw}   {s}   {se} "));
            } else {
                text.push_str(&format!("{sw}{s}{se}"));
            }
        }
        text.push('\n');
    }
    push(world, id, Msg::SysopRaw { text });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(format_date(0), "01/01/70");
        assert_eq!(format_time(3661), "01:01:01");
    }

    #[test]
    fn padding() {
        assert_eq!(left("abcdef", 4), "abcd");
        assert_eq!(left("ab", 4), "ab  ");
    }

    #[test]
    fn report_order_is_lowest_experience_first() {
        let mut a = crate::player_session::clear_player();
        a.exp = 50;
        a.userid = "A".into();
        let mut b = crate::player_session::clear_player();
        b.exp = 10;
        b.userid = "B".into();
        let order = report_order(vec![(AccountId(1), a), (AccountId(2), b)]);
        assert_eq!(order[0].userid, "B");
    }
}
