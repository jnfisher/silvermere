//! The CP437 codec: the telnet wire encoding of the game's text.
//!
//! The table is MBBSEmu's `TextEncoding/CP437Converter.cs` (the standard CP437 mapping
//! for 0x80-0xFF; 0x00-0x7F pass through as controls and ASCII) with its UTF-8 mode's
//! exception kept: DC1-DC4 (0x11-0x14) render as the glyphs `◄ ↕ ‼ ¶`, because the
//! full-screen editor draws with those bytes.
//! Internal text is UTF-8; the transport encodes with [`encode`] for a CP437 session
//! and the renderer encodes catalogue text the same way, so every byte of a legacy
//! client's screen is one column.

/// `CP437_TO_UNICODE[b]`: the character shown for byte `b`.
pub const CP437_TO_UNICODE: [char; 256] = [
    '\u{00}', '\u{01}', '\u{02}', '\u{03}', '\u{04}', '\u{05}', '\u{06}', '\u{07}', '\u{08}',
    '\u{09}', '\u{0A}', '\u{0B}', '\u{0C}', '\u{0D}', '\u{0E}', '\u{0F}', // 0x00
    '\u{10}', '\u{25C4}', '\u{2195}', '\u{203C}', '\u{B6}', '\u{15}', '\u{16}', '\u{17}', '\u{18}',
    '\u{19}', '\u{1A}', '\u{1B}', '\u{1C}', '\u{1D}', '\u{1E}', '\u{1F}', // 0x10
    '\u{20}', '\u{21}', '\u{22}', '\u{23}', '\u{24}', '\u{25}', '\u{26}', '\u{27}', '\u{28}',
    '\u{29}', '\u{2A}', '\u{2B}', '\u{2C}', '\u{2D}', '\u{2E}', '\u{2F}', // 0x20
    '\u{30}', '\u{31}', '\u{32}', '\u{33}', '\u{34}', '\u{35}', '\u{36}', '\u{37}', '\u{38}',
    '\u{39}', '\u{3A}', '\u{3B}', '\u{3C}', '\u{3D}', '\u{3E}', '\u{3F}', // 0x30
    '\u{40}', '\u{41}', '\u{42}', '\u{43}', '\u{44}', '\u{45}', '\u{46}', '\u{47}', '\u{48}',
    '\u{49}', '\u{4A}', '\u{4B}', '\u{4C}', '\u{4D}', '\u{4E}', '\u{4F}', // 0x40
    '\u{50}', '\u{51}', '\u{52}', '\u{53}', '\u{54}', '\u{55}', '\u{56}', '\u{57}', '\u{58}',
    '\u{59}', '\u{5A}', '\u{5B}', '\u{5C}', '\u{5D}', '\u{5E}', '\u{5F}', // 0x50
    '\u{60}', '\u{61}', '\u{62}', '\u{63}', '\u{64}', '\u{65}', '\u{66}', '\u{67}', '\u{68}',
    '\u{69}', '\u{6A}', '\u{6B}', '\u{6C}', '\u{6D}', '\u{6E}', '\u{6F}', // 0x60
    '\u{70}', '\u{71}', '\u{72}', '\u{73}', '\u{74}', '\u{75}', '\u{76}', '\u{77}', '\u{78}',
    '\u{79}', '\u{7A}', '\u{7B}', '\u{7C}', '\u{7D}', '\u{7E}', '\u{7F}', // 0x70
    '\u{C7}', '\u{FC}', '\u{E9}', '\u{E2}', '\u{E4}', '\u{E0}', '\u{E5}', '\u{E7}', '\u{EA}',
    '\u{EB}', '\u{E8}', '\u{EF}', '\u{EE}', '\u{EC}', '\u{C4}', '\u{C5}', // 0x80
    '\u{C9}', '\u{E6}', '\u{C6}', '\u{F4}', '\u{F6}', '\u{F2}', '\u{FB}', '\u{F9}', '\u{FF}',
    '\u{D6}', '\u{DC}', '\u{A2}', '\u{A3}', '\u{A5}', '\u{20A7}', '\u{192}', // 0x90
    '\u{E1}', '\u{ED}', '\u{F3}', '\u{FA}', '\u{F1}', '\u{D1}', '\u{AA}', '\u{BA}', '\u{BF}',
    '\u{2310}', '\u{AC}', '\u{BD}', '\u{BC}', '\u{A1}', '\u{AB}', '\u{BB}', // 0xA0
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{255C}', '\u{255B}',
    '\u{2510}', // 0xB0
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{255E}', '\u{255F}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}',
    '\u{2567}', // 0xC0
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256B}',
    '\u{256A}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{258C}', '\u{2590}',
    '\u{2580}', // 0xD0
    '\u{3B1}', '\u{DF}', '\u{393}', '\u{3C0}', '\u{3A3}', '\u{3C3}', '\u{B5}', '\u{3C4}',
    '\u{3A6}', '\u{398}', '\u{3A9}', '\u{3B4}', '\u{221E}', '\u{3C6}', '\u{3B5}',
    '\u{2229}', // 0xE0
    '\u{2261}', '\u{B1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{F7}', '\u{2248}',
    '\u{B0}', '\u{2219}', '\u{B7}', '\u{221A}', '\u{207F}', '\u{B2}', '\u{25A0}',
    '\u{A0}', // 0xF0
];

