//! `strip_ansi`: the host's `stpans()` for log lines and non-ANSI sessions, with the
//! regex the pattern registry uses (`tools/differential/vendor/play/patterns.py`): `ESC [ [0-9;?]* letter`,
//! `ESC ( x` / `ESC ) x` character set selections, and any other `ESC` with the one byte
//! after it.

use super::ansi::ESC;

/// Removes every ANSI escape sequence from `bytes`. Backspaces, bells and the rest of
/// the text are kept; the registry's `raw_to_lines` deals with those separately.
pub fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != ESC {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes.get(i) {
            // ESC [ params letter; without the letter only `ESC [` goes (the regex's
            // `ESC .` fallback), so the parameters stay as text
            Some(b'[') => {
                let mut end = i + 1;
                while end < bytes.len() && matches!(bytes[end], b'0'..=b'9' | b';' | b'?') {
                    end += 1;
                }
                i = if end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                    end + 1
                } else {
                    i + 1
                };
            }
            // ESC ( x, ESC ) x: a character-set selection
            Some(b'(' | b')') => {
                i += 1;
                if i < bytes.len() && (bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit()) {
                    i += 1;
                }
            }
            // a lone ESC takes the next byte with it
            Some(_) => i += 1,
            None => {}
        }
    }
    out
}

/// [`strip_ansi`] for text.
pub fn strip_ansi_str(text: &str) -> String {
    String::from_utf8(strip_ansi(text.as_bytes())).expect("stripping removes only ASCII bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_clrline_prefix_and_colours() {
        let s = strip_ansi(b"\x1B[79D\x1B[K\x1B[1;31;40mYou have been killed!\r\n");
        assert_eq!(s, b"You have been killed!\r\n");
    }

    #[test]
    fn strips_sets_and_lone_escapes() {
        assert_eq!(strip_ansi(b"a\x1B(Bb\x1B)0c\x1BMd\x1B[?25le"), b"abcde");
    }

    #[test]
    fn keeps_the_rest() {
        assert_eq!(strip_ansi(b"x\x07 \x08y\xC3"), b"x\x07 \x08y\xC3");
        assert_eq!(strip_ansi(b"tail\x1B"), b"tail");
    }

    // as in the registry's regex: a CSI without its final letter falls back to `ESC .`
    #[test]
    fn truncated_csi_keeps_its_parameters() {
        assert_eq!(strip_ansi(b"a\x1B[79"), b"a79");
        assert_eq!(strip_ansi(b"a\x1B[1;"), b"a1;");
        assert_eq!(strip_ansi(b"a\x1B["), b"a");
    }
}
