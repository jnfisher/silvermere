//! 79-column word wrap: what the host's word wrap (`btutsw`) did to the output of the
//! displays that asked for it (room descriptions, lists and the like).
//!
//! Before each byte, and at the end of the text, a line that has more than `width - 1`
//! columns breaks at its last space (the space becomes `\r\n`); the word after it starts
//! the next line. A line with no space is never broken. `\r` starts a new line, and so does
//! `\n`, the host's soft CR (`btuscr`), unless the line has been broken since the last line
//! start: then it is a space. BEL takes no column; every other byte, a backspace included,
//! takes one. `ESC [` sequences take no columns, and one that does not end in `m` starts
//! the line again.
//! Columns start at 0 on every call: each message is wrapped on its own.
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
    let limit = usize::from(width) - 1;
    let mut out = Vec::with_capacity(bytes.len() + 8);
    let mut column = 0usize;
    // the columns since the last space
    let mut word = 0usize;
    // where the line's last space is in `out`
    let mut space: Option<usize> = None;
    let mut escape = false;
    // broken since the line started: a soft CR is a space
    let mut wrapped = false;
    for i in 0..=bytes.len() {
        if column > limit
            && let Some(at) = space
        {
            // once broken, the same space stays the break point
            if out[at] == b' ' {
                out[at] = b'\r';
                out.insert(at + 1, b'\n');
            }
            column = word;
            wrapped = true;
        }
        let Some(&b) = bytes.get(i) else {
            break;
        };
        out.push(b);
        if escape {
            if b >= b'A' && b != b'[' {
                escape = false;
                if b != b'm' {
                    (column, word, space, wrapped) = (0, 0, None, false);
                }
            }
            continue;
        }
        match b {
            ESC if bytes.get(i + 1) == Some(&b'[') => escape = true,
            b'\r' => (column, word, space, wrapped) = (0, 0, None, false),
            b'\n' if !wrapped => (column, word, space) = (0, 0, None),
            b' ' | b'\n' => {
                // a soft CR after a break goes as a space
                *out.last_mut().expect("just pushed") = b' ';
                word = 0;
                space = Some(out.len() - 1);
                column += 1;
            }
            0x07 => {}
            _ => {
                column += 1;
                word += 1;
            }
        }
    }
    out
}

