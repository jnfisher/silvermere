//! The colour table [`CLRTBL`], its row names, the player's palette, and
//! [`get_hp_colour`].
//!
//! A row is one kind of game text (the `COL_*` names); a column is the player's palette
//! (`SET PALETTE 0-3`). Only rows 0-14 are filled; a room or game message is coloured
//! only for rows up to [`MAXCLR`] (14), so callers pass [`TELL_NOCOLOR`] (0xFF) for an
//! uncoloured message.
//!
//! `build.rs` includes this module by path to validate the `{@COL_*}` names the
//! catalogue uses, so it refers to the ANSI constants as `super::ansi`.

use super::ansi::{
    ANSI_BLKBACK, ANSI_BLKONWHT, ANSI_BROWN, ANSI_CYAN, ANSI_GREEN, ANSI_GREY, ANSI_LTBLUE,
    ANSI_LTCYAN, ANSI_LTMAGENTA, ANSI_LTRED, ANSI_MAGENTA, ANSI_RED, ANSI_WHITE, NOCOLOR,
};

/// A row of the colour table, the `COL_*` names.
///
/// Placeholder: T-002 defines the `COL_*` enum in `silvermere-types`; T-015 reconciles
/// this one with it. The discriminants are the table rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ColRow {
    /// `COL_EXITS`: the "Obvious exits:" label.
    Exits = 0,
    /// `COL_ALSOHERE`: the "Also here:" label and its closing ".".
    AlsoHere = 1,
    /// `COL_PROMPT`: prompt text; ends a coloured value.
    Prompt = 2,
    /// `COL_PROMPTVAL`: prompt values; hit points above 1/2.
    PromptVal = 3,
    /// `COL_HPHURT`: hit points at 1/2 or less, above 1/4.
    HpHurt = 4,
    /// `COL_HPLOW`: hit points at 1/4 or less.
    HpLow = 5,
    /// `COL_SPELLGEN`: messages of `SPT_GENERAL` spells.
    SpellGen = 6,
    /// `COL_SPELLOTH`: messages of all other spell types.
    SpellOth = 7,
    /// `COL_HERENAMES`: names listed after "Also here:".
    HereNames = 8,
    /// `COL_MOVENAME`: name of a player coming or going.
    MoveName = 9,
    /// `COL_LINEEND`: end of a movement line.
    LineEnd = 10,
    /// `COL_MOVETEXT`: "just left to", "walks into the".
    MoveText = 11,
    /// `COL_ACTION`: social action seen in the room.
    Action = 12,
    /// `COL_TELEPATH`: telepathed words.
    Telepath = 13,
    /// `COL_BROADCAST`: gossip and auction lines.
    Broadcast = 14,
}

impl ColRow {
    /// Every row, in table order.
    pub const ALL: [ColRow; 15] = [
        ColRow::Exits,
        ColRow::AlsoHere,
        ColRow::Prompt,
        ColRow::PromptVal,
        ColRow::HpHurt,
        ColRow::HpLow,
        ColRow::SpellGen,
        ColRow::SpellOth,
        ColRow::HereNames,
        ColRow::MoveName,
        ColRow::LineEnd,
        ColRow::MoveText,
        ColRow::Action,
        ColRow::Telepath,
        ColRow::Broadcast,
    ];

    /// The `COL_*` name, for the catalogue's `{@COL_*}` references.
    pub const fn macro_name(self) -> &'static str {
        match self {
            ColRow::Exits => "COL_EXITS",
            ColRow::AlsoHere => "COL_ALSOHERE",
            ColRow::Prompt => "COL_PROMPT",
            ColRow::PromptVal => "COL_PROMPTVAL",
            ColRow::HpHurt => "COL_HPHURT",
            ColRow::HpLow => "COL_HPLOW",
            ColRow::SpellGen => "COL_SPELLGEN",
            ColRow::SpellOth => "COL_SPELLOTH",
            ColRow::HereNames => "COL_HERENAMES",
            ColRow::MoveName => "COL_MOVENAME",
            ColRow::LineEnd => "COL_LINEEND",
            ColRow::MoveText => "COL_MOVETEXT",
            ColRow::Action => "COL_ACTION",
            ColRow::Telepath => "COL_TELEPATH",
            ColRow::Broadcast => "COL_BROADCAST",
        }
    }

    /// The row for a `COL_*` name.
    pub fn from_macro_name(name: &str) -> Option<ColRow> {
        ColRow::ALL.into_iter().find(|row| row.macro_name() == name)
    }

    /// The row for a table index (a `tell_*` colour argument); `None` above 14.
    pub fn from_index(index: u8) -> Option<ColRow> {
        ColRow::ALL.get(usize::from(index)).copied()
    }

    /// The table index.
    pub const fn index(self) -> u8 {
        self as u8
    }
}

/// The player's colour palette, 0 to 3.
///
/// `WCCMMUD.DOC` describes them as 0 medium brightness, 1 dim, 2 the brightest and 3 a
/// combination of 0 and 1; the table is the truth. A new character starts on palette 0
/// and a stored value of 4 or more is repaired to 0 at entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Palette(u8);

impl Palette {
    /// The number of palettes (the table's second dimension).
    pub const COUNT: u8 = 4;

    /// Palette `n` if it is 0 to 3.
    pub const fn new(n: u8) -> Option<Palette> {
        if n < Palette::COUNT {
            Some(Palette(n))
        } else {
            None
        }
    }

    /// Palette `n`, with an out-of-range value repaired to 0 as at entry.
    pub const fn repaired(n: u8) -> Palette {
        if n < Palette::COUNT {
            Palette(n)
        } else {
            Palette(0)
        }
    }

