//! The `textblk` record (WCCTEXT).

use serde::Serialize;

use crate::ids::TextBlockNo;
use crate::records::layout::{self, Layout};

/// One part of a text block: the `textblk` record.
///
/// The record stores each text byte plus 0x20; `text` holds it decoded (the importer
/// subtracts 0x20 at the boundary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextBlock {
    /// +0x000 continuation number, 0 first.
    pub part: i16,
    /// +0x00E text block number.
    pub number: TextBlockNo,
    /// +0x012 linked text block.
    pub link: TextBlockNo,
    /// +0x016 the text, decoded (at most `TEXTSIZE` bytes).
    pub text: String,
}

impl TextBlock {
    /// Byte layout of the `textblk` record.
    pub const LAYOUT: &'static Layout = &layout::TEXTBLK;
    /// The `textblk` record length: 2022 bytes.
    pub const RECORD_LEN: usize = 2022;
}
