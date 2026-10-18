//! `WCCUPDAT.DAT`: the content patch stream (the game's automatic database update).
//!
//! The original applies the file at every start (without the file nobody may play), one
//! record per poll in ascending `seqnum`. Each record is a type byte and the image of one
//! game record ([`updrec`]); the update looks the existing record up by its key and merges
//! the image into it with that kind's rules (the `merge_*` modules), or inserts it when the
//! key is new.
//!
//! The importer runs the same pass once, on the decoded `.VIR` content, before the
//! content/state split ([`apply_patches`]). The import has no run-time state yet, so "kept"
//! fields keep their virgin values, and the state tables are created from the patched
//! records. Not carried: the pacing of the polls, the sysop
//! menu states and spinner, and the post-update rebuilds (the monster quick references and
//! the limited-item reindex), which later cards compute from the final content.

pub mod merge_item;
pub mod merge_monster;
pub mod merge_race;
pub mod merge_room;
pub mod merge_shop;
pub mod merge_spell;
pub mod merge_text;
pub mod updrec;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::Hash;
use std::path::Path;

pub use merge_item::merge_item;
pub use merge_monster::merge_monster;
pub use merge_race::{merge_class, merge_race};
pub use merge_room::merge_room;
pub use merge_shop::{SHOP_GANG, merge_shop};
pub use merge_spell::merge_spell;
pub use merge_text::{merge_action, merge_message, merge_text};
pub use updrec::{Patch, PatchKind, PatchRecord, read_patches};

use crate::decode::{ItemRecord, TextblkRecord};
use crate::importer::{Content, Decoded, ImportError, UPDATE_FILE};

/// Text equality on two NUL-terminated byte spans: equal up to and including the first NUL.
pub(crate) fn c_str_eq(a: &[u8], b: &[u8]) -> bool {
    let cut = |s: &[u8]| {
        s.iter()
            .position(|&c| c == 0)
            .map_or(s, |n| &s[..n])
            .to_vec()
    };
    cut(a) == cut(b)
}

/// What the patch pass did, per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchReport {
    /// Records whose key existed: the merge ran.
    pub applied_by_kind: BTreeMap<PatchKind, u32>,
    /// Of those, the records the merge rewrote (the ones whose `modified`, `dirty` or
    /// `changed` write-back byte the merge sets), whether or not a value differs.
    pub rewritten_by_kind: BTreeMap<PatchKind, u32>,
    /// Records whose key was new (inserted).
    pub inserted_by_kind: BTreeMap<PatchKind, u32>,
    /// Existing gang shops left alone, counted in `applied`.
    pub gang_shops_skipped: u32,
    /// Records with `seqnum` 0 or below, which the game's GETGT from 0 never reaches.
    pub not_reached: u32,
    /// The first and last `seqnum` applied.
    pub first_seq: Option<i32>,
    /// See `first_seq`.
    pub last_seq: Option<i32>,
}

impl PatchReport {
    /// Applied plus inserted: every record the pass reached.
    pub fn total(&self) -> u32 {
        self.applied_by_kind.values().sum::<u32>() + self.inserted_by_kind.values().sum::<u32>()
    }

    fn get(map: &BTreeMap<PatchKind, u32>, kind: PatchKind) -> u32 {
        map.get(&kind).copied().unwrap_or(0)
    }
}

impl fmt::Display for PatchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for kind in PatchKind::ALL {
            let applied = Self::get(&self.applied_by_kind, kind);
            let inserted = Self::get(&self.inserted_by_kind, kind);
            if applied + inserted == 0 {
                continue;
            }
            writeln!(
                f,
                "patches [{UPDATE_FILE}] {:<11} {applied} applied ({} rewritten), {inserted} inserted",
                kind.name(),
                Self::get(&self.rewritten_by_kind, kind),
            )?;
        }
        let seqs = match (self.first_seq, self.last_seq) {
            (Some(a), Some(b)) => format!(", seqnum {a}..{b}"),
            _ => String::new(),
        };
        write!(f, "patches [{UPDATE_FILE}]: {} records{seqs}", self.total())?;
        if self.gang_shops_skipped > 0 {
            write!(f, ", {} gang shops left alone", self.gang_shops_skipped)?;
        }
        if self.not_reached > 0 {
            write!(f, ", {} with seqnum <= 0 not reached", self.not_reached)?;
        }
        writeln!(f)
    }
}

