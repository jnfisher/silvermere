//! 79-column word wrap: what the host's word wrap (`btutsw`) did to the output of the
//! displays that asked for it (room descriptions, lists and the like), following
//! MBBSEmu's `Session/LineBreaker.cs`.
//!
//! A line holds at most `width` printable characters: when one more arrives the line
//! breaks at the last space (the space becomes `\r\n`), or before the character when the
//! line has no space. ANSI sequences do not count toward the column; cursor moves
//! (`H`, `f`, `C`, `D`, `G`) adjust it; `\r` starts a new line; `\n` and DEL are ignored;
//! a backspace takes one column back. A line therefore never fills all 80 columns of a
//! terminal (`MBBSEmu.Tests SessionBase_Tests.cs:82`).
//!
//! Wrapping was switched on and off around specific displays, not left on (it is off
//! at realm entry), so the renderer applies it only to the
//! messages that ask (`Msg::wrap`).

use super::ansi::ESC;

/// The width the game wrapped at, for its displays and its lists alike.
pub const WRAPCOLS: u8 = 79;

/// Wraps `bytes` at `width` columns; width 0 means no wrapping (`btutsw` with 0).
pub fn wrap(bytes: &[u8], width: u8) -> Vec<u8> {
    if width == 0 {
        return bytes.to_vec();
    }
    let mut w = Wrapper::new(width);
    w.push(bytes);
    w.finish()
}

/// A printable character on the current line and where it sits in the output. `raw`
/// is `None` when the byte is no longer in the output (taken by [`Wrapper::take`]) or
/// never was (a column skipped by a cursor move); such a cell is never a break point.
#[derive(Clone, Copy)]
struct Cell {
    byte: u8,
    raw: Option<usize>,
}

enum State {
    Normal,
    Escape,
    Params { values: Vec<u32>, acc: Option<u32> },
}

/// The wrapper's state across pushes: the current line's printable characters and the
/// ANSI parser. One per output stream, like the host's per-session `LineBreaker`.
pub struct Wrapper {
    width: usize,
    out: Vec<u8>,
    line: Vec<Cell>,
    state: State,
}

impl Wrapper {
    /// A wrapper for `width` columns (at least 1).
    pub fn new(width: u8) -> Wrapper {
        Wrapper {
            width: usize::from(width.max(1)),
            out: Vec::new(),
            line: Vec::new(),
            state: State::Normal,
        }
    }

