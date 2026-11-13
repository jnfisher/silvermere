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
}
