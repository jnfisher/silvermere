//! `FixedStr` round trips: NUL-terminated CP437 fields to `String` and back.

use proptest::prelude::*;
use silvermere_types::fixed::{FixedStr, depad};

const N: usize = 30;

proptest! {
    // A field holding non-NUL CP437 bytes then NUL padding decodes and re-encodes to itself,
    // trailing spaces included.
    #[test]
    fn nul_terminated_fields_round_trip(text in prop::collection::vec(1u8..=255, 0..N)) {
        let mut field = [0u8; N];
        field[..text.len()].copy_from_slice(&text);
        let decoded = FixedStr::<N>::decode(&field);
        prop_assert_eq!(decoded.chars().count(), text.len());
        prop_assert_eq!(FixedStr::<N>::encode(&decoded), field);
    }

    // A field filled to the last byte has no NUL and still round-trips.
    #[test]
    fn full_fields_round_trip(text in prop::collection::vec(1u8..=255, N)) {
        let field: [u8; N] = text.try_into().unwrap();
        let decoded = FixedStr::<N>::decode(&field);
        prop_assert_eq!(FixedStr::<N>::encode(&decoded), field);
    }

    // Whatever follows the first NUL is ignored, as the original ignores it.
    #[test]
    fn bytes_after_the_nul_are_ignored(
        head in prop::collection::vec(1u8..=255, 0..N - 1),
        tail in prop::collection::vec(any::<u8>(), N),
    ) {
        let mut field = [0u8; N];
        field[..head.len()].copy_from_slice(&head);
        let rest = N - head.len() - 1;
        field[head.len() + 1..].copy_from_slice(&tail[..rest]);
        let mut clean = [0u8; N];
        clean[..head.len()].copy_from_slice(&head);
        prop_assert_eq!(FixedStr::<N>::decode(&field), FixedStr::<N>::decode(&clean));
    }
}

#[test]
fn trailing_spaces_are_kept_until_depad() {
    let field = FixedStr::<11>::encode("Bob   ");
    assert_eq!(&field, b"Bob   \0\0\0\0\0");
    let text = FixedStr::<11>::decode(&field);
    assert_eq!(text, "Bob   ");
    assert_eq!(depad(&text), "Bob");
}

#[test]
fn cp437_high_bytes_decode() {
    assert_eq!(FixedStr::<4>::decode(&[0x80, 0xB0, 0xE1, 0]), "Ç░ß");
    assert_eq!(FixedStr::<4>::encode("Ç░ß"), [0x80, 0xB0, 0xE1, 0]);
    assert_eq!(FixedStr::<4>::encode("€"), [b'?', 0, 0, 0]);
    assert_eq!(FixedStr::<3>::encode("abcdef"), *b"abc");
}