    /// Feeds bytes through the wrapper.
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.out.push(b);
            match std::mem::replace(&mut self.state, State::Normal) {
                State::Normal => self.normal(b),
                State::Escape => {
                    if b == b'[' {
                        self.state = State::Params {
                            values: Vec::new(),
                            acc: None,
                        };
                    }
                    // any other byte after ESC is consumed with it
                }
                State::Params { mut values, acc } => match b {
                    b'0'..=b'9' => {
                        let d = u32::from(b - b'0');
                        let acc = acc.unwrap_or(0).saturating_mul(10).saturating_add(d);
                        self.state = State::Params {
                            values,
                            acc: Some(acc),
                        };
                    }
                    b';' => {
                        values.push(acc.unwrap_or(0));
                        self.state = State::Params { values, acc: None };
                    }
                    b if b.is_ascii_alphabetic() => {
                        if let Some(a) = acc {
                            values.push(a);
                        }
                        self.cursor(b, &values);
                    }
                    _ => self.state = State::Params { values, acc },
                },
            }
        }
    }

    /// Takes the output accumulated so far; the line state carries on.
    pub fn take(&mut self) -> Vec<u8> {
        for cell in &mut self.line {
            // the cells now describe bytes that are gone
            cell.raw = None;
        }
        std::mem::take(&mut self.out)
    }

    /// Ends the stream and returns everything.
    pub fn finish(mut self) -> Vec<u8> {
        self.line.clear();
        std::mem::take(&mut self.out)
    }

    fn normal(&mut self, b: u8) {
        match b {
            b'\r' => self.line.clear(),
            ESC => self.state = State::Escape,
            b'\n' | 0x7F => {}
            0x08 => {
                self.line.pop();
            }
            _ => {
                self.line.push(Cell {
                    byte: b,
                    raw: Some(self.out.len() - 1),
                });
                if self.line.len() > self.width {
                    self.line_break(b);
                }
            }
        }
    }

    /// Cursor moves that change the column (`processAnsiCommand`).
    fn cursor(&mut self, cmd: u8, values: &[u32]) {
        let col = |v: u32| usize::try_from(v).unwrap_or(usize::MAX);
        match (cmd, values) {
            (b'H' | b'f', []) => self.set_column(0),
            (b'H' | b'f', [_, c]) => self.set_column(col(*c).saturating_sub(1)),
            (b'C', [n]) => self.set_column(self.line.len().saturating_add(col(*n))),
            (b'D', [n]) => self.set_column(self.line.len().saturating_sub(col(*n))),
            (b'G', [c]) => self.set_column(col(*c).saturating_sub(1)),
            _ => {}
        }
    }

    fn set_column(&mut self, column: usize) {
        if column <= self.line.len() {
            self.line.truncate(column);
        } else {
            // columns skipped by a cursor move: never a break point
            let pad = column.min(self.width).saturating_sub(self.line.len());
            self.line
                .extend(std::iter::repeat_n(Cell { byte: 0, raw: None }, pad));
        }
    }

    fn is_space(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | 0x0B | 0x0C)
    }

    /// The line just grew past the width with `b` (`doLineBreak`).
    fn line_break(&mut self, b: u8) {
        let last = self.out.len() - 1;
        if Wrapper::is_space(b) {
            // the overflowing space itself becomes the line end
            self.out[last] = b'\r';
            self.out.push(b'\n');
            self.line.clear();
            return;
        }
        let breakable = |c: &Cell| Wrapper::is_space(c.byte) && c.raw.is_some();
        match self.line.iter().rposition(breakable) {
            None => {
                // no space: break before this character
                self.out[last] = b'\r';
                self.out.push(b'\n');
                self.out.push(b);
                self.line.clear();
                self.line.push(Cell {
                    byte: b,
                    raw: Some(self.out.len() - 1),
                });
            }
            Some(space) => {
                // the last space becomes \r\n and the word after it starts the next line
                let at = self.line[space]
                    .raw
                    .expect("breakable cells are in the output");
                self.out[at] = b'\r';
                self.out.insert(at + 1, b'\n');
                let tail: Vec<Cell> = self.line[space + 1..]
                    .iter()
                    .map(|c| Cell {
                        byte: c.byte,
                        raw: c.raw.map(|r| r + 1),
                    })
                    .collect();
                self.line = tail;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(bytes: &[u8]) -> Vec<String> {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .split("\r\n")
            .map(str::to_string)
            .collect()
    }

    // MBBSEmu.Tests SessionBase_Tests.cs:20-24 (width 80)
    #[test]
    fn breaks_at_the_last_space() {
        let s = wrap(
            b"Testing one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen\r\n",
            80,
        );
        assert_eq!(
            lines(&s),
            [
                "Testing one two three four five six seven eight nine ten eleven twelve thirteen",
                "fourteen fifteen sixteen",
                "",
            ]
        );
    }

    // MBBSEmu.Tests SessionBase_Tests.cs:46-51: a word longer than the width
    #[test]
    fn a_word_longer_than_the_width_is_cut() {
        let s = wrap(
            b"012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789 testing\r\n",
            80,
        );
        assert_eq!(
            lines(&s),
            [
                "01234567890123456789012345678901234567890123456789012345678901234567890123456789",
                "0123456789 testing",
                "",
            ]
        );
    }

    // MBBSEmu.Tests SessionBase_Tests.cs:95-100
    #[test]
    fn edge_cases_at_width_four() {
        assert_eq!(
            lines(&wrap(b"test one two three\r\n", 4)),
            ["test", "one", "two", "thre", "e", ""]
        );
        assert_eq!(
            lines(&wrap(b"test\r\none two three\r\n", 4)),
            ["test", "one", "two", "thre", "e", ""]
        );
        assert_eq!(
            wrap(b"test one two three\r\n", 0),
            b"test one two three\r\n"
        );
    }

    #[test]
    fn cursor_moves_change_the_column() {
        // ESC[79D at the start of a message takes the column back to 0
        let s = wrap(b"\x1B[79D\x1B[Kabc def\r\n", 6);
        assert_eq!(lines(&s), ["\x1B[79D\x1B[Kabc", "def", ""]);
        // ESC[5C skips five columns, so "ab" + 5 + "cd" overflows a width of 8 at the d
        let s = wrap(b"ab\x1B[5Ccd\r\n", 8);
        assert_eq!(lines(&s), ["ab\x1B[5Cc", "d", ""]);
        // ESC[1;3H puts the cursor in column 3 (1-based)
        let s = wrap(b"abcdef\x1B[1;3Hxyz\r\n", 4);
        assert_eq!(lines(&s), ["abcd", "ef\x1B[1;3Hxy", "z", ""]);
    }

    #[test]
    fn backspace_and_ignored_bytes() {
        assert_eq!(
            wrap(b"abc\x08\x08\x08def\n\x7F\r\n", 4),
            b"abc\x08\x08\x08def\n\x7F\r\n"
        );
    }

    #[test]
    fn state_carries_across_pushes() {
        let mut w = Wrapper::new(6);
        w.push(b"abc ");
        let first = w.take();
        w.push(b"defgh\r\n");
        let second = w.finish();
        assert_eq!(first, b"abc ");
        // the space of the first push is gone, so the break lands before the overflowing
        // character instead of at that space
        assert_eq!(second, b"de\r\nfgh\r\n");
    }
}