/// The character for one byte.
pub const fn decode_byte(b: u8) -> char {
    CP437_TO_UNICODE[b as usize]
}

/// The byte for a character, or `None` when the table has no such character. Besides
/// the table's own characters, the four C0 controls that the table shows as glyphs
/// (U+0011 to U+0014) still encode to themselves, so a control byte round-trips as a
/// control too.
pub fn encode_char(c: char) -> Option<u8> {
    let cp = u32::from(c);
    if cp < 0x80 {
        return Some(cp as u8);
    }
    CP437_TO_UNICODE[0x80..]
        .iter()
        .position(|&t| t == c)
        .map(|i| (0x80 + i) as u8)
        .or_else(|| {
            CP437_TO_UNICODE[0x11..=0x14]
                .iter()
                .position(|&t| t == c)
                .map(|i| (0x11 + i) as u8)
        })
}

/// The byte sent for a character outside the table.
pub const REPLACEMENT: u8 = b'?';

/// Encodes UTF-8 text to CP437 bytes; characters outside the table become `?`.
pub fn encode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    encode_into(&mut out, text);
    out
}

/// [`encode`], appending to `out`.
pub fn encode_into(out: &mut Vec<u8>, text: &str) {
    out.extend(text.chars().map(|c| encode_char(c).unwrap_or(REPLACEMENT)));
}

/// Decodes CP437 bytes to text (the UTF-8 session mode of the transport).
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| decode_byte(b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_identity() {
        for b in 0u8..0x80 {
            if (0x11..=0x14).contains(&b) {
                continue;
            }
            assert_eq!(decode_byte(b), b as char);
            assert_eq!(encode_char(b as char), Some(b));
        }
    }

    #[test]
    fn glyph_controls_decode_as_glyphs_and_encode_both_ways() {
        assert_eq!(decode(&[0x11, 0x12, 0x13, 0x14]), "◄↕‼¶");
        assert_eq!(encode("◄↕‼¶"), [0x11, 0x12, 0x13, 0x14]);
        assert_eq!(encode("\u{11}\u{14}"), [0x11, 0x14]);
    }

    #[test]
    fn table_has_no_duplicate_glyphs() {
        for (i, a) in CP437_TO_UNICODE.iter().enumerate() {
            assert!(
                !CP437_TO_UNICODE[i + 1..].contains(a),
                "{a:?} appears twice"
            );
        }
    }
}
