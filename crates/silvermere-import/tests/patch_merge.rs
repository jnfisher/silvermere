//! The `WCCUPDAT.DAT` merge rules on synthetic old/new pairs (plan `07` Layer 1): for each
//! kind's merge, the members it copies change, the members it keeps stay, a missing key is
//! inserted, and two patches of one record apply
//! in `seqnum` order whatever their order in the file.

use silvermere_import::decode::{
    ActionRecord, ClassRecord, ItemRecord, MonsterRecord, MudmsgRecord, RaceRecord, Raw, Record,
    RoomRecord, ShopRecord, SpellRecord, Text, TextblkRecord,
};
use silvermere_import::importer::{Content, Decoded, ImportError};
use silvermere_import::patch::{
    Patch, PatchKind, apply_patch_list, merge_action, merge_class, merge_item, merge_message,
    merge_monster, merge_race, merge_room, merge_shop, merge_spell, merge_text,
};

/// A record of zeroes.
fn zero<R: Record>() -> R {
    R::decode(&vec![0; R::LAYOUT.len]).unwrap()
}

fn text<const N: usize>(s: &str) -> Text<N> {
    Text(s.to_string())
}

fn decoded<R>(file: &str, ordinal: u32, record: R) -> Decoded<R> {
    Decoded {
        file: file.to_string(),
        ordinal,
        record,
    }
}

/// An `updrec` carrying `record`.
fn patch<R: Record>(seq: i32, kind: PatchKind, record: &R) -> Patch {
    let mut image = record.encode();
    image.resize(2022, 0);
    Patch {
        seq,
        kind,
        image,
        carried: 2022,
    }
}

// ---------------------------------------------------------------------------------------
// Races and classes: compare a list, copy the whole record.

fn race(number: u16) -> RaceRecord {
    let mut r: RaceRecord = zero();
    r.number = number;
    r.name = text("Human");
    r.minstr = 30;
    r
}

// Race: one compared member differs -> whole record copied.
#[test]
fn race_update_copies_the_whole_record_when_a_compared_member_differs() {
    let mut old = race(1);
    let mut new = race(1);
    new.maxcha = 90;
    new.abilvals[3] = 5;
    new.unk_0x072 = Raw(vec![7; 12]);
    assert!(merge_race(&mut old, &new));
    assert_eq!(
        old, new,
        "every member, unk_0x072 included, comes from the update"
    );
}

// Members outside the compared list do not by themselves cause a copy.
#[test]
fn race_update_keeps_the_record_when_only_uncompared_members_differ() {
    let mut old = race(1);
    let mut new = race(1);
    new.unk_0x072 = Raw(vec![7; 12]);
    new.modified = 1;
    let before = old.clone();
    assert!(!merge_race(&mut old, &new));
    assert_eq!(old, before);
    // unk_0x05C is compared as NUL-terminated text: bytes after its NUL do not count.
    old.unk_0x05C.0 = vec![b'a', 0, 1, 1, 1, 1];
    new = old.clone();
    new.unk_0x05C.0 = vec![b'a', 0, 2, 2, 2, 2];
    assert!(!merge_race(&mut old, &new));
    new.unk_0x05C.0 = vec![b'b', 0, 0, 0, 0, 0];
    assert!(merge_race(&mut old, &new));
}

// Class.
#[test]
fn class_update_copies_the_whole_record_when_a_compared_member_differs() {
    let mut old: ClassRecord = zero();
    old.number = 3;
    old.name = text("Paladin");
    let mut new = old.clone();
    new.unk_0x069 = Raw(vec![9; 48]);
    assert!(!merge_class(&mut old, &new), "unk_0x069 is not compared");
    assert_eq!(old.unk_0x069.0, vec![0; 48]);
    new.titles = 12;
    assert!(merge_class(&mut old, &new));
    assert_eq!(old, new);
}

// ---------------------------------------------------------------------------------------
// Rooms.

fn room(map: u32, number: u32) -> RoomRecord {
    let mut r: RoomRecord = zero();
    r.mapnum = map;
    r.roomnum = number;
    r.name = text("Town Square");
    r
}

