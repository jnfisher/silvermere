//! `--dry-run`'s value checks: does every table decode to values that make sense?
//!
//! A record decoded with the wrong layout still decodes; its values are what give it away.
//! [`checks`] counts, per table, the records whose values fail a plain sanity check (a name
//! that is empty or holds control characters, an exit to no room, a character's level or
//! stats out of range, an item owner's item that is not an item), and compares each active
//! monster's members copied from its known monster (when it is generated, and again each time
//! the game loads it, so a realm's records agree), member by member, in record order: a layout whose offsets go wrong from some member on shows as the
//! agreement dropping from that member on. The lines go into the import report.

use std::collections::{HashMap, HashSet};

use crate::decode::{ActmonRecord, MonsterRecord, Text};
use crate::importer::{Content, Decoded};

/// A name that is empty, or holds a control character or the replacement character.
fn unreadable(s: &str) -> bool {
    let s = s.trim();
    s.is_empty() || s.chars().any(|c| c.is_control() || c == '\u{fffd}')
}

/// `bad of total` records failing `bad`, as one line.
fn count<R>(label: &str, what: &str, v: &[Decoded<R>], bad: impl Fn(&R) -> bool) -> String {
    let n = v.iter().filter(|d| bad(&d.record)).count();
    format!("check {label}: {n} of {} {what}", v.len())
}

fn name<const N: usize>(t: &Text<N>) -> bool {
    unreadable(&t.0)
}

/// The members an active monster copies from its known monster, in the active monster's
/// record order: the member name and whether the two agree.
#[allow(clippy::type_complexity)]
const COPIED: &[(&str, fn(&ActmonRecord, &MonsterRecord) -> bool)] = &[
    ("level", |a, m| a.level == m.level),
    ("unk_0x08C", |a, m| a.unk_0x08C == m.unk_0x0AB),
    ("name", |a, m| a.name.trimmed() == m.name.trimmed()),
    ("wornitem", |a, m| a.wornitem == m.wornitem),
    ("weapon", |a, m| a.weapon == m.weapon),
    ("maxhp", |a, m| a.maxhp == m.maxhp),
    ("alignment", |a, m| a.alignment == m.alignment),
    ("followpct", |a, m| a.followpct == m.followpct),
    ("dr", |a, m| a.dr == m.dr),
    ("ac", |a, m| a.ac == m.ac),
    ("exp", |a, m| a.exp == m.exp),
    ("maxenergy", |a, m| a.maxenergy as u16 == m.energy),
    ("montype", |a, m| a.montype == m.montype),
    ("hpregen", |a, m| a.hpregen == m.hpregen),
    ("unk_0x12C", |a, m| a.unk_0x12C == m.unk_0x19F),
    ("unk_0x12E", |a, m| a.unk_0x12E == m.unk_0x1A1),
    ("unk_0x130", |a, m| a.unk_0x130 == m.unk_0x1A3),
    ("unk_0x131", |a, m| a.unk_0x131 == m.unk_0x1A4),
    ("grouprole", |a, m| a.grouprole == m.grouprole),
];

/// The check lines for `content`.
pub fn checks(content: &Content) -> Vec<String> {
    let c = content;
    let mut out = vec![format!("check layouts: {}", c.set.name())];
    out.push(count("names", "items unreadable", &c.items, |r| {
        name(&r.name)
    }));
    out.push(count("names", "monsters unreadable", &c.monsters, |r| {
        name(&r.name)
    }));
    out.push(count("names", "spells unreadable", &c.spells, |r| {
        name(&r.name)
    }));
    out.push(count("names", "classes unreadable", &c.classes, |r| {
        name(&r.name)
    }));
    out.push(count("names", "races unreadable", &c.races, |r| {
        name(&r.name)
    }));
    out.push(count("names", "shops unreadable", &c.shops, |r| {
        name(&r.name)
    }));
    out.push(count("names", "rooms unreadable", &c.rooms, |r| {
        name(&r.name)
    }));
    out.push(count("names", "actions unreadable", &c.actions, |r| {
        name(&r.name)
    }));
    out.push(count("names", "gangs unreadable", &c.gangs, |r| {
        name(&r.name)
    }));
    out.push(count("names", "characters unreadable", &c.users, |r| {
        name(&r.userid)
    }));

    let rooms: HashSet<(i64, i64)> = c
        .rooms
        .iter()
        .map(|d| (i64::from(d.record.mapnum), i64::from(d.record.roomnum)))
        .collect();
    let (mut exits, mut dangling) = (0, 0);
    for d in &c.rooms {
        let r = &d.record;
        for &to in r.exits.iter().filter(|&&to| to != 0) {
            exits += 1;
            if !rooms.contains(&(i64::from(r.mapnum), i64::from(to))) {
                dangling += 1;
            }
        }
    }
    out.push(format!(
        "check rooms: {dangling} of {exits} exits lead to no room of their map"
    ));

    let races: HashSet<i32> = c.races.iter().map(|d| i32::from(d.record.number)).collect();
    let classes: HashSet<i32> = c
        .classes
        .iter()
        .map(|d| i32::from(d.record.number))
        .collect();
    out.push(count(
        "characters",
        "with an unknown race or class, a level outside 1..=9999, a base stat outside \
         1..=999, or hit points above their maximum",
        &c.users,
        |p| {
            let stats = [p.basint, p.baswil, p.basstr, p.bashea, p.basagi, p.bascha];
            !races.contains(&i32::from(p.race))
                || !classes.contains(&i32::from(p.class))
                || !(1..=9999).contains(&p.level)
                || stats.iter().any(|s| !(1..=999).contains(s))
                || p.hp > p.maxhp
        },
    ));

    let items: HashSet<i32> = c.items.iter().map(|d| d.record.number).collect();
    out.push(count(
        "item ownership",
        "name no item",
        &c.item_ownership,
        |r| !items.contains(&r.itemnum),
    ));
    out.push(count(
        "bank books",
        "with an unreadable owner",
        &c.bank_books,
        |r| name(&r.userid),
    ));

    let known: HashMap<i32, &MonsterRecord> = c
        .monsters
        .iter()
        .map(|d| (d.record.number, &d.record))
        .collect();
    out.push(count(
        "active monsters",
        "name no known monster or no room",
        &c.active_monsters,
        |a| {
            !known.contains_key(&a.knownnum)
                || !rooms.contains(&(i64::from(a.map), i64::from(a.room)))
        },
    ));
    let pairs: Vec<(&ActmonRecord, &MonsterRecord)> = c
        .active_monsters
        .iter()
        .filter_map(|d| known.get(&d.record.knownnum).map(|m| (&d.record, *m)))
        .collect();
    let agree: Vec<String> = COPIED
        .iter()
        .map(|(member, eq)| {
            let n = pairs.iter().filter(|(a, m)| eq(a, m)).count();
            format!("{member} {n}")
        })
        .collect();
    out.push(format!(
        "check active monsters: of {} with a known monster, equal to it: {}",
        pairs.len(),
        agree.join(", ")
    ));
    out
}
