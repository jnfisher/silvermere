//! The cursor tracking of the host's video library (`VIDAPI.C`), as far as the template
//! scan needs it to learn where each field lands on screen and in what colour.

/// Screen columns.
const SCRWIDTH: u8 = 80;
/// Screen rows.
const SCRHEIGHT: u8 = 25;

/// ANSI parsing state (`ansifls`): escape received.
const ANSIER: u8 = 2;
/// Waiting for the first digit.
const ANSIWD: u8 = 4;
/// Accumulating a number.
const ANSIAN: u8 = 8;

/// The set-graphics-rendition AND masks.
const ANDMSK: [u8; 50] = [
    0x00, 0xFF, 0xF7, 0xFF, 0xF8, 0xFF, 0xFF, 0xF8, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF8, 0xF8,
    0xF8, 0xF8, 0xF8, 0xF8, 0xF8, 0xF8, 0xFF, 0xFF, 0x8F, 0x8F, 0x8F, 0x8F, 0x8F, 0x8F, 0x8F, 0x8F,
    0xFF, 0xFF,
];
/// The set-graphics-rendition OR bits.
const ORBITS: [u8; 50] = [
    0x07, 0x08, 0x00, 0x00, 0x01, 0x80, 0x80, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04,
    0x02, 0x06, 0x01, 0x05, 0x03, 0x07, 0x00, 0x00, 0x00, 0x40, 0x20, 0x60, 0x10, 0x50, 0x30, 0x70,
    0x00, 0x00,
];

/// Cursor and attribute of an 80x25 window that text is "printed" to.
#[derive(Clone, Debug)]
pub(crate) struct Screen {
    /// Current attribute.
    pub(crate) attr: u8,
    ansifls: u8,
    ansicnt: usize,
    ansiarg: [u8; 20],
    ulx: u8,
    uly: u8,
    lrx: u8,
    lry: u8,
    /// Cursor, `(row << 8) | col` as in the `dx` register.
    dx: u16,
}

impl Screen {
    /// The window the template scan sets up: whole screen, cursor home, attribute 7.
    pub(crate) fn new() -> Self {
        Self {
            attr: 0x07,
            ansifls: 0,
            ansicnt: 0,
            ansiarg: [0; 20],
            ulx: 0,
            uly: 0,
            lrx: SCRWIDTH - 1,
            lry: SCRHEIGHT - 1,
            dx: 0,
        }
    }

    /// Cursor column.
    pub(crate) fn col(&self) -> u8 {
        (self.dx & 0xFF) as u8
    }

    /// Cursor row.
    pub(crate) fn row(&self) -> u8 {
        (self.dx >> 8) as u8
    }

    /// "Prints" text.
    pub(crate) fn print(&mut self, text: &[u8]) {
        for &ch in text {
            self.put(ch);
        }
    }

    /// Prints one byte (as `outbyt`).
    fn put(&mut self, ch: u8) {
        if self.ansifls & (ANSIER | ANSIWD | ANSIAN) != 0 {
            self.escape_byte(ch);
        } else if ch == 0x1B {
            self.ansifls |= ANSIER;
            self.ansicnt = 0;
            self.ansiarg[0] = 1;
            self.ansiarg[1] = 1;
        } else {
            match ch {
                b'\n' => self.newline(),
                b'\r' => self.dx = (self.dx & 0xFF00) | u16::from(self.ulx),
                0x0C => self.dx = (u16::from(self.uly) << 8) | u16::from(self.ulx),
                0x07 => {}
                0x08 => self.back(),
                _ => {
                    if self.col() == self.lrx {
                        self.newline();
                    } else {
                        self.dx += 1;
                    }
                }
            }
        }
    }

    /// Handles a byte of an escape sequence (as `pransi`).
    fn escape_byte(&mut self, ch: u8) {
        if self.ansifls & ANSIAN != 0 {
            if ch.is_ascii_digit() {
                let arg = &mut self.ansiarg[self.ansicnt];
                *arg = arg.wrapping_mul(10).wrapping_add(ch - b'0');
            } else {
                self.ansicnt += 1;
                if ch == b';' {
                    self.ansifls &= !ANSIAN;
                } else {
                    self.run_escape(ch);
                }
            }
        } else if self.ansifls & ANSIWD != 0 {
            if ch == b';' {
                self.ansicnt += 1;
            } else if !ch.is_ascii_digit() {
                self.run_escape(ch);
            } else {
                self.ansiarg[self.ansicnt] = ch - b'0';
                self.ansifls |= ANSIAN;
            }
        } else {
            match ch {
                b'[' => self.ansifls |= ANSIWD,
                b'D' => {
                    self.ansiarg[0] = 1;
                    self.run_escape(b'B');
                }
                b'M' => {
                    self.ansiarg[0] = 1;
                    self.run_escape(b'A');
                }
                _ => {
                    self.ansifls = 0;
                    self.put(ch);
                }
            }
        }
    }