/// An old room with run-time state in every run-time member.
fn room_in_play() -> RoomRecord {
    let mut r = room(1, 10);
    r.monsters[0] = 55;
    r.nummonsters = 1;
    r.items[0] = 100;
    r.itemuses[0] = 3;
    r.itemcounts[0] = 2;
    r.copper = 40;
    r.hidgold = 2;
    r.owner = text("Somebody");
    r.lastkill = 7;
    r.foundmask = 0x3;
    r.areacount = 4;
    r.searchers = 2;
    r.hidcounts[0] = 1;
    r.flags = 0x08; // RF_SPECALIVE, run-time
    r
}

fn no_items(_: i32) -> Option<&'static ItemRecord> {
    None
}

// Room: the design members are copied.
#[test]
fn room_update_copies_the_design_members() {
    let mut old = room_in_play();
    let mut new = room(1, 10);
    new.name = text("Old Square");
    new.desc[2] = text("A fountain.");
    new.mapfile = text("MAP1.ANS");
    new.type_ = 2;
    new.shopnum = 5;
    new.minlevel = 1;
    new.maxlevel = 3;
    new.genmonster = 9;
    new.light = -50;
    new.ganghouse = 4;
    new.maxmonsters = 3;
    new.unk_0x555 = 6;
    new.mongroup = 2;
    new.deathroom = 11;
    new.huproom = 12;
    new.cmdtext = 13;
    new.unk_0x5AF = 14;
    new.regendelay = 15;
    new.areamax = 16;
    new.ctlroom = 17;
    new.specmon = 18;
    new.protmsg = 19;
    new.roomspell = 20;
    new.picktrap = 21;
    new.placed[0] = 300;
    new.exits[0] = 11;
    assert!(merge_room(&mut old, &new, no_items));
    // Kept from the old record: run-time state and everything the merge does not name.
    let src = room_in_play();
    let mut expect = new.clone();
    expect.monsters = src.monsters;
    expect.nummonsters = src.nummonsters;
    expect.items = src.items;
    expect.itemuses = src.itemuses;
    expect.itemcounts = src.itemcounts;
    expect.copper = src.copper;
    expect.hidgold = src.hidgold;
    expect.owner = src.owner.clone();
    expect.lastkill = src.lastkill;
    expect.foundmask = src.foundmask;
    expect.areacount = src.areacount;
    expect.searchers = src.searchers;
    expect.hidcounts = src.hidcounts;
    expect.flags = src.flags;
    assert_eq!(old, expect);
}

// Room: run-time state survives; an unchanged room is left alone.
#[test]
fn room_update_keeps_run_time_state_and_skips_an_unchanged_room() {
    let mut old = room_in_play();
    let mut new = room(1, 10);
    // Run-time members differ, design members do not: nothing to do.
    new.copper = 1;
    new.monsters[3] = 9;
    new.lastkill = 99;
    let before = old.clone();
    assert!(!merge_room(&mut old, &new, no_items));
    assert_eq!(old, before);
}

// Room: Protected, Patrol and Owned follow the update both ways; Gang House
// is cleared, never set; other bits stay.
#[test]
fn room_update_brings_three_flags_into_line_and_only_clears_gang_house() {
    let mut old = room(1, 10);
    old.flags = 0x01 | 0x40 | 0x08; // protected, gang house, specalive
    let mut new = room(1, 10);
    new.flags = 0x02 | 0x04 | 0x10; // patrol, owned, (another bit)
    assert!(merge_room(&mut old, &new, no_items));
    assert_eq!(old.flags, 0x02 | 0x04 | 0x08);

    let mut old = room(1, 10);
    let mut new = room(1, 10);
    new.flags = 0x40;
    assert!(
        !merge_room(&mut old, &new, no_items),
        "gang house alone is no change"
    );
    assert_eq!(old.flags, 0);
}

