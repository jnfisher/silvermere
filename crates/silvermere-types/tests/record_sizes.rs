//! Record lengths and byte layouts match the committed facts.

use crate::facts;

use silvermere_types::ids::RoomKey;
use silvermere_types::records::*;

/// (record name, layout, RECORD_LEN, expected length)
fn records() -> Vec<(&'static str, &'static Layout, usize, usize)> {
    vec![
        ("player", Character::LAYOUT, Character::RECORD_LEN, 1998),
        (
            "permrec",
            PermanentRecord::LAYOUT,
            PermanentRecord::RECORD_LEN,
            205,
        ),
        ("evilrec", EvilRecord::LAYOUT, EvilRecord::RECORD_LEN, 57),
        ("roomkey", RoomKey::LAYOUT, RoomKey::RECORD_LEN, 8),
        ("room", Room::LAYOUT, Room::RECORD_LEN, 1528),
        ("item", Item::LAYOUT, Item::RECORD_LEN, 1061),
        ("shop", Shop::LAYOUT, Shop::RECORD_LEN, 500),
        ("bankbook", BankBook::LAYOUT, BankBook::RECORD_LEN, 72),
        (
            "itemown",
            ItemOwnership::LAYOUT,
            ItemOwnership::RECORD_LEN,
            65,
        ),
        (
            "monster",
            KnownMonster::LAYOUT,
            KnownMonster::RECORD_LEN,
            732,
        ),
        (
            "actmon",
            ActiveMonster::LAYOUT,
            ActiveMonster::RECORD_LEN,
            386,
        ),
        ("spell", Spell::LAYOUT, Spell::RECORD_LEN, 253),
        ("class", Class::LAYOUT, Class::RECORD_LEN, 153),
        ("race", Race::LAYOUT, Race::RECORD_LEN, 126),
        ("mudmsg", Message::LAYOUT, Message::RECORD_LEN, 253),
        ("gang", Gang::LAYOUT, Gang::RECORD_LEN, 253),
        ("action", Action::LAYOUT, Action::RECORD_LEN, 1010),
        ("textblk", TextBlock::LAYOUT, TextBlock::RECORD_LEN, 2022),
        (
            "updrec",
            UpdateRecord::LAYOUT,
            UpdateRecord::RECORD_LEN,
            2027,
        ),
    ]
}

// Record lengths
#[test]
fn record_lengths() {
    for (name, layout, len, want) in records() {
        assert_eq!(len, want, "{name} RECORD_LEN");
        assert_eq!(layout.len, want, "{name} layout length");
        assert_eq!(layout.decoder, name);
        let mut at = 0;
        for f in layout.fields {
            assert_eq!(
                f.offset, at,
                "{name}.{} starts where the last ended",
                f.name
            );
            assert_eq!(f.size % f.elem, 0, "{name}.{}", f.name);
            at += f.size;
        }
        assert_eq!(at, want, "{name} fields cover the record");
    }
}

// The record length the game opens each data file with
#[test]
fn data_file_record_lengths_match() {
    let files = [
        ("WCCRACE.DAT", Race::RECORD_LEN),
        ("WCCCLASS.DAT", Class::RECORD_LEN),
        ("WCCUSERS.DAT", Character::RECORD_LEN),
        ("WCCMP001.DAT", Room::RECORD_LEN),
        ("WCCACMSR.DAT", ActiveMonster::RECORD_LEN),
        ("WCCSHOPS.DAT", Shop::RECORD_LEN),
        ("WCCITEMS.DAT", Item::RECORD_LEN),
        ("WCCITOWN.DAT", ItemOwnership::RECORD_LEN),
        ("WCCSPELS.DAT", Spell::RECORD_LEN),
        ("WCCKNMSR.DAT", KnownMonster::RECORD_LEN),
        ("WCCBANKS.DAT", BankBook::RECORD_LEN),
        ("WCCMSG.DAT", Message::RECORD_LEN),
        ("WCCTEXT.DAT", TextBlock::RECORD_LEN),
        ("WCCGANGS.DAT", Gang::RECORD_LEN),
        ("WCCACTS.DAT", Action::RECORD_LEN),
    ];
    let lengths = facts::facts().record_lengths;
    assert_eq!(lengths.len(), files.len());
    for (fact, (file, len)) in lengths.iter().zip(files) {
        assert_eq!(fact.file, file);
        assert_eq!(fact.record_len, len, "{file}");
    }
}

// Every member's offset and size.
#[test]
fn field_offsets_match_the_facts() {
    let facts = facts::facts();
    for (name, layout, _, _) in records() {
        let members = &facts.structs[name];
        assert_eq!(
            members.len(),
            layout.fields.len(),
            "record {name}: member count"
        );
        for (m, f) in members.iter().zip(layout.fields) {
            assert_eq!(
                (m.name.as_str(), m.offset, m.size, m.elem),
                (f.name, f.offset, f.size, f.elem),
                "record {name}"
            );
        }
        let total: usize = members.iter().map(|m| m.size).sum();
        assert_eq!(total, layout.len, "record {name}: sum of member sizes");
    }
    assert_eq!(facts.structs.len(), records().len());
}

#[test]
fn coins_reverse_the_runic_first_arrays() {
    let coins = Coins::from_runic_first([5, 4, 3, 2, 1]);
    assert_eq!(coins.runic, 5);
    assert_eq!(coins.copper, 1);
    assert_eq!(coins.to_runic_first(), [5, 4, 3, 2, 1]);
}
