//! The ANSI screen strings and the if-ANSI resolver.
//!
//! The original wrote every colour change as a Galacticomm "if-ANSI" string,
//! `ESC [ [ <ansi> | <plain> ]`: the host sent the ANSI half
//! to ANSI users and the plain half (space/backspace pairs that print nothing) to the
//! others. silvermere sessions are always ANSI, so the constants below are the ANSI
//! halves already resolved, and [`resolve_if_ansi`] turns any raw if-ANSI text that
//! still reaches the renderer (a `.MSG` text, a test) into the same bytes.
//!
//! This module has no dependencies on the rest of the crate: `build.rs` includes it by
//! path to validate the macro names the catalogue uses.

/// The escape byte.
pub const ESC: u8 = 0x1B;

// The twenty common macros, resolved to their ANSI half.

/// `ANSI_CLRLINE`: column 1, erase the line. Starts nearly every message.
pub const ANSI_CLRLINE: &[u8] = b"\x1B[79D\x1B[K";
/// `ANSI_NORMAL`: grey on black, not bright.
pub const ANSI_NORMAL: &[u8] = b"\x1B[0;37;40m";
/// `ANSI_LTRED`: bright red.
pub const ANSI_LTRED: &[u8] = b"\x1B[1;31m";
/// `ANSI_LTREDBLK`: bright red on black.
pub const ANSI_LTREDBLK: &[u8] = b"\x1B[1;31;40m";
/// `ANSI_GREY`: grey (normal white).
pub const ANSI_GREY: &[u8] = b"\x1B[0;37m";
/// `ANSI_BROWN`: brown (dark yellow).
pub const ANSI_BROWN: &[u8] = b"\x1B[0;33m";
/// `ANSI_WHITE`: bright white.
pub const ANSI_WHITE: &[u8] = b"\x1B[1;37m";
/// `ANSI_GREEN`: green.
pub const ANSI_GREEN: &[u8] = b"\x1B[0;32m";
/// `ANSI_CYAN`: cyan.
pub const ANSI_CYAN: &[u8] = b"\x1B[0;36m";
/// `ANSI_YELLOW`: yellow (bright).
pub const ANSI_YELLOW: &[u8] = b"\x1B[1;33m";
/// `ANSI_LTBLUE`: bright blue.
pub const ANSI_LTBLUE: &[u8] = b"\x1B[1;34m";
/// `ANSI_MAGENTA`: magenta.
pub const ANSI_MAGENTA: &[u8] = b"\x1B[0;35m";
/// `ANSI_LTCYAN`: bright cyan.
pub const ANSI_LTCYAN: &[u8] = b"\x1B[1;36m";
/// `ANSI_RED`: red (normal intensity).
pub const ANSI_RED: &[u8] = b"\x1B[0;31m";
/// `ANSI_BOLD`: bright on, colour kept.
pub const ANSI_BOLD: &[u8] = b"\x1B[1m";
/// `ANSI_FGRED`: red, intensity kept.
pub const ANSI_FGRED: &[u8] = b"\x1B[31m";
/// `ANSI_FGBROWN`: brown/yellow, intensity kept.
pub const ANSI_FGBROWN: &[u8] = b"\x1B[33m";
/// `ANSI_FGCYAN`: cyan, intensity kept.
pub const ANSI_FGCYAN: &[u8] = b"\x1B[36m";
/// `ANSI_FGGREY`: grey/white, intensity kept.
pub const ANSI_FGGREY: &[u8] = b"\x1B[37m";
/// `ANSI_FGGREEN`: green, intensity kept.
pub const ANSI_FGGREEN: &[u8] = b"\x1B[32m";

// The colour table's own colours.

/// `ANSI_LTMAGENTA`: bright magenta.
pub const ANSI_LTMAGENTA: &[u8] = b"\x1B[1;35m";
/// `ANSI_BLKONWHT`: black text on a white background, two if-ANSI
/// strings joined into one literal.
pub const ANSI_BLKONWHT: &[u8] = b"\x1B[0;30m\x1B[47m";
/// `ANSI_BLKBACK`: background back to black, text colour kept.
pub const ANSI_BLKBACK: &[u8] = b"\x1B[40m";
/// `NOCOLOR`: a table entry that sends nothing.
pub const NOCOLOR: &[u8] = b"";

// Other macros the catalogue needs.

/// `ANSI_FGMAGENTA`: magenta text, intensity kept.
pub const ANSI_FGMAGENTA: &[u8] = b"\x1B[35m";
/// `ANSI_DKGREY` (also spelt `ANSI_DKGRAY`): dark grey text.
pub const ANSI_DKGREY: &[u8] = b"\x1B[1;30m";
/// `ANSI_RESET`: all attributes off.
pub const ANSI_RESET: &[u8] = b"\x1B[0m";
/// `ANSI_BGRED`: red background.
pub const ANSI_BGRED: &[u8] = b"\x1B[41m";
/// `ANSI_BGBLACK`: black background.
pub const ANSI_BGBLACK: &[u8] = b"\x1B[40m";
/// `ANSI_FGBLUE`: blue, intensity kept.
pub const ANSI_FGBLUE: &[u8] = b"\x1B[34m";

/// `PROMPT_SYSOP`: the sysop menu prompt (plain ANSI, not if-ANSI).
pub const PROMPT_SYSOP: &[u8] = b"\x1B[1;30m[\x1B[0mMAJORMUD Sysop\x1B[1;30m]\x1B[0m: ";