// Room exits, compare and copy: same-type exits keep their run-time
// parameter; doors, gates and keys are locked again; a new type takes all four.
#[test]
fn room_update_exit_parameters_by_type() {
    let mut old = room(1, 10);
    let mut new = room(1, 10);
    // 0 door, 1 key, 2 spell, 3 trap, 4 timed (same types); 5 normal -> hidden (new type)
    let types = [7, 2, 1, 9, 16, 0];
    for (i, t) in types.iter().enumerate() {
        old.exittypes[i] = *t;
        new.exittypes[i] = *t;
        old.exits[i] = 100 + i as u32;
        new.exits[i] = 100 + i as u32;
        old.exitpar1s[i] = 1;
        old.exitpar2s[i] = 1;
        old.exitpar3s[i] = 1;
        old.exitpar4s[i] = 1;
        new.exitpar1s[i] = 5;
        new.exitpar2s[i] = 5;
        new.exitpar3s[i] = 1;
        new.exitpar4s[i] = 1;
    }
    new.exittypes[5] = 6;
    // Only the run-time parameter of the door differs: not a change by itself.
    let mut door_only = room(1, 10);
    door_only.exittypes[0] = 7;
    let mut door_new = door_only.clone();
    door_only.exitpar1s[0] = 0;
    door_new.exitpar1s[0] = 2;
    assert!(!merge_room(&mut door_only, &door_new, no_items));
    assert_eq!(door_only.exitpar1s[0], 0);

    assert!(merge_room(&mut old, &new, no_items));
    // door: par1 locked (2), the rest from the update
    assert_eq!(
        (old.exitpar1s[0], old.exitpar2s[0]),
        (2, 5),
        "door relocked"
    );
    // key: par2 locked (2), par1 from the update
    assert_eq!((old.exitpar1s[1], old.exitpar2s[1]), (5, 2), "key relocked");
    // spell: par2 kept
    assert_eq!(
        (old.exitpar1s[2], old.exitpar2s[2]),
        (5, 1),
        "spell keeps par2"
    );
    // trap and timed: all four from the update
    assert_eq!((old.exitpar1s[3], old.exitpar2s[3]), (5, 5));
    assert_eq!((old.exitpar1s[4], old.exitpar2s[4]), (5, 5));
    // a changed type takes all four
    assert_eq!(
        (old.exittypes[5], old.exitpar1s[5], old.exitpar2s[5]),
        (6, 5, 5)
    );
}

fn item(number: i32, gettable: i16, limit: u16) -> ItemRecord {
    let mut i: ItemRecord = zero();
    i.number = number;
    i.gettable = gettable;
    i.limit = limit;
    i
}

// Room: a non-gettable hidden item of the update is put in the first free hidden slot unless present; looking for it empties slots of items that
// are gone or gettable without a limit; gettable hidden items of the update are not added.
#[test]
fn room_update_hidden_slot_rule() {
    let items = [
        item(1, 0, 0), // a lever: cannot be picked up
        item(2, 1, 0), // unlimited loot
        item(3, 1, 5), // limited loot
        item(4, 0, 0), // another fixture, already in the room
    ];
    let lookup = |n: i32| items.iter().find(|i| i.number == n);
    let mut old = room(1, 10);
    old.hiditems = [0; 15];
    old.hiduses = [-1; 15];
    old.hiditems[0] = 2; // unlimited loot: emptied while looking
    old.hiduses[0] = 9;
    old.hiditems[1] = 3; // limited loot: stays
    old.hiditems[2] = 77; // no such item: emptied
    old.hiditems[3] = 4; // the fixture the update also has
    let mut new = room(1, 10);
    new.hiditems[5] = 1;
    new.hiduses[5] = 6;
    new.hiditems[6] = 2; // gettable: not added
    new.hiditems[7] = 4;
    assert!(merge_room(&mut old, &new, lookup));
    // The lever went into the first free slot (slot 0, freed on the way) with its uses.
    assert_eq!(old.hiditems[..5], [1, 3, 0, 4, 0]);
    assert_eq!(old.hiduses[0], 6);
    assert_eq!(old.hiduses[2], -1);
    // Item 4 was already there: not added twice.
    assert_eq!(old.hiditems.iter().filter(|&&h| h == 4).count(), 1);
}