    /// Executes an escape sequence's command; only what moves the cursor or sets the
    /// attribute (as `doansi`).
    fn run_escape(&mut self, ch: u8) {
        self.ansifls = 0;
        match ch & 0xDF {
            b'M' => {
                if self.ansicnt == 0 {
                    self.ansicnt = 1;
                    self.ansiarg[0] = 0;
                    self.ansiarg[1] = 1;
                }
                for i in 0..self.ansicnt {
                    let arg = usize::from(self.ansiarg[i]);
                    if arg < ANDMSK.len() {
                        self.attr &= ANDMSK[arg];
                        self.attr |= ORBITS[arg];
                    }
                }
            }
            b'A' => {
                let row = (self.dx >> 8) as i16 - i16::from(self.ansiarg[0]);
                self.dx = ((row as u16) << 8) | (self.dx & 0xFF);
                self.keep_inside();
            }
            b'B' => {
                let row = (self.dx >> 8) + u16::from(self.ansiarg[0]);
                self.dx = (row << 8) | (self.dx & 0xFF);
                self.keep_inside();
            }
            b'C' => {
                self.dx += u16::from(self.ansiarg[0]);
                self.keep_inside();
            }
            b'D' => {
                self.dx = self.dx.wrapping_sub(u16::from(self.ansiarg[0]));
                self.keep_inside();
            }
            b'J' => {
                if self.ansicnt != 0 && self.ansiarg[0] == 2 {
                    self.dx = (u16::from(self.uly) << 8) | u16::from(self.ulx);
                }
            }
            b'H' | b'F' => {
                if self.ansiarg[0] == 0 {
                    self.ansiarg[0] = 1;
                }
                if self.ansicnt > 1 && self.ansiarg[1] == 0 {
                    self.ansiarg[1] = 1;
                }
                let row = u16::from(self.ansiarg[0]) + u16::from(self.uly) - 1;
                let col = if self.ansicnt > 1 {
                    u16::from(self.ansiarg[1]) + u16::from(self.ulx) - 1
                } else {
                    u16::from(self.ulx)
                };
                self.dx = (row << 8) | col;
                self.keep_inside();
            }
            _ => {}
        }
    }

    /// Keeps the cursor inside the window.
    fn keep_inside(&mut self) {
        let row = self.row().clamp(self.uly, self.lry);
        let col = self.col().clamp(self.ulx, self.lrx);
        self.dx = (u16::from(row) << 8) | u16::from(col);
    }

    /// Backspace: a column left, to the end of the row above from the first column, not
    /// past the top left (as `doback`).
    fn back(&mut self) {
        if self.col() == self.ulx {
            if self.row() == self.uly {
                return;
            }
            self.dx = ((u16::from(self.row()) - 1) << 8) | u16::from(self.lrx);
        } else {
            self.dx -= 1;
        }
    }

    /// Column 0 and down a row, staying on the bottom row.
    fn newline(&mut self) {
        self.dx = (self.dx & 0xFF00) | u16::from(self.ulx);
        let row = self.row();
        if row < self.lry {
            self.dx = ((u16::from(row) + 1) << 8) | (self.dx & 0xFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Screen;

    #[test]
    fn text_and_newlines_move_the_cursor() {
        let mut s = Screen::new();
        s.print(b"abc\r\nde");
        assert_eq!((s.row(), s.col()), (1, 2));
        // CR returns to the first column only, LF starts a new line
        s.print(b"\rx\ny");
        assert_eq!((s.row(), s.col()), (2, 1));
        // backspace from the first column goes to the end of the row above
        s.print(b"\x08\x08\x07");
        assert_eq!((s.row(), s.col()), (1, 79));
    }

    #[test]
    fn cursor_position_and_colour_sequences() {
        let mut s = Screen::new();
        s.print(b"\x1B[5;10H\x1B[1;31m");
        assert_eq!((s.row(), s.col()), (4, 9));
        assert_eq!(s.attr, 0x0C);
    }

    #[test]
    fn moves_stay_inside_the_window() {
        let mut s = Screen::new();
        s.print(b"\x1B[99C\x1B[99B");
        assert_eq!((s.row(), s.col()), (24, 79));
    }
}
