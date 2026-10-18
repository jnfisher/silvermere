//! WCCTEXT text blocks: the `textblk` record.
//!
//! The file is variable-length (T-007): a record is the 22-byte fixed part plus up to
//! 2000 text bytes, so [`decode_textblk`] zero-fills it to the 2022-byte buffer the game
//! reads it into.
//!
//! The text is stored with 0x20 added to each byte. The game takes the stored bytes up to
//! the first stored NUL, subtracts 0x20 from each and keeps at most 1999 bytes, so the text
//! ends at the first stored 0x00 or 0x20 (a decoded NUL), whichever comes first.
//! [`decode_text`] is that rule.
//! `unk_0x002` (inside key 0, the same in every record) is not carried.

use silvermere_types::ids::TextBlockNo;
use silvermere_types::records::{TextBlock, layout};

use super::{DecodeError, Raw, Record, cp437, record};

/// The stored-text shift (`TEXTSHIFT`).
pub const TEXT_SHIFT: u8 = 0x20;

record! {
    /// The `textblk` record, WCCTEXT: every member, in record order.
    pub struct TextblkRecord = layout::TEXTBLK {
        part: i16,
        unk_0x002: Raw<12>,
        number: i32,
        link: i32,
        text: Raw<2000>,
    }
}

/// Decodes one WCCTEXT record, zero-filling a short variable-length image to the
/// 2022-byte record buffer first.
pub fn decode_textblk(raw: &[u8]) -> Result<TextblkRecord, DecodeError> {
    let len = TextblkRecord::LAYOUT.len;
    if raw.len() < len {
        let mut full = raw.to_vec();
        full.resize(len, 0);
        TextblkRecord::decode(&full)
    } else {
        TextblkRecord::decode(raw)
    }
}

/// The text of a stored block (see the module docs): bytes up to the first stored 0x00
/// or 0x20, at most `TEXTSIZE - 1`, each shifted down by 0x20, as CP437.
pub fn decode_text(stored: &[u8]) -> String {
    let limit = stored.len().saturating_sub(1);
    let bytes: Vec<u8> = stored[..limit]
        .iter()
        .take_while(|&&b| b != 0 && b != TEXT_SHIFT)
        .map(|b| b.wrapping_sub(TEXT_SHIFT))
        .collect();
    cp437(&bytes)
}

/// The schema's text block part.
pub fn to_text_block(r: &TextblkRecord) -> TextBlock {
    TextBlock {
        part: r.part,
        number: TextBlockNo(r.number),
        link: TextBlockNo(r.link),
        text: decode_text(&r.text.0),
    }
}