// Room: the search stops at the slot that holds the item, so slots after it
// are not tidied.
#[test]
fn room_update_hidden_search_stops_at_the_item() {
    let items = [item(1, 0, 0), item(2, 1, 0)];
    let lookup = |n: i32| items.iter().find(|i| i.number == n);
    let mut old = room(1, 10);
    old.hiditems[0] = 1;
    old.hiditems[1] = 2;
    let mut new = room(1, 10);
    new.hiditems[0] = 1;
    assert!(merge_room(&mut old, &new, lookup));
    assert_eq!(old.hiditems[..2], [1, 2]);
}

// ---------------------------------------------------------------------------------------
// Shops.

fn shop(number: i32, type_: i16) -> ShopRecord {
    let mut s: ShopRecord = zero();
    s.number = number;
    s.type_ = type_;
    s.name = text("General Store");
    s
}

// Shop: names and settings copied; per slot a new item brings its
// quantity, other slot members are copied alone, the quantity is otherwise kept.
#[test]
fn shop_update_merges_per_slot_and_keeps_quantities() {
    let mut old = shop(1, 0);
    old.items[0] = 10;
    old.qty[0] = 3;
    old.maxqty[0] = 5;
    old.items[1] = 11;
    old.qty[1] = 4;
    old.dirty = 0;
    old.unk_0x1dc = Raw(vec![1; 24]);
    let mut new = shop(1, 0);
    new.name = text("Old Store");
    new.desc[1] = text("Dusty.");
    new.markup = 20;
    new.items[0] = 10;
    new.qty[0] = 9; // same item: quantity kept
    new.maxqty[0] = 8;
    new.regentime[0] = 30;
    new.regennum[0] = 2;
    new.regenpct[0] = 50;
    new.items[1] = 12; // new item: quantity comes with it
    new.qty[1] = 7;
    assert!(merge_shop(&mut old, &new));
    assert_eq!(old.name, text("Old Store"));
    assert_eq!(old.desc[1], text("Dusty."));
    assert_eq!(old.markup, 20);
    assert_eq!(
        (
            old.qty[0],
            old.maxqty[0],
            old.regentime[0],
            old.regennum[0],
            old.regenpct[0]
        ),
        (3, 8, 30, 2, 50)
    );
    assert_eq!((old.items[1], old.qty[1]), (12, 7));
    assert_eq!(old.unk_0x1dc.0, vec![1; 24], "unk_0x1dc kept");
}

// Shop: a new shop type takes the whole stock list, quantities included.
#[test]
fn shop_update_new_type_takes_the_stock_list() {
    let mut old = shop(1, 0);
    old.items[0] = 10;
    old.qty[0] = 3;
    let mut new = shop(1, 2);
    new.items[0] = 10;
    new.qty[0] = 9;
    assert!(merge_shop(&mut old, &new));
    assert_eq!((old.type_, old.qty[0]), (2, 9));
}

// Shop: gang shops are left alone (by the merge and by the dispatcher).
#[test]
fn shop_update_leaves_gang_shops_alone() {
    let mut old = shop(1, 11);
    old.items[0] = 5;
    let mut new = shop(1, 0);
    new.name = text("Mine now");
    let before = old.clone();
    assert!(!merge_shop(&mut old, &new));
    assert_eq!(old, before);
    let mut old = shop(2, 0);
    let mut new = shop(2, 11);
    new.name = text("Gang");
    assert!(!merge_shop(&mut old, &new), "a gang shop in the update too");

    let mut content = Content::default();
    content.shops.push(decoded("WCCSHOPS.VIR", 1, shop(1, 11)));
    let report = apply_patch_list(&mut content, &[patch(1, PatchKind::Shop, &shop(1, 0))]).unwrap();
    assert_eq!(report.gang_shops_skipped, 1);
    assert_eq!(content.shops[0].record.type_, 11);
}

// ---------------------------------------------------------------------------------------
// Items, spells, monsters.

