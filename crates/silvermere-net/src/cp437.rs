//! CP437 input decoding.
//!
//! Interim home of the CP437 table for input only: the transport hands the engine raw line
//! bytes and the engine decodes them with [`decode`]. Output is already CP437 bytes (rendered
//! by `silvermere-text`). When T-010 lands its codec in `silvermere-text`, this module becomes
//! a re-export of that one.

/// The characters for bytes 0x80..=0xFF (the upper half of code page 437).
const HIGH: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00A2}', '\u{00A3}', '\u{00A5}', '\u{20A7}', '\u{0192}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    '\u{00BF}', '\u{2310}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{255C}', '\u{255B}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{255E}', '\u{255F}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256B}',
    '\u{256A}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{258C}', '\u{2590}', '\u{2580}',
    '\u{03B1}', '\u{00DF}', '\u{0393}', '\u{03C0}', '\u{03A3}', '\u{03C3}', '\u{00B5}', '\u{03C4}',
    '\u{03A6}', '\u{0398}', '\u{03A9}', '\u{03B4}', '\u{221E}', '\u{03C6}', '\u{03B5}', '\u{2229}',
    '\u{2261}', '\u{00B1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00F7}', '\u{2248}',
    '\u{00B0}', '\u{2219}', '\u{00B7}', '\u{221A}', '\u{207F}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

/// Decodes one CP437 byte. Bytes below 0x80 are ASCII and map to themselves (the line
/// editor never delivers control bytes, so their CP437 glyph forms do not matter here).
pub fn decode_byte(byte: u8) -> char {
    if byte < 0x80 {
        char::from(byte)
    } else {
        HIGH[usize::from(byte - 0x80)]
    }
}

/// Decodes a CP437 byte string (a received line) into UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().copied().map(decode_byte).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_identity() {
        assert_eq!(decode(b"Kasimir st"), "Kasimir st");
    }

    #[test]
    fn high_half_is_code_page_437() {
        assert_eq!(decode_byte(0xC9), '\u{2554}');
        assert_eq!(decode_byte(0x80), '\u{00C7}');
        assert_eq!(decode_byte(0xFF), '\u{00A0}');
    }
}