    /// The table column.
    pub const fn index(self) -> u8 {
        self.0
    }
}

/// The last row with colours. Room and game messages are coloured only for rows up to
/// this one.
pub const MAXCLR: u8 = 14;

/// The colour argument that sends nothing.
pub const TELL_NOCOLOR: u8 = 0xFF;

/// The colour table, by row and palette, for the fifteen filled rows, each entry
/// resolved to its ANSI half. `NOCOLOR` entries are empty.
pub const CLRTBL: [[&[u8]; Palette::COUNT as usize]; 15] = [
    // row 0, COL_EXITS
    [ANSI_GREEN, ANSI_BROWN, ANSI_GREEN, ANSI_GREEN],
    // row 1, COL_ALSOHERE
    [ANSI_MAGENTA, ANSI_GREEN, ANSI_LTMAGENTA, ANSI_MAGENTA],
    // row 2, COL_PROMPT
    [ANSI_GREY, ANSI_GREY, ANSI_CYAN, ANSI_CYAN],
    // row 3, COL_PROMPTVAL
    [NOCOLOR, NOCOLOR, ANSI_LTCYAN, ANSI_LTCYAN],
    // row 4, COL_HPHURT
    [ANSI_WHITE, ANSI_WHITE, ANSI_GREY, ANSI_GREY],
    // row 5, COL_HPLOW
    [ANSI_LTRED, ANSI_LTRED, ANSI_LTRED, ANSI_LTRED],
    // row 6, COL_SPELLGEN
    [ANSI_LTBLUE, ANSI_LTBLUE, ANSI_LTCYAN, ANSI_LTCYAN],
    // row 7, COL_SPELLOTH
    [ANSI_LTRED, ANSI_LTRED, ANSI_LTRED, ANSI_LTRED],
    // row 8, COL_HERENAMES
    [ANSI_LTMAGENTA, NOCOLOR, NOCOLOR, ANSI_LTMAGENTA],
    // row 9, COL_MOVENAME
    [ANSI_LTRED, ANSI_RED, ANSI_BLKONWHT, ANSI_LTRED],
    // row 10, COL_LINEEND
    [NOCOLOR, NOCOLOR, ANSI_BLKBACK, NOCOLOR],
    // row 11, COL_MOVETEXT
    [ANSI_GREEN, ANSI_GREEN, NOCOLOR, ANSI_GREEN],
    // row 12, COL_ACTION
    [ANSI_GREEN, ANSI_GREEN, ANSI_GREEN, ANSI_GREEN],
    // row 13, COL_TELEPATH
    [ANSI_GREY, ANSI_GREY, ANSI_GREY, ANSI_LTCYAN],
    // row 14, COL_BROADCAST
    [ANSI_MAGENTA, ANSI_CYAN, ANSI_LTMAGENTA, ANSI_LTCYAN],
];

/// A colour-table entry: a row in the player's palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour {
    pub row: ColRow,
    pub palette: Palette,
}

impl Colour {
    pub const fn new(row: ColRow, palette: Palette) -> Colour {
        Colour { row, palette }
    }

    /// The bytes the original printed just before the text it coloured, the table
    /// entry for the row and palette. Empty for a `NOCOLOR` entry.
    pub const fn prefix(self) -> &'static [u8] {
        CLRTBL[self.row as usize][self.palette.0 as usize]
    }
}

/// The colour prefix for a room or game message's colour argument: the table entry for
/// rows up to [`MAXCLR`], nothing for [`TELL_NOCOLOR`] or any other row above it (the
/// sysop broadcast uses row 15 for an uncoloured line the same way).
pub fn tell_colour(row: u8, palette: Palette) -> &'static [u8] {
    match ColRow::from_index(row) {
        Some(r) if row <= MAXCLR => Colour::new(r, palette).prefix(),
        _ => NOCOLOR,
    }
}

/// The row a hit point figure is shown in ([`get_hp_colour`] without the table lookup).
/// Healthy above half the maximum, hurt above a quarter, badly hurt at a quarter or less;
/// integer division, as written.
pub fn hp_colour_row(hp: i32, maxhp: i32) -> ColRow {
    if hp > maxhp / 2 {
        ColRow::PromptVal
    } else if hp > maxhp / 4 {
        ColRow::HpHurt
    } else {
        ColRow::HpLow
    }
}

/// The colour prefix for a hit point figure in the player's palette.
pub fn get_hp_colour(hp: i32, maxhp: i32, palette: Palette) -> &'static [u8] {
    Colour::new(hp_colour_row(hp, maxhp), palette).prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_index_the_table_in_order() {
        for (i, row) in ColRow::ALL.iter().enumerate() {
            assert_eq!(usize::from(row.index()), i);
            assert_eq!(ColRow::from_index(row.index()), Some(*row));
            assert_eq!(ColRow::from_macro_name(row.macro_name()), Some(*row));
        }
        assert_eq!(ColRow::from_index(15), None);
        assert_eq!(ColRow::from_macro_name("COL_NOPE"), None);
    }

    #[test]
    fn rows_above_maxclr_send_nothing() {
        let p = Palette::default();
        assert_eq!(tell_colour(0, p), ANSI_GREEN);
        assert_eq!(tell_colour(14, p), ANSI_MAGENTA);
        assert_eq!(tell_colour(15, p), b"");
        assert_eq!(tell_colour(TELL_NOCOLOR, p), b"");
    }

    // a new character starts on 0; 4 or more is repaired to 0
    #[test]
    fn palette_range() {
        assert_eq!(Palette::new(3), Some(Palette(3)));
        assert_eq!(Palette::new(4), None);
        assert_eq!(Palette::repaired(4), Palette(0));
        assert_eq!(Palette::default(), Palette(0));
    }
}
