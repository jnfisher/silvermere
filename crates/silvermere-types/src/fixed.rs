//! Fixed-width DOS strings: NUL-terminated CP437 bytes to `String` and back.
//!
//! The records hold text in fixed `N`-byte fields. Reading stops at the first NUL (or at `N`
//! when the field is full); bytes 0x80-0xFF are CP437. Trailing spaces are kept: the original
//! strips them only in some places, so trimming is the caller's decision. Used by the
//! importer (T-008) and tests; the engine works with `String`.

/// A fixed-width text field of `N` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedStr<const N: usize>;

impl<const N: usize> FixedStr<N> {
    /// Decodes the field: the bytes up to the first NUL (at most `N`), CP437 to Unicode.
    pub fn decode(bytes: &[u8]) -> String {
        let field = &bytes[..bytes.len().min(N)];
        let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
        field[..end].iter().map(|&b| cp437_to_char(b)).collect()
    }

    /// Encodes `text` into the field: Unicode to CP437, NUL-padded to `N`.
    ///
    /// Text longer than `N` bytes is cut at `N`; a field filled to `N` has no NUL, as the
    /// original allows. Characters CP437 cannot hold become `?`; a NUL inside `text` ends it.
    pub fn encode(text: &str) -> [u8; N] {
        let mut out = [0u8; N];
        for (slot, ch) in out
            .iter_mut()
            .zip(text.chars().map(char_to_cp437).take_while(|&b| b != 0))
        {
            *slot = ch;
        }
        out
    }
}

/// The text without trailing spaces (what the host's `depad` does).
pub fn depad(text: &str) -> &str {
    text.trim_end_matches(' ')
}

/// CP437 0x80-0xFF.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}', //
];

/// One CP437 byte as a `char`. Bytes below 0x80 are ASCII (control bytes stay controls).
pub fn cp437_to_char(byte: u8) -> char {
    if byte < 0x80 {
        char::from(byte)
    } else {
        CP437_HIGH[usize::from(byte - 0x80)]
    }
}

/// One `char` as a CP437 byte, or `?` when CP437 has no such character.
pub fn char_to_cp437(ch: char) -> u8 {
    if ch.is_ascii() {
        return ch as u8;
    }
    CP437_HIGH
        .iter()
        .position(|&c| c == ch)
        .map_or(b'?', |i| 0x80 + i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_half_round_trips() {
        for b in 0u8..=255 {
            assert_eq!(char_to_cp437(cp437_to_char(b)), b);
        }
    }
}
