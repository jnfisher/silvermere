//! The Worldgroup 3.x layouts: the byte-packed layouts with 4-byte alignment applied, the
//! lengths of a Worldgroup realm's files, and the two corrections (`updrec`, `actmon`).

use silvermere_types::records::layout::{self, DOS_DATA, WORLDGROUP};
use silvermere_types::records::{Field, Layout, LayoutSet};

/// The record lengths of a Worldgroup 3.3 realm's files, as Btrieve opens them.
const LENGTHS: [(&str, usize); 16] = [
    ("player", 2028),
    ("room", 1544),
    ("item", 1072),
    ("shop", 504),
    ("bankbook", 76),
    ("itemown", 72),
    ("monster", 756),
    ("actmon", 400),
    ("spell", 260),
    ("class", 156),
    ("race", 126),
    ("mudmsg", 256),
    ("gang", 256),
    ("action", 1010),
    ("textblk", 2024),
    ("updrec", 2032),
];

fn is_pad(f: &Field) -> bool {
    f.name.starts_with("pad_")
}

/// The alignment rule on a byte-packed layout: every member moved up to a multiple of its
/// element size (at most 4), the end to a multiple of the largest. Returns the members'
/// offsets and the length.
fn aligned(dos: &Layout) -> (Vec<(&'static str, usize)>, usize) {
    aligned_fields(dos.fields)
}

/// [`aligned`] over `fields` in place of the layout's own.
fn aligned_fields(fields: &[Field]) -> (Vec<(&'static str, usize)>, usize) {
    let (mut at, mut most) = (0usize, 1);
    let mut out = Vec::new();
    for f in fields {
        let a = f.elem.min(4);
        most = most.max(a);
        at = at.next_multiple_of(a);
        out.push((f.name, at));
        at += f.size;
    }
    (out, at.next_multiple_of(most))
}

#[test]
fn lengths_match_a_worldgroup_realm() {
    for ((name, len), wg) in LENGTHS.iter().zip(WORLDGROUP) {
        assert_eq!(wg.decoder, *name);
        assert_eq!(wg.len, *len, "{name}");
    }
}

#[test]
fn every_layout_covers_its_record_with_named_padding() {
    for wg in WORLDGROUP {
        let mut at = 0;
        for f in wg.fields {
            assert_eq!(
                f.offset, at,
                "{}.{} starts where the last ended",
                wg.decoder, f.name
            );
            assert_eq!(f.size % f.elem, 0, "{}.{}", wg.decoder, f.name);
            if !is_pad(f) {
                assert_eq!(
                    f.offset % f.elem.min(4),
                    0,
                    "{}.{} aligned",
                    wg.decoder,
                    f.name
                );
            } else {
                assert_eq!(f.name, format!("pad_{:#05x}", f.offset), "pad name");
                assert!(f.size < 4, "{}.{}", wg.decoder, f.name);
            }
            at += f.size;
        }
        assert_eq!(at, wg.len, "{} members cover the record", wg.decoder);
    }
}

#[test]
fn members_are_the_byte_packed_ones_in_the_same_order() {
    for (dos, wg) in DOS_DATA.iter().zip(WORLDGROUP) {
        assert_eq!(dos.decoder, wg.decoder);
        let members: Vec<_> = wg.fields.iter().filter(|f| !is_pad(f)).collect();
        assert_eq!(members.len(), dos.fields.len(), "{}", dos.decoder);
        for (d, w) in dos.fields.iter().zip(members) {
            let size = if (dos.decoder, d.name) == ("updrec", "data") {
                2024
            } else {
                d.size
            };
            assert_eq!(
                (w.name, w.size, w.elem),
                (d.name, size, d.elem),
                "{}",
                dos.decoder
            );
        }
    }
}

#[test]
fn offsets_follow_the_alignment_rule() {
    for (dos, wg) in DOS_DATA.iter().zip(WORLDGROUP) {
        let (members, len) = aligned(dos);
        let got: Vec<_> = wg
            .fields
            .iter()
            .filter(|f| !is_pad(f))
            .map(|f| (f.name, f.offset))
            .collect();
        match dos.decoder {
            // The payload is aligned to 4: the union of the aligned records.
            "updrec" => {
                assert_eq!(got, [("seqnum", 0), ("type", 4), ("data", 8)]);
                assert_eq!(len, 2028);
            }
            // `inactive` takes 4 bytes: the rule on that layout.
            "actmon" => {
                assert_eq!((len, wg.len), (396, 400));
                let fields: Vec<Field> = dos
                    .fields
                    .iter()
                    .map(|f| match f.name {
                        "inactive" => Field {
                            size: 4,
                            elem: 4,
                            ..*f
                        },
                        _ => *f,
                    })
                    .collect();
                let (want, len) = aligned_fields(&fields);
                assert_eq!(got, want, "actmon");
                assert_eq!(len, 400);
                let at = |n| wg.field(n).unwrap().offset;
                assert_eq!(
                    (at("inactive"), at("flags"), at("unk_0x122")),
                    (0x124, 0x128, 0x12A)
                );
                assert_eq!((at("montype"), at("hpregen")), (0x12C, 0x130));
                assert_eq!((at("unk_0x131"), at("grouprole")), (0x13C, 0x148));
                assert_eq!(wg.field("pad_0x125").unwrap().size, 3);
            }
            _ => {
                assert_eq!(got, members, "{}", dos.decoder);
                assert_eq!(len, wg.len, "{}", dos.decoder);
            }
        }
    }
}

#[test]
fn the_text_block_fixed_part_is_24_bytes() {
    let t = &layout::WG_TEXTBLK;
    assert_eq!(t.field("number").unwrap().offset, 0x10);
    assert_eq!(t.field("link").unwrap().offset, 0x14);
    assert_eq!(t.field("text").unwrap().offset, 0x18);
}

#[test]
fn the_set_picks_the_layout() {
    for (dos, wg) in DOS_DATA.iter().zip(WORLDGROUP) {
        assert_eq!(LayoutSet::Dos.layout(dos), *dos);
        assert_eq!(LayoutSet::Worldgroup.layout(dos), wg);
    }
}