// Item: the definition is copied; number, active, unk_0x008,
// unk_0x3bf and dirty are kept.
#[test]
fn item_update_copies_the_definition_and_keeps_active() {
    let mut old = item(5, 1, 0);
    old.active = 3;
    old.dirty = 0;
    old.unk_0x008 = Raw(vec![1; 165]);
    old.unk_0x3bf = Raw(vec![2; 24]);
    let mut new = item(5, 1, 2);
    new.name = text("longsword");
    new.desc[5] = text("Sharp.");
    new.hiddesc[2] = text("Magic.");
    new.abilities[19] = 4;
    new.abilvals[19] = 7;
    new.classes[9] = 1;
    new.races[9] = 2;
    new.negates[9] = 3;
    new.cash = [1, 2, 3, 4, 5];
    new.maxdmg = 12;
    new.destroymsg = 9;
    new.nodrop = 1;
    new.active = 99;
    new.dirty = 1;
    assert!(merge_item(&mut old, &new));
    let mut expect = new.clone();
    expect.active = 3;
    expect.dirty = 0;
    expect.unk_0x008 = Raw(vec![1; 165]);
    expect.unk_0x3bf = Raw(vec![2; 24]);
    assert_eq!(old, expect);

    // Only active differs: nothing copied.
    let mut old = item(6, 1, 0);
    let mut new = old.clone();
    new.active = 4;
    assert!(!merge_item(&mut old, &new));
    assert_eq!(old.active, 0);
}

// Spell: any byte differs -> the whole record.
#[test]
fn spell_update_replaces_the_whole_record_on_any_byte() {
    let mut old: SpellRecord = zero();
    old.number = 7;
    let mut new = old.clone();
    assert!(!merge_spell(&mut old, &new));
    new.unk_0x0FC = 1; // a span no other merge would look at
    assert!(merge_spell(&mut old, &new));
    assert_eq!(old, new);
}

// Known monster: the design is copied; number, curalive, killdate,
// killtime, unk_0x004 and dirty are kept.
#[test]
fn monster_update_copies_the_design_and_keeps_the_run_time_members() {
    let mut old: MonsterRecord = zero();
    old.number = 12;
    old.curalive = 3;
    old.killdate = 0x5021;
    old.killtime = 0x6000;
    old.unk_0x004 = Raw(vec![4; 50]);
    let mut new: MonsterRecord = zero();
    new.number = 12;
    new.name = text("orc rogue");
    new.desc[3] = text("Ugly.");
    new.level = 4;
    new.atkmaxs[4] = 9;
    new.spelllvls[4] = 2;
    new.items[9] = 100;
    new.cash = [0, 0, 0, 10, 20];
    new.randtext = 5;
    new.gender = 1;
    new.curalive = 0;
    new.killdate = 0;
    assert!(merge_monster(&mut old, &new));
    let mut expect = new.clone();
    expect.curalive = 3;
    expect.killdate = 0x5021;
    expect.killtime = 0x6000;
    expect.unk_0x004 = Raw(vec![4; 50]);
    assert_eq!(old, expect);

    // Only run-time members differ: nothing copied.
    let mut old2 = old.clone();
    let mut new2 = old.clone();
    new2.curalive = 0;
    assert!(!merge_monster(&mut old2, &new2));
    assert_eq!(old2.curalive, 3);
}

// ---------------------------------------------------------------------------------------
// Messages, actions, text blocks.

// Message: the three texts, together.
#[test]
fn message_update_copies_the_three_texts() {
    let mut old: MudmsgRecord = zero();
    old.number = 3;
    old.unk_0x0FB = Raw(vec![5, 5]);
    let mut new = old.clone();
    new.unk_0x0FB = Raw(vec![0, 0]);
    assert!(!merge_message(&mut old, &new), "unk_0x0FB is not compared");
    new.msgs[2] = text("You see %s.");
    assert!(merge_message(&mut old, &new));
    assert_eq!(old.msgs[2], text("You see %s."));
    assert_eq!(old.unk_0x0FB.0, vec![5, 5], "unk_0x0FB kept");
}

fn action(name: &str, targets: i16) -> ActionRecord {
    let mut a: ActionRecord = zero();
    a.name = text(name);
    a.targets = targets;
    a
}

