//! The CP437 codec against MBBSEmu's `TextEncoding/CP437Converter.cs` table.

use silvermere_text::cp437::{
    CP437_TO_UNICODE, REPLACEMENT, decode, decode_byte, encode, encode_char,
};

#[test]
fn every_byte_round_trips() {
    let all: Vec<u8> = (0..=255u8).collect();
    let text = decode(&all);
    assert_eq!(text.chars().count(), 256);
    assert_eq!(encode(&text), all);
    for b in 0..=255u8 {
        assert_eq!(encode_char(decode_byte(b)), Some(b), "byte {b:#04x}");
    }
}

// the box-drawing bytes of the MBBSEmu banner
#[test]
fn banner_glyphs_encode_to_their_bytes() {
    assert_eq!(encode("├"), [0xC3]);
    assert_eq!(encode("█"), [0xDB]);
    assert_eq!(encode("▀"), [0xDF]);
    assert_eq!(encode("├█▀"), [0xC3, 0xDB, 0xDF]);
    assert_eq!(decode(&[0xC3, 0xDB, 0xDF]), "├█▀");
}

#[test]
fn a_character_outside_the_table_becomes_a_question_mark() {
    assert_eq!(REPLACEMENT, b'?');
    assert_eq!(encode("€"), b"?");
    assert_eq!(encode("a😀b"), b"a?b");
    assert_eq!(encode_char('ĸ'), None);
}

#[test]
fn table_corners() {
    assert_eq!(CP437_TO_UNICODE[0x80], 'Ç');
    assert_eq!(CP437_TO_UNICODE[0x9E], '₧');
    assert_eq!(CP437_TO_UNICODE[0xB0], '░');
    assert_eq!(CP437_TO_UNICODE[0xE1], 'ß');
    assert_eq!(CP437_TO_UNICODE[0xFE], '■');
    assert_eq!(CP437_TO_UNICODE[0xFF], '\u{A0}');
    // DC1-DC4 render as glyphs (MBBSEmu's UTF-8 mode); other controls pass through
    assert_eq!(decode(&[0x11, 0x12, 0x13, 0x14]), "◄↕‼¶");
    assert_eq!(decode(&[0x1B, 0x07, 0x08, 0x7F]), "\u{1B}\u{7}\u{8}\u{7F}");
    assert_eq!(encode("\u{1B}[0m"), b"\x1B[0m");
}
