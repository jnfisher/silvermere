//! `--json OUT`: the decoded records in the shape of T-006's export, so the two
//! exporters compare line by line.
//!
//! One `OUT/<TABLE>.jsonl` per Btrieve file (`WCCMP001.jsonl`, `WCCITEMS.jsonl`, ...),
//! one object per record in reader order:
//!
//! ```text
//! {"_id": 1, "_raw_len": 1528, "mapnum": 1, "roomnum": 1, "unk_0x008": "AAAA...", "name": ...}
//! ```
//!
//! - `_id` is the reader's 1-based ordinal ([`crate::btrieve::Record::ordinal`]), which is
//!   MBBSEmu's `data_t.id` and so T-006's `_id`; `_raw_len` is the record length.
//! - Every other key is a record member name with the value
//!   [`crate::decode::Wire::json`] gives it: integers by their stored width and sign, texts cut at the
//!   first NUL, CP437 and trimmed like `mmud_data._s`, arrays in slot order, `unk_*` byte
//!   spans base64. T-006 omits most `unk_*` members; this export writes them all. Keys
//!   are sorted (serde_json's map); T-006 writes them in record order, so compare
//!   by key.
//! - The renames and additions are made here only: a known monster also carries
//!   `cash_copper_first` (its `cash` reversed); a text block's 2000 stored bytes are
//!   `text_raw` (base64, still shifted by 0x20, as T-006 writes them) and the decoded text
//!   is `text` (T-006 has none).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::decode::{MonsterRecord, Record, TextblkRecord, textblk};

/// T-006's name for a text block's stored text.
pub const TEXT_RAW: &str = "text_raw";
/// T-006's name for a known monster's cash in copper-first order.
pub const CASH_COPPER_FIRST: &str = "cash_copper_first";

/// A record type as the export writes it: the members plus this table's extra keys.
pub trait ExportRecord: Record {
    /// The object for record `id` (the reader ordinal).
    fn export(&self, id: u32) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("_id".to_string(), Value::from(id));
        map.insert("_raw_len".to_string(), Value::from(Self::LAYOUT.len));
        map.extend(self.to_json());
        self.extra(&mut map);
        map
    }

    /// Adds or renames keys after the members (nothing for most tables).
    fn extra(&self, _map: &mut Map<String, Value>) {}
}

macro_rules! plain_export {
    ($($t:ty),*) => { $( impl ExportRecord for $t {} )* };
}
plain_export!(
    crate::decode::RoomRecord,
    crate::decode::ItemRecord,
    crate::decode::ActmonRecord,
    crate::decode::SpellRecord,
    crate::decode::ClassRecord,
    crate::decode::RaceRecord,
    crate::decode::ShopRecord,
    crate::decode::MudmsgRecord,
    crate::decode::ActionRecord,
    crate::decode::GangRecord,
    crate::decode::BankbookRecord,
    crate::decode::ItemownRecord,
    crate::decode::PlayerRecord
);

impl ExportRecord for MonsterRecord {
    fn extra(&self, map: &mut Map<String, Value>) {
        let mut reversed = self.cash;
        reversed.reverse();
        map.insert(
            CASH_COPPER_FIRST.to_string(),
            Value::Array(reversed.iter().map(|&c| Value::from(c)).collect()),
        );
    }
}

impl ExportRecord for TextblkRecord {
    fn extra(&self, map: &mut Map<String, Value>) {
        if let Some(raw) = map.remove("text") {
            map.insert(TEXT_RAW.to_string(), raw);
        }
        map.insert(
            "text".to_string(),
            Value::String(textblk::decode_text(&self.text.0)),
        );
    }
}

/// Writes `OUT/<TABLE>.jsonl` files.
#[derive(Debug)]
pub struct JsonWriter {
    dir: PathBuf,
}

impl JsonWriter {
    /// Creates `dir` if needed.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Writes `table`'s records (`(ordinal, record)`), replacing the file.
    pub fn write<'a, R: ExportRecord + 'a>(
        &self,
        table: &str,
        records: impl IntoIterator<Item = (u32, &'a R)>,
    ) -> std::io::Result<PathBuf> {
        let path = self.dir.join(format!("{table}.jsonl"));
        let mut out = BufWriter::new(File::create(&path)?);
        for (id, r) in records {
            serde_json::to_writer(&mut out, &Value::Object(r.export(id)))?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
        Ok(path)
    }
}
