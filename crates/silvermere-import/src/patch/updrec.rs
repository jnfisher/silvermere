//! `WCCUPDAT.DAT` records: the `updrec` record.
//!
//! `seqnum` (32-bit, +0x000, key 0), `type` (a byte, +0x004, one of `UPD_RACE 1 ..
//! UPD_ACTION 10`) and `data[2022]` (+0x005), the image of one record of that kind. The
//! game opens the file with a 2027-byte record length, but the stock file's records are
//! 2015 bytes, so only 2010 bytes of `data` come from the file. The rest of the buffer is never
//! written by a read; it is taken as zero here (the buffer as allocated). Every record kind
//! except text blocks fits in 2010 bytes (the room, the largest, is 1528); a text block's
//! image loses its last 12 bytes (`text[1988..2000]`), which the text merge keeps from the
//! stored block ([`Patch::carried`], `merge_text`).
//!
//! The Worldgroup 3.x build's `WCCUPDA2` record is 2032 bytes: `seqnum` and `type` as above,
//! 3 bytes of padding, then `data[2024]` at +0x008, the image of a Worldgroup-layout record
//! (the largest is the aligned text block's 2024 bytes). [`Patch::parse_as`] takes the
//! layout set and [`Patch::decode`] decodes the image with it.

use std::path::Path;

use silvermere_types::records::LayoutSet;

use crate::btrieve::BtrieveFile;
use crate::decode::{
    ActionRecord, ClassRecord, DecodeError, ItemRecord, MonsterRecord, MudmsgRecord, RaceRecord,
    Record, RoomRecord, ShopRecord, SpellRecord, TextblkRecord,
};
use crate::importer::ImportError;

/// The `updrec` record length: the game's read buffer.
pub const UPDREC_LEN: usize = 2027;
/// The length of the record's `data` image.
pub const DATA_LEN: usize = 2022;
/// `seqnum` and `type` before the image.
const HEADER_LEN: usize = 5;

/// The record length, the bytes before the image and the image's length in `set`.
pub fn updrec_shape(set: LayoutSet) -> (usize, usize, usize) {
    match set {
        LayoutSet::Dos => (UPDREC_LEN, HEADER_LEN, DATA_LEN),
        LayoutSet::Worldgroup => (2032, 8, 2024),
    }
}

/// The record's `type` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PatchKind {
    /// `UPD_RACE` 1, WCCRACE.
    Race = 1,
    /// `UPD_CLASS` 2, WCCCLASS.
    Class = 2,
    /// `UPD_ROOM` 3, WCCMP001.
    Room = 3,
    /// `UPD_SHOP` 4, WCCSHOPS.
    Shop = 4,
    /// `UPD_ITEM` 5, WCCITEMS.
    Item = 5,
    /// `UPD_SPELL` 6, WCCSPELS.
    Spell = 6,
    /// `UPD_KNMON` 7, WCCKNMSR.
    Monster = 7,
    /// `UPD_MESSAGE` 8, WCCMSG.
    Message = 8,
    /// `UPD_TEXT` 9, WCCTEXT.
    Text = 9,
    /// `UPD_ACTION` 10, WCCACTS.
    Action = 10,
}

impl PatchKind {
    /// Every kind, in type-code order.
    pub const ALL: [PatchKind; 10] = [
        PatchKind::Race,
        PatchKind::Class,
        PatchKind::Room,
        PatchKind::Shop,
        PatchKind::Item,
        PatchKind::Spell,
        PatchKind::Monster,
        PatchKind::Message,
        PatchKind::Text,
        PatchKind::Action,
    ];

    /// The kind for a type byte, `None` outside 1..10.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code).wrapping_sub(1)).copied()
    }

    /// The `UPD_xxx` name.
    pub fn name(self) -> &'static str {
        match self {
            PatchKind::Race => "UPD_RACE",
            PatchKind::Class => "UPD_CLASS",
            PatchKind::Room => "UPD_ROOM",
            PatchKind::Shop => "UPD_SHOP",
            PatchKind::Item => "UPD_ITEM",
            PatchKind::Spell => "UPD_SPELL",
            PatchKind::Monster => "UPD_KNMON",
            PatchKind::Message => "UPD_MESSAGE",
            PatchKind::Text => "UPD_TEXT",
            PatchKind::Action => "UPD_ACTION",
        }
    }

    /// The schema table the kind patches.
    pub fn table(self) -> &'static str {
        match self {
            PatchKind::Race => "races",
            PatchKind::Class => "classes",
            PatchKind::Room => "rooms",
            PatchKind::Shop => "shops",
            PatchKind::Item => "items",
            PatchKind::Spell => "spells",
            PatchKind::Monster => "monsters",
            PatchKind::Message => "messages",
            PatchKind::Text => "text_blocks",
            PatchKind::Action => "actions",
        }
    }
}