// Social action: replaced unless AT_KEEP; looked up by verb
// in any case.
#[test]
fn action_update_replaces_unless_kept_and_matches_any_case() {
    let mut old = action("smile", 0);
    let mut new = action("SMILE", 1);
    new.texts[0] = text("You smile.");
    assert!(merge_action(&mut old, &new));
    assert_eq!(old, new);

    let mut kept = action("wave", 0x40);
    let before = kept.clone();
    assert!(!merge_action(&mut kept, &action("wave", 1)));
    assert_eq!(kept, before);

    let mut content = Content::default();
    content
        .actions
        .push(decoded("WCCACTS.VIR", 1, action("smile", 0)));
    let report = apply_patch_list(
        &mut content,
        &[patch(1, PatchKind::Action, &action("Smile", 3))],
    )
    .unwrap();
    assert_eq!(report.applied_by_kind[&PatchKind::Action], 1);
    assert_eq!(content.actions.len(), 1);
    assert_eq!(content.actions[0].record.targets, 3);
}

fn block(number: i32, part: i16, stored: &[u8]) -> TextblkRecord {
    let mut t: TextblkRecord = zero();
    t.number = number;
    t.part = part;
    t.text.0[..stored.len()].copy_from_slice(stored);
    t
}

// Text block: the carried bytes differ -> replaced.
#[test]
fn text_update_replaces_a_block_that_differs() {
    let mut old = block(4, 0, b"hello");
    let new = block(4, 0, b"hello");
    assert!(!merge_text(&mut old, &new, 2022));
    let mut new = block(4, 0, b"hello");
    new.text.0[1999] = 1; // past the text: still a difference when carried
    assert!(merge_text(&mut old, &new, 2022));
    assert_eq!(old, new);
}

// Text block, deliberate deviation [I] (silvermere#25 option (b)): only the bytes
// the updrec carried are the update's; the rest of the stored block is kept, so a long text
// is not shortened by a 2015-byte record.
#[test]
fn text_update_keeps_the_bytes_the_record_did_not_carry() {
    let long = vec![b'a' + 0x20; 1995];
    let mut old = block(4, 0, &long);
    let mut new = block(4, 0, &long[..1988]); // the image: text[1988..] missing (zero)
    assert!(
        !merge_text(&mut old, &new, 2010),
        "equal in the carried bytes"
    );
    assert_eq!(old.text.0[..1995], long[..]);
    new.text.0[10] = b'z';
    assert!(merge_text(&mut old, &new, 2010));
    assert_eq!(old.text.0[10], b'z');
    assert_eq!(old.text.0[1988..1995], long[1988..1995], "the tail is kept");

    // Through the driver, with a record of the stock length.
    let mut content = Content::default();
    content
        .text_blocks
        .push(decoded("WCCTEXT.VIR", 1, block(4, 0, &long)));
    let mut p = patch(1, PatchKind::Text, &block(4, 0, &long[..1988]));
    p.carried = 2010;
    let report = apply_patch_list(&mut content, &[p]).unwrap();
    assert_eq!(report.applied_by_kind[&PatchKind::Text], 1);
    assert!(report.rewritten_by_kind.is_empty());
    assert_eq!(content.text_blocks[0].record.text.0[..1995], long[..]);
}

// An inserted text block keeps the text's length plus 23 bytes: the bytes after the stored
// text's NUL are not kept. Looked up by number and part.
#[test]
fn text_insert_keeps_the_text_up_to_its_nul() {
    let mut content = Content::default();
    content
        .text_blocks
        .push(decoded("WCCTEXT.VIR", 1, block(4, 0, b"abc")));
    let mut new = block(4, 1, b"xyz\0junk");
    new.link = 8;
    let report = apply_patch_list(&mut content, &[patch(9, PatchKind::Text, &new)]).unwrap();
    assert_eq!(report.inserted_by_kind[&PatchKind::Text], 1);
    let added = &content.text_blocks[1];
    assert_eq!((added.file.as_str(), added.ordinal), ("WCCUPDAT.DAT", 9));
    assert_eq!(added.record.link, 8);
    assert_eq!(&added.record.text.0[..8], b"xyz\0\0\0\0\0");
}

// ---------------------------------------------------------------------------------------
// The driver.

