//! The `EDITCHA1` form (WCCTEXT.MSG message 6) as screen cells: the full-screen
//! character editor's background and the bytes that draw, focus and clear its fields
//! (T-063).
//!
//! The original handed the template to the host's full-screen data entry (`fsdroom`);
//! `?` runs are text fields, `$` runs numeric fields and the `!` run on the last line is
//! the error line (`fsdemg`). The bytes the host sent come from the T-005 capture
//! `tests/captures/creation-fsd/raw.bin`:
//!
//! | What | Bytes |
//! |---|---|
//! | background | `ESC[0m ESC[2J ESC[H ESC[0m`, then the template with `\r\n` line ends |
//! | a field | `ESC[0;1m ESC[r;cf` attr, `width` blanks, `ESC[r;cf` value |
//! | the focused field | `ESC[r;cf ESC[0;1;7m`, `width` blanks, `ESC[r;cf` value `ESC[r;cf` |
//! | the error line, empty | `ESC[0;1m ESC[23;1f ESC[1;31m` and 61 blanks |
//! | leaving the form | `ESC[24;0f ESC[0m` |
//!
//! A field's attr is the last SGR sequence of the template between the previous field
//! and this one (empty when there is none: the race maximum after ` to `). The CP display
//! is the game's own: `ESC[20;33f ESC[0m` and the number right-aligned in four columns.
//!
//! Row and column numbers live in this module only: the fields' come from the template,
//! the CP display's and the exit position are the constants below.

use crate::cp437;

/// The CP display: row 20, column 33.
const CP_LEFT_POS: (u16, u16) = (20, 33);
/// Where the FSD kernel left the cursor when the form closed (capture, after SAVE).
const LEAVE_POS: (u16, u16) = (24, 0);
/// What the kernel sent before the template (capture).
const BACKGROUND_PREFIX: &[u8] = b"\x1b[0m\x1b[2J\x1b[H\x1b[0m";
/// The focused field's attribute (capture).
const FOCUS_ATTR: &[u8] = b"\x1b[0;1;7m";
/// Sent before every unfocused field (capture).
const FIELD_RESET: &[u8] = b"\x1b[0;1m";

/// The kind of a placeholder run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// `?`: any text.
    Text,
    /// `$`: digits.
    Numeric,
    /// `!`: the kernel's error line.
    Message,
}

/// One field of the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenField {
    /// 1-based row.
    pub row: u16,
    /// 1-based column.
    pub col: u16,
    /// The placeholder's length.
    pub width: u16,
    pub kind: CellKind,
    /// The SGR sequence that colours the field when it is not focused (may be empty).
    pub attr: Vec<u8>,
}

/// The parsed form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorScreen {
    /// The template as Unicode, `\r` line ends.
    template: String,
    /// The input fields in template order, numbered from 0.
    fields: Vec<ScreenField>,
    /// The error line.
    message: ScreenField,
}

/// Why a template did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScreenError {
    #[error("the template has no `!` error line")]
    NoMessageLine,
}

