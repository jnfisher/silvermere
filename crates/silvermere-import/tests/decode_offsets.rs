//! The decoders against the `silvermere-types` layouts (the `FIELD_OFFSETS` contract):
//! every member of every record lands on its own bytes.
//!
//! For each of the 15 record types: the decoder's member list is the layout's, name for
//! name and in order; an image with every member set to a distinct value (built from the
//! layout's offsets and sizes) decodes, and each decoded member equals the decode of an
//! image where only that member is set, so no member reads another's bytes; encoding
//! gives the image back. Then the semantic checks: text trimming, the WCCTEXT shift, the
//! coin orders, and a proptest that any buffer of a record's length decodes.

use proptest::prelude::*;
use serde_json::Value;
use silvermere_import::decode::{self, Record, textblk};
use silvermere_types::records::Layout;

/// A byte for offset `o` that is printable, not a space and never NUL, and differs
/// between neighbouring offsets, so every member's image is distinct and text members
/// are full-width.
fn pattern(o: usize) -> u8 {
    0x21 + ((o * 31 + 7) % 94) as u8
}

/// An image where only the members in `which` (layout indices) hold the pattern.
fn image(layout: &Layout, which: impl Fn(usize) -> bool) -> Vec<u8> {
    let mut out = vec![0u8; layout.len];
    for (i, f) in layout.fields.iter().enumerate() {
        if which(i) {
            for (o, b) in out[f.offset..f.offset + f.size].iter_mut().enumerate() {
                *b = pattern(f.offset + o);
            }
        }
    }
    out
}

fn check<R: Record>() {
    let layout = R::LAYOUT;
    let names: Vec<&str> = R::FIELDS.iter().map(|f| f.0).collect();
    let expected: Vec<&str> = layout.fields.iter().map(|f| f.name).collect();
    assert_eq!(names, expected, "struct {}: member list", layout.decoder);
    for (&(name, size), f) in R::FIELDS.iter().zip(layout.fields) {
        assert_eq!(size, f.size, "struct {}.{name}: size", layout.decoder);
    }

    let full = image(layout, |_| true);
    let decoded = R::decode(&full).unwrap();
    assert_eq!(
        decoded.encode(),
        full,
        "struct {}: round trip",
        layout.decoder
    );
    let json = decoded.to_json();
    let zero = R::decode(&vec![0; layout.len]).unwrap().to_json();
    for (i, f) in layout.fields.iter().enumerate() {
        let alone = R::decode(&image(layout, |j| j == i)).unwrap().to_json();
        for (name, value) in &alone {
            let want = if name == f.name {
                &json[name]
            } else {
                &zero[name]
            };
            assert_eq!(
                value, want,
                "struct {}: setting only {} changed {name}",
                layout.decoder, f.name
            );
        }
        assert_ne!(
            json[f.name], zero[f.name],
            "struct {}.{}: the pattern did not register",
            layout.decoder, f.name
        );
    }
    assert!(matches!(
        R::decode(&full[..layout.len - 1]),
        Err(decode::DecodeError::Length { .. })
    ));
}

// `player` .. `textblk`: the 15 record types (updrec is T-009).
#[test]
fn every_member_decodes_from_its_own_bytes() {
    check::<decode::PlayerRecord>();
    check::<decode::RoomRecord>();
    check::<decode::ItemRecord>();
    check::<decode::ShopRecord>();
    check::<decode::BankbookRecord>();
    check::<decode::ItemownRecord>();
    check::<decode::MonsterRecord>();
    check::<decode::ActmonRecord>();
    check::<decode::SpellRecord>();
    check::<decode::ClassRecord>();
    check::<decode::RaceRecord>();
    check::<decode::MudmsgRecord>();
    check::<decode::GangRecord>();
    check::<decode::ActionRecord>();
    check::<decode::TextblkRecord>();
}