/// Output recorded under MBBSEmu (the test captures) with its word wrap redone as
/// the game's host wraps ([`wrap`]). The two differ on one case: a 79-column line followed
/// by a space keeps its last word under MBBSEmu, and the game moves it to the next line.
/// A line MBBSEmu broke is joined to the next: a full line (its line end was the space that
/// overflowed it, or a cut when it has no space) or one the next line's first word would not
/// have fitted after. Each joined run is wrapped again.
pub fn from_mbbsemu(raw: &[u8]) -> Vec<u8> {
    let width = usize::from(WRAPCOLS);
    // the columns a piece takes: ANSI colours none, a cursor move starts again
    let columns = |piece: &[u8]| {
        let (mut n, mut escape) = (0usize, false);
        for (i, &b) in piece.iter().enumerate() {
            if escape {
                if b.is_ascii_alphabetic() {
                    escape = false;
                    if b != b'm' {
                        n = 0;
                    }
                }
            } else if b == 0x1B && piece.get(i + 1) == Some(&b'[') {
                escape = true;
            } else {
                n += 1;
            }
        }
        n
    };
    // whether MBBSEmu broke `line` before `next`; a full line's line end was the space
    // that overflowed it (or, with no space in it, a cut)
    let broke = |line: &[u8], next: &[u8]| {
        let used = columns(line);
        let first = next.split(|&b| b == b' ').next().unwrap_or_default();
        used == width
            || (used < width
                && !next.starts_with(b" ")
                && !first.is_empty()
                && used + 1 + columns(first) > width)
    };
    let lines = split_crlf(raw);
    let last = lines.len() - 1;
    let mut out = Vec::with_capacity(raw.len());
    let mut run: Vec<u8> = Vec::new();
    let mut joined = false;
    for (i, line) in lines.iter().enumerate() {
        run.extend_from_slice(line);
        // the piece after the last line end is never joined to
        if i + 1 < last && broke(line, lines[i + 1]) {
            if line.contains(&b' ') || columns(line) < width {
                run.push(b' ');
            }
            joined = true;
            continue;
        }
        // only what MBBSEmu wrapped is wrapped again
        if joined {
            out.extend(wrap(&run, WRAPCOLS));
        } else {
            out.extend_from_slice(&run);
        }
        run.clear();
        joined = false;
        if i < last {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

/// `raw` cut at each CR LF (the last piece is what follows the last one).
fn split_crlf(raw: &[u8]) -> Vec<&[u8]> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + 1 < raw.len() {
        if raw[i] == b'\r' && raw[i + 1] == b'\n' {
            pieces.push(&raw[start..i]);
            i += 2;
            start = i;
        } else {
            i += 1;
        }
    }
    pieces.push(&raw[start..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(text: &str, width: u8) -> String {
        String::from_utf8(wrap(text.as_bytes(), width))
            .unwrap()
            .replace("\r\n", "|")
    }

    /// Each line the host's word wrap was given, one call each, and what it sent.
    fn check(width: u8, cases: &[(String, String)]) {
        for (text, want) in cases {
            assert_eq!(&wrapped(text, width), want, "{text:?}");
        }
    }

    fn r(c: char, n: usize) -> String {
        c.to_string().repeat(n)
    }

    // recorded from the original host at the game's width
    #[test]
    fn the_hosts_wrap_at_79() {
        let (a, x, c) = (r('a', 70), r('x', 72), r('c', 76));
        check(
            79,
            &[
                (
                    format!("{a} bcdefgh next words\r\n"),
                    format!("{a} bcdefgh|next words|"),
                ),
                // a space as the 80th character: the break is at the space before
                (format!("{x} yyyyyy z\r\n"), format!("{x}|yyyyyy z|")),
                (format!("{x} yyyyyy\r\n"), format!("{x}|yyyyyy|")),
                // 79 columns at the end of the text break too
                (format!("{x} yyyyyy"), format!("{x}|yyyyyy")),
                (format!("{x} yyyyyy tail\r\n"), format!("{x}|yyyyyy tail|")),
                // no space: never cut
                (
                    format!("{} r\r\n", r('q', 85)),
                    format!("{}|r|", r('q', 85)),
                ),
                (
                    format!("{} {}\r\n", r('y', 60), r('z', 30)),
                    format!("{}|{}|", r('y', 60), r('z', 30)),
                ),
                // a backspace takes a column; BEL does not, but still breaks
                (
                    format!("ab {c}\x08\x08 d\r\n"),
                    format!("ab|{c}\x08\x08|d|"),
                ),
                // with LF the soft CR the player's channel has, an LF after a break is a
                // space (recorded without it, this kept the LF and no second break)
                (format!("ab {c}\n\x07 d\r\n"), format!("ab|{c} \x07|d|")),
                // a colour takes none; a cursor move or a line clear starts the line again
                (
                    format!("ab {c}\x1B[1;31m d\x1B[0m\r\n"),
                    format!("ab|{c}\x1B[1;31m d\x1B[0m|"),
                ),
                (
                    format!("ab {}\x1B[79D{} f\r\n", r('c', 60), r('e', 30)),
                    format!("ab {}\x1B[79D{} f|", r('c', 60), r('e', 30)),
                ),
                (
                    format!("ab {}\x1B[K{} f\r\n", r('c', 60), r('e', 30)),
                    format!("ab {}\x1B[K{} f|", r('c', 60), r('e', 30)),
                ),
                // each call starts at column 0
                (
                    format!("{} {}", r('g', 40), r('h', 30)),
                    format!("{} {}", r('g', 40), r('h', 30)),
                ),
                (format!(" {}\r\n", r('i', 20)), format!(" {}|", r('i', 20))),
            ],
        );
    }

    // recorded from the original host at width 10
    #[test]
    fn the_hosts_wrap_at_10() {
        let cases: Vec<(String, String)> = [
            ("abcd efghi", "abcd|efghi"),
            ("jk lm\r\n", "jk lm|"),
            ("abcde fghij\r\n", "abcde|fghij|"),
            ("abcde fghijk\r\n", "abcde|fghijk|"),
            ("abcde fghij klm nop\r\n", "abcde|fghij klm|nop|"),
            ("a bcdefghijklmnopq rs\r\n", "a|bcdefghijklmnopq|rs|"),
            ("abc def gh i\r\n", "abc def|gh i|"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        check(10, &cases);
    }

    #[test]
    fn width_zero_is_no_wrap() {
        assert_eq!(
            wrap(b"test one two three\r\n", 0),
            b"test one two three\r\n"
        );
    }

    #[test]
    fn mbbsemu_wraps_become_the_games() {
        let x = "x".repeat(72);
        // a space as the 80th character: MBBSEmu keeps "yyyyyy", the game moves it down
        let mbbsemu = format!("{x} yyyyyy\r\nz\r\n[HP=1]:");
        assert_eq!(
            from_mbbsemu(mbbsemu.as_bytes()),
            format!("{x}\r\nyyyyyy z\r\n[HP=1]:").into_bytes()
        );
        // lines MBBSEmu did not wrap are left alone, however long
        let long = format!("{x}{x}\r\nshort\r\n");
        assert_eq!(from_mbbsemu(long.as_bytes()), long.into_bytes());
        // a break the next word explains is redone the same way
        let same = format!("{} b\r\ncccccccc d\r\n", "a".repeat(70));
        assert_eq!(from_mbbsemu(same.as_bytes()), same.into_bytes());
    }
}