/// One `updrec`: its sequence number, kind and the 2022-byte image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// `seqnum`, key 0: the application order.
    pub seq: i32,
    /// `type`.
    pub kind: PatchKind,
    /// `data[2022]`: the bytes read, zero after them.
    pub image: Vec<u8>,
    /// How many bytes of `data` the record carried (2010 in the stock file).
    pub carried: usize,
    /// The layouts the image is in.
    pub set: LayoutSet,
}

/// An image decoded with the T-008 decoder for its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchRecord {
    /// The `race` record.
    Race(RaceRecord),
    /// The `class` record.
    Class(ClassRecord),
    /// The `room` record.
    Room(Box<RoomRecord>),
    /// The `shop` record.
    Shop(Box<ShopRecord>),
    /// The `item` record.
    Item(Box<ItemRecord>),
    /// The `spell` record.
    Spell(Box<SpellRecord>),
    /// The `monster` record.
    Monster(Box<MonsterRecord>),
    /// The `mudmsg` record.
    Message(Box<MudmsgRecord>),
    /// The `textblk` record: all 2022 bytes of `data`.
    Text(Box<TextblkRecord>),
    /// The `action` record.
    Action(Box<ActionRecord>),
}

impl Patch {
    /// Splits one record of the file; `raw` is the record as stored (2015 bytes in the
    /// stock file, at most [`UPDREC_LEN`]). A type byte outside 1..10 is an error naming
    /// the record's `seqnum`.
    pub fn parse(raw: &[u8]) -> Result<Self, ImportError> {
        Self::parse_as(raw, LayoutSet::Dos)
    }

    /// [`Patch::parse`] for a record of `set`'s layout (a Worldgroup 3.x record is at most
    /// 2032 bytes, its image at +0x008).
    pub fn parse_as(raw: &[u8], set: LayoutSet) -> Result<Self, ImportError> {
        let (record_len, header_len, data_len) = updrec_shape(set);
        if raw.len() < header_len || raw.len() > record_len {
            return Err(ImportError::Patch(format!(
                "record of {} bytes (struct updrec is {record_len})",
                raw.len()
            )));
        }
        let seq = i32::from_le_bytes(raw[..4].try_into().expect("4 bytes"));
        let kind = PatchKind::from_code(raw[4]).ok_or_else(|| {
            ImportError::Patch(format!("seqnum {seq}: type {} is not 1..10", raw[4]))
        })?;
        let mut image = raw[header_len..].to_vec();
        let carried = image.len();
        image.resize(data_len, 0);
        Ok(Self {
            seq,
            kind,
            image,
            carried,
            set,
        })
    }

    /// The image decoded as its kind's record (the leading bytes of `data`; the rest of
    /// the 2022 is ignored, as the game ignores it).
    pub fn decode(&self) -> Result<PatchRecord, DecodeError> {
        fn head<R: Record>(image: &[u8], set: LayoutSet) -> Result<R, DecodeError> {
            R::decode_as(&image[..R::layout(set).len], set)
        }
        let i = self.image.as_slice();
        let s = self.set;
        Ok(match self.kind {
            PatchKind::Race => PatchRecord::Race(head(i, s)?),
            PatchKind::Class => PatchRecord::Class(head(i, s)?),
            PatchKind::Room => PatchRecord::Room(Box::new(head(i, s)?)),
            PatchKind::Shop => PatchRecord::Shop(Box::new(head(i, s)?)),
            PatchKind::Item => PatchRecord::Item(Box::new(head(i, s)?)),
            PatchKind::Spell => PatchRecord::Spell(Box::new(head(i, s)?)),
            PatchKind::Monster => PatchRecord::Monster(Box::new(head(i, s)?)),
            PatchKind::Message => PatchRecord::Message(Box::new(head(i, s)?)),
            PatchKind::Text => PatchRecord::Text(Box::new(head(i, s)?)),
            PatchKind::Action => PatchRecord::Action(Box::new(head(i, s)?)),
        })
    }
}

/// Every record of the update file, in ascending `seqnum` (key 0) whatever their physical
/// order. Only records with `seqnum` above 0 are returned: the game reads the file with
/// GETGT on key 0 starting from sequence number 0, so a record at 0 or below is never
/// applied. The second
/// value is how many were left out for that.
pub fn read_patches(path: &Path) -> Result<(Vec<Patch>, u32), ImportError> {
    read_patches_as(path, LayoutSet::Dos)
}

/// [`read_patches`] for a file of `set`'s layout (`WCCUPDA2.DAT`).
pub fn read_patches_as(path: &Path, set: LayoutSet) -> Result<(Vec<Patch>, u32), ImportError> {
    let mut file = BtrieveFile::open(path)?;
    let mut patches = Vec::with_capacity(file.header.record_count as usize);
    let mut records = file.records();
    while let Some(record) = records.next_record() {
        patches.push(Patch::parse_as(record?.data, set)?);
    }
    let before = patches.len();
    patches.retain(|p| p.seq > 0);
    let not_after_zero = (before - patches.len()) as u32;
    // Key 0 is unique (an autoincrement key), so the order is total.
    patches.sort_by_key(|p| p.seq);
    Ok((patches, not_after_zero))
}