// A missing key is inserted, for every kind.
#[test]
fn a_missing_key_is_inserted_for_every_kind() {
    let mut content = Content::default();
    let mut text_block = block(1, 0, b"x");
    text_block.link = 0;
    let patches = vec![
        patch(1, PatchKind::Race, &race(2)),
        patch(2, PatchKind::Class, &{
            let mut c: ClassRecord = zero();
            c.number = 2;
            c
        }),
        patch(3, PatchKind::Room, &room(1, 5)),
        patch(4, PatchKind::Shop, &shop(4, 0)),
        patch(5, PatchKind::Item, &item(5, 1, 0)),
        patch(6, PatchKind::Spell, &{
            let mut s: SpellRecord = zero();
            s.number = 6;
            s
        }),
        patch(7, PatchKind::Monster, &{
            let mut m: MonsterRecord = zero();
            m.number = 7;
            m
        }),
        patch(8, PatchKind::Message, &{
            let mut m: MudmsgRecord = zero();
            m.number = 8;
            m
        }),
        patch(9, PatchKind::Text, &text_block),
        patch(10, PatchKind::Action, &action("bow", 0)),
    ];
    let report = apply_patch_list(&mut content, &patches).unwrap();
    for kind in PatchKind::ALL {
        assert_eq!(report.inserted_by_kind.get(&kind), Some(&1), "{kind:?}");
    }
    assert!(report.applied_by_kind.is_empty());
    assert_eq!(report.total(), 10);
    assert_eq!((report.first_seq, report.last_seq), (Some(1), Some(10)));
    assert_eq!(content.rooms[0].record.roomnum, 5);
    assert_eq!(content.items[0].file, "WCCUPDAT.DAT");
    assert_eq!(content.actions[0].record.name, text("bow"));
}

// GETGT on key 0 from the last seqnum applied: ascending seqnum,
// whatever the physical order; the later patch of a record wins.
#[test]
fn patches_apply_in_seqnum_order() {
    let mut content = Content::default();
    content
        .items
        .push(decoded("WCCITEMS.VIR", 1, item(5, 1, 0)));
    let mut first = item(5, 1, 0);
    first.name = text("first");
    first.maxdmg = 4;
    let mut second = item(5, 1, 0);
    second.name = text("second");
    // In the slice the later seqnum comes first.
    let patches = [
        patch(20, PatchKind::Item, &second),
        patch(10, PatchKind::Item, &first),
    ];
    let report = apply_patch_list(&mut content, &patches).unwrap();
    assert_eq!(content.items[0].record.name, text("second"));
    assert_eq!(content.items[0].record.maxdmg, 0);
    assert_eq!(report.rewritten_by_kind[&PatchKind::Item], 2);
    assert_eq!((report.first_seq, report.last_seq), (Some(10), Some(20)));
    // An insert followed by a patch of the same key: the second merges into the first.
    let mut content = Content::default();
    let patches = [
        patch(2, PatchKind::Item, &second),
        patch(1, PatchKind::Item, &first),
    ];
    let report = apply_patch_list(&mut content, &patches).unwrap();
    assert_eq!(content.items.len(), 1);
    assert_eq!(content.items[0].record.name, text("second"));
    assert_eq!(report.inserted_by_kind[&PatchKind::Item], 1);
    assert_eq!(report.applied_by_kind[&PatchKind::Item], 1);
}

// GETGT from sequence number 0 never reaches a seqnum of 0 or below.
#[test]
fn a_seqnum_of_zero_is_not_reached() {
    let mut content = Content::default();
    let report =
        apply_patch_list(&mut content, &[patch(0, PatchKind::Item, &item(5, 1, 0))]).unwrap();
    assert_eq!(report.not_reached, 1);
    assert!(content.items.is_empty());
}

// Card T-009: a type byte outside 1..10 is an error naming the seqnum (the original skips
// it; the stock file has none).
#[test]
fn a_type_outside_1_to_10_is_an_error_naming_the_seqnum() {
    let mut raw = vec![0u8; 2015];
    raw[..4].copy_from_slice(&4321i32.to_le_bytes());
    raw[4] = 11;
    let err = Patch::parse(&raw).unwrap_err();
    assert!(
        matches!(&err, ImportError::Patch(m) if m.contains("seqnum 4321")),
        "{err}"
    );
    raw[4] = 0;
    assert!(Patch::parse(&raw).is_err());
    raw[4] = 10;
    assert_eq!(Patch::parse(&raw).unwrap().kind, PatchKind::Action);
}