/// Every macro by its catalogue name, for the catalogue's `{@NAME}` references and for tests.
pub const BY_NAME: &[(&str, &[u8])] = &[
    ("ANSI_CLRLINE", ANSI_CLRLINE),
    ("ANSI_NORMAL", ANSI_NORMAL),
    ("ANSI_LTRED", ANSI_LTRED),
    ("ANSI_LTREDBLK", ANSI_LTREDBLK),
    ("ANSI_GREY", ANSI_GREY),
    ("ANSI_BROWN", ANSI_BROWN),
    ("ANSI_WHITE", ANSI_WHITE),
    ("ANSI_GREEN", ANSI_GREEN),
    ("ANSI_CYAN", ANSI_CYAN),
    ("ANSI_YELLOW", ANSI_YELLOW),
    ("ANSI_LTBLUE", ANSI_LTBLUE),
    ("ANSI_MAGENTA", ANSI_MAGENTA),
    ("ANSI_LTCYAN", ANSI_LTCYAN),
    ("ANSI_RED", ANSI_RED),
    ("ANSI_BOLD", ANSI_BOLD),
    ("ANSI_FGRED", ANSI_FGRED),
    ("ANSI_FGBROWN", ANSI_FGBROWN),
    ("ANSI_FGCYAN", ANSI_FGCYAN),
    ("ANSI_FGGREY", ANSI_FGGREY),
    ("ANSI_FGGREEN", ANSI_FGGREEN),
    ("ANSI_LTMAGENTA", ANSI_LTMAGENTA),
    ("ANSI_BLKONWHT", ANSI_BLKONWHT),
    ("ANSI_BLKBACK", ANSI_BLKBACK),
    ("NOCOLOR", NOCOLOR),
    ("ANSI_FGMAGENTA", ANSI_FGMAGENTA),
    ("ANSI_DKGREY", ANSI_DKGREY),
    ("ANSI_RESET", ANSI_RESET),
    ("ANSI_BGRED", ANSI_BGRED),
    ("ANSI_BGBLACK", ANSI_BGBLACK),
    ("ANSI_FGBLUE", ANSI_FGBLUE),
    ("PROMPT_SYSOP", PROMPT_SYSOP),
];

/// Looks a macro up by its catalogue name.
pub fn by_name(name: &str) -> Option<&'static [u8]> {
    BY_NAME.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Resolves every if-ANSI string in `input` to its ANSI half.
///
/// `ESC [ [ <ansi> | <plain> ]` becomes `<ansi>`; the `|`, the plain half and the
/// closing `]` are dropped, as the host did for an ANSI user. An opener without a
/// `|` keeps everything up to `]`; an unterminated string keeps the rest of the input.
/// Text without if-ANSI strings is returned unchanged.
pub fn resolve_if_ansi(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == ESC && input.get(i + 1) == Some(&b'[') && input.get(i + 2) == Some(&b'[') {
            i += 3;
            // the ANSI half, up to '|' or ']'
            while i < input.len() && input[i] != b'|' && input[i] != b']' {
                out.push(input[i]);
                i += 1;
            }
            // the plain half, skipped
            if i < input.len() && input[i] == b'|' {
                while i < input.len() && input[i] != b']' {
                    i += 1;
                }
            }
            // the closing ']'
            if i < input.len() {
                i += 1;
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out
}

/// [`resolve_if_ansi`] for text. Both halves of an if-ANSI string are ASCII, so the
/// result is as valid as the input.
pub fn resolve_if_ansi_str(input: &str) -> String {
    String::from_utf8(resolve_if_ansi(input.as_bytes()))
        .expect("if-ANSI resolution only removes ASCII bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ANSI_CLRLINE
    #[test]
    fn clrline_resolves_to_its_ansi_half() {
        let raw = b"\x1B[[\x1B[79D\x1B[K| \x08 \x08 \x08 \x08]";
        assert_eq!(resolve_if_ansi(raw), ANSI_CLRLINE);
    }

    // ANSI_BLKONWHT: two if-ANSI strings back to back
    #[test]
    fn adjacent_strings_resolve_one_after_the_other() {
        let raw = b"\x1B[[\x1B[0;30m| \x08 \x08 \x08 \x08]\x1B[[\x1B[47m| \x08 \x08 \x08]";
        assert_eq!(resolve_if_ansi(raw), ANSI_BLKONWHT);
    }

    #[test]
    fn plain_text_and_plain_ansi_pass_through() {
        let text = b"\x1B[1;31mYou have been killed!\r";
        assert_eq!(resolve_if_ansi(text), text);
    }

    #[test]
    fn unterminated_string_keeps_the_rest() {
        assert_eq!(resolve_if_ansi(b"\x1B[[\x1B[1m| \x08"), b"\x1B[1m");
        assert_eq!(resolve_if_ansi(b"ab\x1B[[\x1B[1m"), b"ab\x1B[1m");
    }

    #[test]
    fn every_name_is_unique() {
        for (i, (a, _)) in BY_NAME.iter().enumerate() {
            assert!(
                BY_NAME[i + 1..].iter().all(|(b, _)| a != b),
                "{a} listed twice"
            );
        }
        assert_eq!(by_name("ANSI_LTRED"), Some(ANSI_LTRED));
        assert_eq!(by_name("ANSI_NOPE"), None);
    }
}