/// Applies `WCCUPDAT.DAT` at `path` to `content` (the game's update polling, run to the
/// end).
pub fn apply_patches(content: &mut Content, path: &Path) -> Result<PatchReport, ImportError> {
    let (patches, not_reached) = read_patches(path)?;
    let mut report = apply_patch_list(content, &patches)?;
    report.not_reached += not_reached;
    Ok(report)
}

/// The key index of one table: the first record with each key (the importer keeps the
/// first of duplicates too).
fn index<R, K: Hash + Eq>(v: &[Decoded<R>], key: impl Fn(&R) -> K) -> HashMap<K, usize> {
    let mut map = HashMap::with_capacity(v.len());
    for (i, d) in v.iter().enumerate() {
        map.entry(key(&d.record)).or_insert(i);
    }
    map
}

/// The update's lookup of an action: the verb, any case.
fn action_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// An inserted text block keeps only the text's length plus `TEXTOVERHEAD` bytes: the stored
/// text up to its first NUL; the rest reads back as zero.
fn as_inserted_text(mut t: TextblkRecord) -> TextblkRecord {
    if let Some(n) = t.text.0.iter().position(|&b| b == 0) {
        t.text.0[n..].fill(0);
    }
    t
}

/// Applies `patches` to `content` in ascending `seqnum`, whatever their order in the slice;
/// records at `seqnum` 0 or below are not reached (the game's GETGT starts from 0).
/// [`apply_patches`] reads them from the file; tests pass their own.
pub fn apply_patch_list(
    content: &mut Content,
    patches: &[Patch],
) -> Result<PatchReport, ImportError> {
    let mut order: Vec<&Patch> = patches.iter().collect();
    order.sort_by_key(|p| p.seq);
    let mut report = PatchReport::default();

    let mut races = index(&content.races, |r| r.number);
    let mut classes = index(&content.classes, |r| r.number);
    let mut rooms = index(&content.rooms, |r| (r.mapnum, r.roomnum));
    let mut shops = index(&content.shops, |r| r.number);
    let mut items = index(&content.items, |r| r.number);
    let mut spells = index(&content.spells, |r| r.number);
    let mut monsters = index(&content.monsters, |r| r.number);
    let mut messages = index(&content.messages, |r| r.number);
    let mut texts = index(&content.text_blocks, |r| (r.number, r.part));
    let mut actions = index(&content.actions, |r| action_key(&r.name.0));

    /// Finds the record with `key` in `$table` and merges into it, or inserts the image.
    macro_rules! upsert {
        ($p:expr, $table:ident, $idx:ident, $key:expr, $new:expr, $merge:expr) => {{
            let new = $new;
            let key = $key(&new);
            match $idx.get(&key) {
                Some(&i) => {
                    let changed = ($merge)(&mut content.$table[i].record, &new);
                    *report.applied_by_kind.entry($p.kind).or_default() += 1;
                    if changed {
                        *report.rewritten_by_kind.entry($p.kind).or_default() += 1;
                    }
                }
                None => {
                    $idx.insert(key, content.$table.len());
                    content.$table.push(Decoded {
                        file: UPDATE_FILE.to_string(),
                        ordinal: $p.seq as u32,
                        record: new,
                    });
                    *report.inserted_by_kind.entry($p.kind).or_default() += 1;
                }
            }
        }};
    }

    for p in order {
        if p.seq <= 0 {
            report.not_reached += 1;
            continue;
        }
        let record = p.decode().map_err(|source| ImportError::Decode {
            file: format!("{UPDATE_FILE} seqnum {}", p.seq),
            ordinal: p.seq as u32,
            source,
        })?;
        report.first_seq.get_or_insert(p.seq);
        report.last_seq = Some(p.seq);
        match record {
            PatchRecord::Race(new) => {
                upsert!(
                    p,
                    races,
                    races,
                    |r: &crate::decode::RaceRecord| r.number,
                    new,
                    merge_race
                )
            }
            PatchRecord::Class(new) => upsert!(
                p,
                classes,
                classes,
                |r: &crate::decode::ClassRecord| r.number,
                new,
                merge_class
            ),
            // By room and map number; the hidden-item rule reads the item table as patched so
            // far.
            PatchRecord::Room(new) => {
                let new = *new;
                let key = (new.mapnum, new.roomnum);
                match rooms.get(&key) {
                    Some(&i) => {
                        let item_list = &content.items;
                        let item_idx = &items;
                        let lookup = |n: i32| -> Option<&ItemRecord> {
                            item_idx.get(&n).map(|&j| &item_list[j].record)
                        };
                        let changed = merge_room(&mut content.rooms[i].record, &new, lookup);
                        *report.applied_by_kind.entry(p.kind).or_default() += 1;
                        if changed {
                            *report.rewritten_by_kind.entry(p.kind).or_default() += 1;
                        }
                    }
                    None => {
                        // A new room goes into WCCMP001.
                        rooms.insert(key, content.rooms.len());
                        content.rooms.push(Decoded {
                            file: UPDATE_FILE.to_string(),
                            ordinal: p.seq as u32,
                            record: new,
                        });
                        *report.inserted_by_kind.entry(p.kind).or_default() += 1;
                    }
                }
            }
            // An existing gang shop is skipped and logged.
            PatchRecord::Shop(new) => {
                if let Some(&i) = shops.get(&new.number)
                    && content.shops[i].record.type_ == SHOP_GANG
                {
                    tracing::info!(shop = new.number, "AUTO UPDATE : skipping GANG SHOP.");
                    report.gang_shops_skipped += 1;
                    *report.applied_by_kind.entry(p.kind).or_default() += 1;
                    continue;
                }
                upsert!(
                    p,
                    shops,
                    shops,
                    |r: &crate::decode::ShopRecord| r.number,
                    *new,
                    merge_shop
                )
            }
            PatchRecord::Item(new) => {
                upsert!(p, items, items, |r: &ItemRecord| r.number, *new, merge_item)
            }
            PatchRecord::Spell(new) => upsert!(
                p,
                spells,
                spells,
                |r: &crate::decode::SpellRecord| r.number,
                *new,
                merge_spell
            ),
            PatchRecord::Monster(new) => upsert!(
                p,
                monsters,
                monsters,
                |r: &crate::decode::MonsterRecord| r.number,
                *new,
                merge_monster
            ),
            PatchRecord::Message(new) => upsert!(
                p,
                messages,
                messages,
                |r: &crate::decode::MudmsgRecord| r.number,
                *new,
                merge_message
            ),
            // The block with the same number and part.
            PatchRecord::Text(new) => {
                let new = *new;
                if texts.contains_key(&(new.number, new.part)) {
                    upsert!(
                        p,
                        text_blocks,
                        texts,
                        |r: &TextblkRecord| (r.number, r.part),
                        new,
                        |old: &mut TextblkRecord, new: &TextblkRecord| merge_text(
                            old, new, p.carried
                        )
                    )
                } else {
                    upsert!(
                        p,
                        text_blocks,
                        texts,
                        |r: &TextblkRecord| (r.number, r.part),
                        as_inserted_text(new),
                        |old: &mut TextblkRecord, new: &TextblkRecord| merge_text(
                            old, new, p.carried
                        )
                    )
                }
            }
            // The verb, any case.
            PatchRecord::Action(new) => upsert!(
                p,
                actions,
                actions,
                |r: &crate::decode::ActionRecord| action_key(&r.name.0),
                *new,
                merge_action
            ),
        }
    }
    Ok(report)
}