impl EditorScreen {
    /// The `EDITCHA1` form of the installed package text ([`crate::package`]), or `None`
    /// while the text is not installed.
    pub fn editcha1() -> Option<&'static EditorScreen> {
        crate::package::installed().map(|p| p.editcha1_screen())
    }

    /// Parses a template given as text (`\r` line ends, ANSI sequences inline).
    pub fn parse(text: &str) -> Result<Self, ScreenError> {
        let mut fields = Vec::new();
        let (mut row, mut col) = (1u16, 1u16);
        let mut attr: Vec<u8> = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\u{1b}' => {
                    let mut seq = String::from(c);
                    for n in chars.by_ref() {
                        seq.push(n);
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                    if seq.ends_with('m') {
                        attr = seq.into_bytes();
                    }
                }
                '\r' => {
                    row += 1;
                    col = 1;
                }
                '?' | '$' | '!' => {
                    let mut width = 1u16;
                    while chars.peek() == Some(&c) {
                        chars.next();
                        width += 1;
                    }
                    let kind = match c {
                        '?' => CellKind::Text,
                        '$' => CellKind::Numeric,
                        _ => CellKind::Message,
                    };
                    fields.push(ScreenField {
                        row,
                        col,
                        width,
                        kind,
                        attr: std::mem::take(&mut attr),
                    });
                    col += width;
                }
                _ => col += 1,
            }
        }
        let at = fields
            .iter()
            .position(|f| f.kind == CellKind::Message)
            .ok_or(ScreenError::NoMessageLine)?;
        let message = fields.remove(at);
        Ok(Self {
            template: text.to_string(),
            fields,
            message,
        })
    }

    /// The number of input fields (26 for `EDITCHA1`: `SF_FIRSTNAME` .. `SF_SAVE`).
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Field `n` (an `SF_*` number).
    pub fn field(&self, n: usize) -> &ScreenField {
        &self.fields[n]
    }

    /// Row and column of field `n`.
    pub fn field_pos(&self, n: usize) -> (u16, u16) {
        let f = &self.fields[n];
        (f.row, f.col)
    }

    /// The error line.
    pub fn message_field(&self) -> &ScreenField {
        &self.message
    }

    /// Clears the screen and draws the template, placeholders and all.
    pub fn background(&self) -> Vec<u8> {
        let mut out = BACKGROUND_PREFIX.to_vec();
        out.extend(cp437::encode(&self.template.replace('\r', "\r\n")));
        out
    }

    /// Field `n` unfocused, showing `value`.
    pub fn draw(&self, n: usize, value: &str) -> Vec<u8> {
        let f = &self.fields[n];
        let mut out = FIELD_RESET.to_vec();
        goto(&mut out, f.row, f.col);
        out.extend_from_slice(&f.attr);
        blanks(&mut out, f.width);
        goto(&mut out, f.row, f.col);
        out.extend(cp437::encode(&fit(value, f.width)));
        out
    }

    /// Field `n` focused (reverse video), showing `value`, the cursor at its start.
    pub fn focus(&self, n: usize, value: &str) -> Vec<u8> {
        let f = &self.fields[n];
        let mut out = Vec::new();
        goto(&mut out, f.row, f.col);
        out.extend_from_slice(FOCUS_ATTR);
        blanks(&mut out, f.width);
        goto(&mut out, f.row, f.col);
        out.extend(cp437::encode(&fit(value, f.width)));
        goto(&mut out, f.row, f.col);
        out
    }

    /// The error line showing `text`; empty clears it. The cursor is left on the line;
    /// the caller puts it back on a field.
    pub fn message(&self, text: &str) -> Vec<u8> {
        let f = &self.message;
        let mut out = FIELD_RESET.to_vec();
        goto(&mut out, f.row, f.col);
        out.extend_from_slice(&f.attr);
        blanks(&mut out, f.width);
        if !text.is_empty() {
            goto(&mut out, f.row, f.col);
            out.extend(cp437::encode(&fit(text, f.width)));
        }
        out
    }

    /// The CP-left display on the form.
    pub fn cp_left(&self, cp: i32) -> Vec<u8> {
        let mut out = Vec::new();
        goto(&mut out, CP_LEFT_POS.0, CP_LEFT_POS.1);
        out.extend_from_slice(b"\x1b[0m");
        out.extend(format!("{cp:4}").into_bytes());
        out
    }

    /// The kernel leaving the form: the cursor below it, attributes reset.
    pub fn leave(&self) -> Vec<u8> {
        let mut out = Vec::new();
        goto(&mut out, LEAVE_POS.0, LEAVE_POS.1);
        out.extend_from_slice(b"\x1b[0m");
        out
    }
}

/// `ESC[row;colf`: the absolute move the kernel used (final byte `f`).
fn goto(out: &mut Vec<u8>, row: u16, col: u16) {
    out.extend(format!("\x1b[{row};{col}f").into_bytes());
}

fn blanks(out: &mut Vec<u8>, width: u16) {
    out.extend(std::iter::repeat_n(b' ', usize::from(width)));
}

fn fit(value: &str, width: u16) -> String {
    value.chars().take(usize::from(width)).collect()
}