#[test]
fn signed_and_unsigned_members_follow_the_layout() {
    // exittypes[] is signed 16-bit, mapnum unsigned 32-bit.
    let mut raw = vec![0u8; decode::RoomRecord::LAYOUT.len];
    let f = |n| decode::RoomRecord::LAYOUT.field(n).unwrap().offset;
    raw[f("exittypes")..f("exittypes") + 2].copy_from_slice(&(-3i16).to_le_bytes());
    raw[f("mapnum")..f("mapnum") + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    raw[f("maxmonsters")] = 0xFF; // unsigned char
    let r = decode::RoomRecord::decode(&raw).unwrap();
    assert_eq!(r.exittypes[0], -3);
    assert_eq!(r.mapnum, u32::MAX);
    assert_eq!(r.maxmonsters, 255);
    // level is unsigned 32-bit (mmud_data reads it as <I too).
    let mut raw = vec![0u8; decode::MonsterRecord::LAYOUT.len];
    let at = decode::MonsterRecord::LAYOUT.field("level").unwrap().offset;
    raw[at..at + 4].copy_from_slice(&0x8000_0001u32.to_le_bytes());
    assert_eq!(
        decode::MonsterRecord::decode(&raw).unwrap().level,
        0x8000_0001
    );
}

#[test]
fn text_members_cut_at_nul_and_trim_like_t006() {
    let mut raw = vec![0u8; decode::MudmsgRecord::LAYOUT.len];
    let at = decode::MudmsgRecord::LAYOUT.field("msgs").unwrap().offset;
    raw[at..at + 9].copy_from_slice(b"  Hi \x82 \0x");
    let r = decode::MudmsgRecord::decode(&raw).unwrap();
    // The flat record keeps the text up to the NUL as the game sees it; JSON and the schema
    // get it trimmed (mmud_data._s strips both ends).
    assert_eq!(r.msgs[0].0, "  Hi \u{e9} ");
    assert_eq!(r.to_json()["msgs"][0], Value::from("Hi \u{e9}"));
    assert_eq!(decode::to_message(&r).msgs[0], "Hi \u{e9}");
}

// The game's text block decoding: shift down by 0x20, end at a stored NUL or space.
#[test]
fn text_blocks_shift_down_and_end_at_a_stored_nul_or_space() {
    let mut stored = vec![0u8; 2000];
    stored[..6].copy_from_slice(b"\x2a\x68\x69\x20\x41\x00");
    assert_eq!(textblk::decode_text(&stored), "\nHI");
    stored[3] = 0x41;
    assert_eq!(textblk::decode_text(&stored), "\nHI!!");
    // At most TEXTSIZE-1 bytes: the game forces the last text byte to NUL.
    let full = vec![0x61u8; 2000];
    assert_eq!(textblk::decode_text(&full).len(), 1999);
    // A short variable-length record is zero-filled to the 2022-byte buffer.
    let mut short = vec![0u8; 22];
    short[0x0E] = 7;
    short.extend_from_slice(b"\x68\x69");
    let r = textblk::decode_textblk(&short).unwrap();
    assert_eq!(r.number, 7);
    assert_eq!(decode::to_text_block(&r).text, "HI");
}

// cash[5] is runic first; the game drops cash[0] as runic.
#[test]
fn monster_cash_is_runic_first() {
    let mut raw = vec![0u8; decode::MonsterRecord::LAYOUT.len];
    let at = decode::MonsterRecord::LAYOUT.field("cash").unwrap().offset;
    for (i, v) in [1u32, 2, 3, 4, 5].iter().enumerate() {
        raw[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    let clock = decode::LegacyClock::utc_now();
    let (m, _) =
        decode::split_monster(&decode::MonsterRecord::decode(&raw).unwrap(), &clock).unwrap();
    assert_eq!(
        (
            m.cash.runic,
            m.cash.platinum,
            m.cash.gold,
            m.cash.silver,
            m.cash.copper
        ),
        (1, 2, 3, 4, 5)
    );
}

// An item's cash[0] goes to the player's runic, cash[4] to the player's copper.
#[test]
fn item_cash_is_runic_first() {
    let mut raw = vec![0u8; decode::ItemRecord::LAYOUT.len];
    let at = decode::ItemRecord::LAYOUT.field("cash").unwrap().offset;
    raw[at..at + 4].copy_from_slice(&9i32.to_le_bytes());
    raw[at + 16..at + 20].copy_from_slice(&4i32.to_le_bytes());
    let item = decode::to_item(&decode::ItemRecord::decode(&raw).unwrap());
    assert_eq!((item.cash.runic, item.cash.copper), (9, 4));
}

// Room coins are named fields, runic..copper.
#[test]
fn room_coins_are_named() {
    let mut raw = vec![0u8; decode::RoomRecord::LAYOUT.len];
    let at = |n| decode::RoomRecord::LAYOUT.field(n).unwrap().offset;
    raw[at("copper")..at("copper") + 4].copy_from_slice(&11u32.to_le_bytes());
    raw[at("hidrunic")..at("hidrunic") + 4].copy_from_slice(&3u32.to_le_bytes());
    let (room, state) = decode::split_room(&decode::RoomRecord::decode(&raw).unwrap());
    assert_eq!(room.coins.copper, 11);
    assert_eq!(room.hidcoins.runic, 3);
    assert_eq!(state.coins, room.coins);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_room_sized_buffer_decodes(raw in proptest::collection::vec(any::<u8>(), 1528)) {
        let r = decode::RoomRecord::decode(&raw).unwrap();
        let _ = decode::split_room(&r);
        let _ = r.to_json();
    }

    #[test]
    fn any_buffer_of_each_length_decodes(seed in proptest::collection::vec(any::<u8>(), 2022)) {
        macro_rules! each {
            ($($t:ty),*) => { $(
                let r = <$t>::decode(&seed[..<$t>::LAYOUT.len]).unwrap();
                prop_assert_eq!(r.encode().len(), <$t>::LAYOUT.len);
            )* };
        }
        each!(
            decode::ItemRecord, decode::ShopRecord, decode::BankbookRecord,
            decode::ItemownRecord, decode::MonsterRecord, decode::ActmonRecord,
            decode::SpellRecord, decode::ClassRecord, decode::RaceRecord,
            decode::MudmsgRecord, decode::GangRecord, decode::ActionRecord,
            decode::TextblkRecord
        );
        let _ = decode::to_text_block(&decode::decode_textblk(&seed[..100]).unwrap());
    }
}

fn dos_date(y: u16, m: u16, d: u16) -> u16 {
    ((y - 1980) << 9) | (m << 5) | d
}

fn dos_time(h: u16, m: u16, s: u16) -> u16 {
    (h << 11) | (m << 5) | (s / 2)
}

fn put_u16(raw: &mut [u8], layout: &silvermere_types::records::Layout, field: &str, v: u16) {
    let at = layout.field(field).unwrap().offset;
    raw[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

// killdate/killtime: one moment in the legacy zone (PR #14).
#[test]
fn monster_kill_date_and_time_become_unix_seconds_in_the_legacy_zone() {
    let layout = decode::MonsterRecord::LAYOUT;
    let mut raw = vec![0u8; layout.len];
    put_u16(&mut raw, layout, "killdate", dos_date(1997, 3, 14));
    put_u16(&mut raw, layout, "killtime", dos_time(13, 45, 30));
    let r = decode::MonsterRecord::decode(&raw).unwrap();
    // The flat record keeps the raw DOS values (they are what T-006 exports).
    assert_eq!(r.killdate, dos_date(1997, 3, 14));
    let utc = decode::LegacyClock::new(jiff::tz::TimeZone::UTC, jiff::Timestamp::UNIX_EPOCH);
    let (m, state) = decode::split_monster(&r, &utc).unwrap();
    assert_eq!(m.killed_at, 858_347_130);
    assert_eq!(state.killed_at, 858_347_130);
    let est = decode::LegacyClock::new(
        jiff::tz::TimeZone::fixed(jiff::tz::offset(-5)),
        jiff::Timestamp::UNIX_EPOCH,
    );
    let (m, _) = decode::split_monster(&r, &est).unwrap();
    assert_eq!(m.killed_at, 858_347_130 + 5 * 3600);
    // Zero is never.
    let zero = decode::MonsterRecord::decode(&vec![0u8; layout.len]).unwrap();
    assert_eq!(decode::split_monster(&zero, &utc).unwrap().0.killed_at, 0);
}

// An active monster's `created` is set from `now()` when it is generated: a time only,
// read on the import day.
#[test]
fn active_monster_created_is_a_time_on_the_import_day() {
    let layout = decode::ActmonRecord::LAYOUT;
    let mut raw = vec![0u8; layout.len];
    put_u16(&mut raw, layout, "created", dos_time(13, 45, 30));
    let r = decode::ActmonRecord::decode(&raw).unwrap();
    // The import happens on 1997-03-14 UTC.
    let now: jiff::Timestamp = "1997-03-14T20:00:00Z".parse().unwrap();
    let clock = decode::LegacyClock::new(jiff::tz::TimeZone::UTC, now);
    assert_eq!(clock.today, dos_date(1997, 3, 14));
    assert_eq!(
        decode::to_active_monster(&r, &clock).unwrap().created,
        858_347_130
    );
}

#[test]
fn an_invalid_dos_date_is_an_error_naming_the_record() {
    let layout = decode::GangRecord::LAYOUT;
    let mut raw = vec![0u8; layout.len];
    let key = layout.field("key").unwrap().offset;
    raw[key..key + 4].copy_from_slice(b"BAND");
    put_u16(&mut raw, layout, "created", dos_date(1997, 2, 30));
    let r = decode::GangRecord::decode(&raw).unwrap();
    let err = decode::to_gang(&r, &decode::LegacyClock::utc_now()).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "struct gang key BAND: created: invalid DOS date {:#06x}",
            dos_date(1997, 2, 30)
        )
    );
}
