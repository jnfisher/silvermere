//! T-030: items and inventory. Get/drop/give/stash, wear/remove/ready, `user_can_use`,
//! encumbrance, light and burn, keys, limited items, the inventory display, against the
//! original's behaviour and the `status-screens` capture.
//!
//! The item records are the stock ones (`WCCITEMS` rows of the T-006 world export, the
//! fields these tests read); the rest are made up for the refusals.

use crate::common;

use common::enter;
use proptest::prelude::*;
use silvermere_engine::commands::execute_input;
use silvermere_engine::commands::inventory::encumbrance_of;
use silvermere_engine::items::can_use::{CantUse, EXEMPT_ITEMS, UseCheck, can_use};
use silvermere_engine::items::inventory::{
    Search, add_item_to_inventory, find_item_in_inventory, matches_any_word,
};
use silvermere_engine::items::limited;
use silvermere_engine::items::room_items::add_item_to_room;
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{EngineOutput, PersistRequest, SessionId, World};
use silvermere_rules::economy::EncumbranceLevel;
use silvermere_types::enums::{
    Ability, ArmourType, Currency, ItemType, MessageLevel, WeaponClass, WeaponType, WornLocation,
};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{ClassNo, ItemNo, MessageNo, RaceNo, RoomKey};
use silvermere_types::records::{Character, Class, Item, Message};
use silvermere_types::sizes::{NINVITEMS, NROOMITEMS};

// Stock item numbers (WCCITEMS).
const QUARTERSTAFF: i32 = 100;
const TORCH: i32 = 175;
const LANTERN: i32 = 176;
const VEST: i32 = 332;
const PANTS: i32 = 333;
const HELM: i32 = 334;
const GLOVES: i32 = 335;
const BOOTS: i32 = 336;
// Made up.
const BUCKLER: i32 = 900;
const CURSED_RING: i32 = 901;
const RING: i32 = 902;
const IRON_KEY: i32 = 903;
const SOULSTONE: i32 = 904;
const RELIC: i32 = 905;
const WAND: i32 = 906;
const GREATSWORD: i32 = 907;
const DAGGER: i32 = 908;
const SHORT_SWORD: i32 = 909;
const SHORT_BOW: i32 = 910;

fn item(n: i32, name: &str, ty: ItemType, weight: i16) -> Item {
    let mut i = blank::clear_item(ItemNo(n));
    i.name = name.to_string();
    i.item_type = ty;
    i.weight = weight;
    i.gettable = 1;
    i.charges = -1;
    i
}

fn armour(n: i32, name: &str, weight: i16, worn: WornLocation) -> Item {
    let mut i = item(n, name, ItemType::Armour, weight);
    i.armtype = ArmourType::Robes;
    i.worn = worn;
    i
}

fn weapon(n: i32, name: &str, weight: i16, wpn: WeaponType, strreq: i16, speed: i16) -> Item {
    let mut i = item(n, name, ItemType::Weapon, weight);
    i.wpntype = wpn;
    i.strreq = strreq;
    i.speed = speed;
    i
}

fn items() -> Vec<Item> {
    // WCCITEMS 100: quarterstaff, weight 100, 2-handed blunt, strreq 30, speed 1200
    let qs = weapon(
        QUARTERSTAFF,
        "quarterstaff",
        100,
        WeaponType::TwoHandBlunt,
        30,
        1200,
    );
    // WCCITEMS 175: torch, weight 40, light, 800 charges, Alter General Light 100, destroy
    // message 8603
    let mut torch = item(TORCH, "torch", ItemType::Light, 40);
    torch.charges = 800;
    torch.abilities[0] = (Ability::GenLight, 100);
    torch.destroymsg = MessageNo(8603);
    // WCCITEMS 176: lantern, weight 75, 2400 charges, light 175
    let mut lantern = item(LANTERN, "lantern", ItemType::Light, 75);
    lantern.charges = 2400;
    lantern.abilities[0] = (Ability::GenLight, 175);
    lantern.destroymsg = MessageNo(8604);
    let mut cursed = armour(CURSED_RING, "cursed ring", 1, WornLocation::Finger);
    cursed.abilities[0] = (Ability::Cursed, 1);
    let mut key = item(IRON_KEY, "iron key", ItemType::Key, 1);
    key.charges = 3;
    let mut soulstone = item(SOULSTONE, "soulstone", ItemType::General, 1);
    soulstone.nodrop = 1;
    let mut relic = item(RELIC, "relic", ItemType::General, 1);
    relic.limit = 1;
    let mut wand = item(WAND, "wand", ItemType::General, 1);
    wand.abilities[0] = (Ability::Magical, 1);
    let mut bow = weapon(
        SHORT_BOW,
        "short bow",
        10,
        WeaponType::OneHandSharp,
        0,
        1000,
    );
    bow.abilities[0] = (Ability::Magical, 1);
    vec![
        qs,
        torch,
        lantern,
        // WCCITEMS 332-336: the padded set, armour type 1, Torso/Legs/Head/Hands/Feet
        armour(VEST, "padded vest", 240, WornLocation::Torso),
        armour(PANTS, "padded pants", 40, WornLocation::Legs),
        armour(HELM, "padded helm", 40, WornLocation::Head),
        armour(GLOVES, "padded gloves", 40, WornLocation::Hands),
        armour(BOOTS, "padded boots", 40, WornLocation::Feet),
        armour(BUCKLER, "buckler", 50, WornLocation::OffHand),
        cursed,
        armour(RING, "silver ring", 1, WornLocation::Finger),
        key,
        soulstone,
        relic,
        wand,
        weapon(
            GREATSWORD,
            "greatsword",
            150,
            WeaponType::TwoHandSharp,
            200,
            2000,
        ),
        weapon(DAGGER, "dagger", 10, WeaponType::OneHandSharp, 0, 300),
        weapon(
            SHORT_SWORD,
            "short sword",
            30,
            WeaponType::OneHandSharp,
            0,
            1100,
        ),
        bow,
    ]
}

fn class(n: u16, weapon: WeaponClass) -> Class {
    let mut c = blank::clear_class(ClassNo(n));
    c.weapon = weapon;
    c.armour = ArmourType::Platemail;
    c.combat = 3;
    c
}

/// The tiny world with the items, the torch and lantern messages, and classes 1 (All
/// weapons), 2 (Limited), 3 (1-handed sharp, Anti Magic: a Witchunter-like class).
fn world() -> World {
    let mut w = common::tiny_world(8);
    for i in items() {
        w.items.insert(i.number, i);
    }
    // WCCMSG 8603, 8604
    for (n, text) in [
        (8603, "Your torch flickers and goes out."),
        (8604, "Your lamp runs out of oil, and goes out."),
    ] {
        w.messages.insert(
            MessageNo(n),
            Message {
                number: MessageNo(n),
                msgs: [text.to_string(), String::new(), String::new()],
            },
        );
    }
    w.classes.insert(ClassNo(1), class(1, WeaponClass::All));
    w.classes.insert(ClassNo(2), class(2, WeaponClass::Limited));
    let mut wh = class(3, WeaponClass::OneHandSharp);
    wh.abilities[0] = (Ability::AntiMagic, 1);
    w.classes.insert(ClassNo(3), wh);
    w
}

fn room_key() -> RoomKey {
    RoomKey::new(1, common::special_rooms()[0])
}

/// A level 1 character in the start room, strength 50 (`maxwgt` 2400), 35 hit points.
fn player(w: &mut World, name: &str) -> SessionId {
    let id = enter(w, name, common::special_rooms()[0], MessageLevel::High);
    edit(w, id, |c| {
        c.level = 1;
        c.str = 50;
        c.agi = 50;
        c.maxwgt = 2400;
        c.hp = 35;
        c.maxhp = 35;
        c.readied = -1;
        c.nkeyslots = 50;
        c.nspellslots = 1;
        c.itemuses.iter_mut().for_each(|u| *u = -1);
        c.maxenergy = 1000;
    });
    id
}

fn edit(w: &mut World, id: SessionId, f: impl FnOnce(&mut Character)) {
    f(w.session_mut(id).unwrap().character.as_mut().unwrap());
}

fn plr(w: &World, id: SessionId) -> &Character {
    w.session(id).unwrap().player().unwrap()
}

/// What the session would receive now (prompts empty), the batch emptied.
fn take(w: &mut World, id: SessionId) -> Vec<u8> {
    let palette = palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let out = s.output.render(palette, b"");
    s.output.clear();
    out
}

fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(bytes);
    String::from_utf8_lossy(&plain)
        .split("\r\n")
        .map(|l| l.trim_end().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Types `line` as the player and returns the screen lines it produced.
fn cmd(w: &mut World, id: SessionId, line: &str) -> Vec<String> {
    execute_input(w, id, line, false);
    lines(&take(w, id))
}

fn give(w: &mut World, id: SessionId, n: i32) {
    assert!(add_item_to_inventory(w, id, ItemNo(n), -2), "item {n} fits");
    take(w, id);
}

fn count(c: &Character, n: i32) -> usize {
    c.items.iter().filter(|&&i| i == ItemNo(n)).count()
}

fn floor(w: &World) -> Vec<(i32, i16, i16)> {
    w.room_state[&room_key()]
        .items
        .iter()
        .filter(|s| s.item != ItemNo(0))
        .map(|s| (s.item.0, s.uses, s.count))
        .collect()
}

fn pattern(name: &str) -> regex::Regex {
    let reg: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    let rx = reg["static"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no pattern {name}"))["rx"]
        .as_str()
        .unwrap()
        .to_owned();
    regex::Regex::new(&format!("^(?:{rx})$")).unwrap()
}

fn assert_matches(ls: &[String], name: &str) {
    let rx = pattern(name);
    assert!(ls.iter().any(|l| rx.is_match(l)), "{name}: {ls:?}");
}

// ---------------------------------------------------------------------------------------
// Name matching.

// matches_any_word: an abbreviation of any word of the name
#[test]
fn matches_any_word_checks_each_word() {
    assert!(matches_any_word("dag", "silver dagger"));
    assert!(matches_any_word("sil", "silver dagger"));
    assert!(matches_any_word("silver", "silver dagger"));
    assert!(matches_any_word("SILVER DAG", "silver dagger"));
    assert!(!matches_any_word("sword", "silver dagger"));
    assert!(!matches_any_word("daggers", "silver dagger"));
}

// find_item_in_inventory: words are dropped from the end until something
// fits; an exact name is never ambiguous; copies of the first candidate count once
#[test]
fn find_drops_words_and_counts_candidates() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, SHORT_SWORD);
    give(&mut w, id, SHORT_BOW);
    give(&mut w, id, SHORT_BOW);
    let c = plr(&w, id).clone();
    let f = find_item_in_inventory(&w, &c, "short sword to bob", Search::Any);
    assert_eq!(f.item, Some(ItemNo(SHORT_SWORD)));
    assert_eq!(f.words, 2);
    // only further copies of the *first* candidate are skipped: the
    // second bow is offered again
    let f = find_item_in_inventory(&w, &c, "short", Search::Any);
    assert_eq!(f.count, 3);
    assert!(f.ambiguous());
    assert_eq!(f.options, ["short sword", "short bow", "short bow"]);
    let f = find_item_in_inventory(&w, &c, "bow", Search::Any);
    assert_eq!(
        (f.item, f.count),
        (Some(ItemNo(SHORT_BOW)), 1),
        "copies of the first once"
    );
    let f = find_item_in_inventory(&w, &c, "short bow", Search::Any);
    assert_eq!((f.item, f.count), (Some(ItemNo(SHORT_BOW)), 0));
}

// the list, DROP stops on it
#[test]
fn ambiguous_drop_lists_the_candidates() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, SHORT_SWORD);
    give(&mut w, id, SHORT_BOW);
    let ls = cmd(&mut w, id, "drop sh");
    assert_matches(&ls, "ambiguous");
    assert_eq!(&ls[1..], ["-- short sword", "-- short bow"]);
    assert_matches(&ls[1..], "ambiguous_item");
    assert_eq!(count(plr(&w, id), SHORT_SWORD), 1);
}

// ---------------------------------------------------------------------------------------
// GET and DROP.

#[test]
fn get_and_drop_alone_show_the_syntax() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    assert_eq!(cmd(&mut w, id, "get"), ["Syntax: GET {Item Name}"]);
    assert_eq!(cmd(&mut w, id, "drop"), ["Syntax: DROP {Item Name}"]);
}

// "You took", "picks up" at TRN_HIGH; "You dropped",
// "dropped"; the weight
#[test]
fn drop_and_get_an_item_with_the_room_lines() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    give(&mut w, id, QUARTERSTAFF);
    assert_eq!(plr(&w, id).wgt, 100);
    let ls = cmd(&mut w, id, "drop quarterstaff");
    assert_eq!(ls, ["You dropped quarterstaff."]);
    assert_matches(&ls, "dropped");
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric dropped quarterstaff."]);
    assert_eq!(plr(&w, id).wgt, 0);
    assert_eq!(floor(&w), [(QUARTERSTAFF, -1, 0)]);
    assert_eq!(cmd(&mut w, id, "get quart"), ["You took quarterstaff."]);
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric picks up quarterstaff."]);
    assert_eq!(count(plr(&w, id), QUARTERSTAFF), 1);
    assert!(floor(&w).is_empty());
    assert_eq!(
        cmd(&mut w, id, "get quarterstaff"),
        ["You don't see quarterstaff here."]
    );
}

// an uncharged copy stacks on its slot; charged copies take their own
#[test]
fn floor_slots_stack_uncharged_copies() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, QUARTERSTAFF);
    give(&mut w, id, QUARTERSTAFF);
    give(&mut w, id, TORCH);
    give(&mut w, id, TORCH);
    for _ in 0..2 {
        cmd(&mut w, id, "drop quarterstaff");
        cmd(&mut w, id, "drop torch");
    }
    assert_eq!(
        floor(&w),
        [(QUARTERSTAFF, -1, 1), (TORCH, 800, 0), (TORCH, 800, 0)]
    );
}

// the floor holds NROOMITEMS (17) slots; the item comes back
#[test]
fn a_full_floor_refuses_the_drop() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    for _ in 0..NROOMITEMS {
        assert!(add_item_to_room(&mut w, room_key(), ItemNo(TORCH), false, 5).is_some());
    }
    give(&mut w, id, TORCH);
    assert_eq!(
        cmd(&mut w, id, "drop torch"),
        ["There is no room to drop torch here."]
    );
    assert_eq!(count(plr(&w, id), TORCH), 1);
    assert_eq!(plr(&w, id).itemuses[0], 800, "with its charges");
}

// no-drop; a worn cursed item with no second copy
#[test]
fn nodrop_and_worn_cursed_items_stay() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, SOULSTONE);
    assert_eq!(
        cmd(&mut w, id, "drop soulstone"),
        ["You may not drop that item!"]
    );
    give(&mut w, id, CURSED_RING);
    cmd(&mut w, id, "wear cursed ring");
    assert_eq!(
        cmd(&mut w, id, "drop cursed"),
        ["You may not drop that item!"]
    );
    give(&mut w, id, CURSED_RING);
    assert_eq!(cmd(&mut w, id, "drop cursed"), ["You dropped cursed ring."]);
}

// "You cannot carry that much!" and the item stays on the floor
#[test]
fn too_heavy_to_get() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    edit(&mut w, id, |c| c.maxwgt = 120);
    add_item_to_room(&mut w, room_key(), ItemNo(VEST), false, -1);
    assert_eq!(cmd(&mut w, id, "get vest"), ["You cannot carry that much!"]);
    assert_eq!(floor(&w), [(VEST, -1, 0)]);
}

// NINVITEMS (100) slots
#[test]
fn a_full_pack_cannot_carry_more() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    edit(&mut w, id, |c| c.maxwgt = 30000);
    let pebble = item(960, "pebble", ItemType::General, 1);
    w.items.insert(pebble.number, pebble);
    for _ in 0..NINVITEMS {
        give(&mut w, id, 960);
    }
    assert!(!add_item_to_inventory(&mut w, id, ItemNo(DAGGER), -2));
    assert_eq!(lines(&take(&mut w, id)), ["You cannot carry that much!"]);
}

// the coin lines, the refusals
#[test]
fn get_and_drop_coins() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    w.room_state.get_mut(&room_key()).unwrap().coins.copper = 9;
    let ls = cmd(&mut w, id, "get 9 copper");
    assert_eq!(ls, ["You picked up 9 copper farthings"]);
    assert_matches(&ls, "picked_up_coins");
    assert_eq!(
        lines(&take(&mut w, bob)),
        ["Aldric picks up some copper farthings."]
    );
    assert_eq!(plr(&w, id).coins.copper, 9);
    assert_eq!(
        cmd(&mut w, id, "get 1 copper"),
        ["You don't see 1 copper farthing"]
    );
    assert_eq!(
        cmd(&mut w, id, "get copper"),
        ["You don't see any copper farthings"]
    );
    assert_eq!(
        cmd(&mut w, id, "get 5 bananas"),
        ["Syntax: GET 5 {Currency}"]
    );
    assert_eq!(
        cmd(&mut w, id, "get 99999 gold"),
        ["Syntax: GET {Amount} {Currency}"]
    );
    assert_eq!(
        cmd(&mut w, id, "get bananas"),
        ["You don't see bananas here."]
    );
    assert_eq!(
        cmd(&mut w, id, "drop 4 copper"),
        ["You dropped 4 copper farthings."]
    );
    assert_eq!(
        lines(&take(&mut w, bob)),
        ["Aldric dropped some copper farthings."]
    );
    assert_eq!(
        cmd(&mut w, id, "get copper"),
        ["You picked up 4 copper farthings"]
    );
    assert_eq!(
        cmd(&mut w, id, "drop 1 gold"),
        ["You don't have 1 gold crown to drop!"]
    );
    assert_eq!(
        cmd(&mut w, id, "drop 1 bananas"),
        ["Syntax: DROP 1 {Currency}"]
    );
    assert_eq!(
        cmd(&mut w, id, "drop 0 gold"),
        ["Syntax: DROP {Amount} {Currency}"]
    );
    assert_eq!(
        cmd(&mut w, id, "drop sword"),
        ["You don't have sword to drop!"]
    );
}

// three coins per unit of capacity left; the excess goes back
#[test]
fn coins_up_to_what_can_be_carried() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    // strength 0: maxwgt 0, the carry limit is 1
    edit(&mut w, id, |c| {
        c.str = 0;
        c.maxwgt = 0;
    });
    w.room_state.get_mut(&room_key()).unwrap().coins.gold = 100;
    assert_eq!(cmd(&mut w, id, "get gold"), ["You picked up 3 gold crowns"]);
    assert_eq!(w.room_state[&room_key()].coins.gold, 97);
    // the quirk: with no room left the amount becomes 0 and 0 coins are "taken"
    assert_eq!(
        cmd(&mut w, id, "get 5 gold"),
        ["You picked up 0 gold crowns"]
    );
    assert_eq!(
        cmd(&mut w, id, "get gold"),
        ["You don't see any gold crowns"]
    );
}

// ---------------------------------------------------------------------------------------
// STASH and the hidden list.

// (hidden items need the found bit)
#[test]
fn stash_hides_an_item_that_get_finds_after_a_search() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    w.rooms.get_mut(&room_key()).unwrap().light = 100;
    assert_eq!(cmd(&mut w, id, "stash"), ["Syntax: STASH {Item Name}"]);
    give(&mut w, id, DAGGER);
    assert_eq!(cmd(&mut w, id, "stash dagger"), ["You hid dagger."]);
    assert_eq!(w.room_state[&room_key()].hiditems[0].item, ItemNo(DAGGER));
    assert_eq!(
        cmd(&mut w, id, "get dagger"),
        ["You don't see dagger here."]
    );
    w.room_state.get_mut(&room_key()).unwrap().foundmask = 1;
    assert_eq!(cmd(&mut w, id, "get dagger"), ["You took dagger."]);
    assert_eq!(
        w.room_state[&room_key()].foundmask,
        0,
        "the slot is empty again"
    );
    edit(&mut w, id, |c| c.coins.gold = 5);
    assert_eq!(cmd(&mut w, id, "stash 3 gold"), ["You hid 3 gold crowns."]);
    assert_eq!(w.room_state[&room_key()].hidcoins.gold, 3);
    assert_eq!(
        cmd(&mut w, id, "get 3 gold"),
        ["You picked up 3 gold crowns"]
    );
    // HIDE with a name is the same item and coin code
    give(&mut w, id, DAGGER);
    assert_eq!(cmd(&mut w, id, "hide dagger"), ["You hid dagger."]);
    assert_eq!(
        cmd(&mut w, id, "hide 9 gold"),
        ["You don't have 9 gold crowns to hide!"]
    );
    assert_eq!(
        cmd(&mut w, id, "hide 2 bananas"),
        ["Syntax: HIDE 2 {Currency}"]
    );
    give(&mut w, id, SOULSTONE);
    assert_eq!(
        cmd(&mut w, id, "hide soulstone"),
        ["You may not hide that item!"]
    );
}

// ---------------------------------------------------------------------------------------
// GIVE.

// the three lines; receive off; the receiver's prompt
#[test]
fn give_an_item_with_receive_on_and_off() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    let carl = player(&mut w, "Carl");
    give(&mut w, id, DAGGER);
    assert_eq!(
        cmd(&mut w, id, "give dagger to bob"),
        ["You just gave dagger to Bob."]
    );
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric just gave you dagger."]);
    assert_eq!(
        lines(&take(&mut w, carl)),
        ["Aldric just gave Bob something."]
    );
    assert_eq!(count(plr(&w, bob), DAGGER), 1);
    assert_eq!(plr(&w, bob).wgt, 10);
    edit(&mut w, id, |c| c.settings |= PlayerSettings::NORECEIVE);
    assert_eq!(
        cmd(&mut w, bob, "give dagger to aldric"),
        ["Aldric refuses your offer."]
    );
    assert_eq!(count(plr(&w, bob), DAGGER), 1);
    // nobody of that name: not a GIVE command (`Handled::NotCommand`), the line goes on to speech
    let ls = cmd(&mut w, bob, "give dagger to nobody");
    assert!(!ls.iter().any(|l| l.contains("gave")), "{ls:?}");
    assert_eq!(count(plr(&w, bob), DAGGER), 1);
}

// coins, receive off, not that much
#[test]
fn give_coins() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    let carl = player(&mut w, "Carl");
    edit(&mut w, id, |c| c.coins.silver = 20);
    assert_eq!(
        cmd(&mut w, id, "give 5 silver to bob"),
        ["You gave Bob 5 silver"]
    );
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric gave you 5 silver"]);
    assert_eq!(
        lines(&take(&mut w, carl)),
        ["Aldric just gave Bob some coins."]
    );
    assert_eq!(
        (plr(&w, id).coins.silver, plr(&w, bob).coins.silver),
        (15, 5)
    );
    assert_eq!(
        cmd(&mut w, id, "give 50 silver to bob"),
        ["You do not have that much silver!"]
    );
    assert_eq!(
        cmd(&mut w, id, "give 5 bananas to bob"),
        ["Syntax: GIVE {amount} {currency} TO {someone}"]
    );
    edit(&mut w, bob, |c| c.settings |= PlayerSettings::NORECEIVE);
    assert_eq!(
        cmd(&mut w, id, "give 5 silver to bob"),
        ["Bob refuses your offer."]
    );
    assert_eq!(
        cmd(&mut w, id, "give 5 silver to fred"),
        ["Why would you want to give to that?"]
    );
}

// the receiver cannot carry it: he sees the refusal, the item returns
#[test]
fn give_to_someone_who_cannot_carry_it() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    edit(&mut w, bob, |c| c.maxwgt = 100);
    give(&mut w, id, VEST);
    assert_eq!(
        cmd(&mut w, id, "give vest to bob"),
        ["Bob cannot accept your offer."]
    );
    assert_eq!(lines(&take(&mut w, bob)), ["You cannot carry that much!"]);
    assert_eq!(count(plr(&w, id), VEST), 1);
}

// ---------------------------------------------------------------------------------------
// WEAR, REMOVE, READY.

// each word's syntax line alone
#[test]
fn equip_words_alone_show_their_syntax() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    assert_eq!(cmd(&mut w, id, "arm"), ["Syntax: ARM {weapon name}"]);
    assert_eq!(cmd(&mut w, id, "wield"), ["Syntax: WIELD {weapon name}"]);
    assert_eq!(cmd(&mut w, id, "wear"), ["Syntax: WEAR {item name}"]);
    assert_eq!(cmd(&mut w, id, "ready"), ["Syntax: READY {weapon name}"]);
    assert_eq!(cmd(&mut w, id, "remove"), ["Syntax: REMOVE {item name}"]);
}

// cmd_equip's find (a `clear_output`) takes back the dispatcher's ANSI_NORMAL: the reply starts at
// the clear-line, as in the `shop` capture (steps 12, 22-26)
#[test]
fn wear_and_wield_start_without_the_dispatchers_reset() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, HELM);
    execute_input(&mut w, id, "wear helm", false);
    let bytes = take(&mut w, id);
    assert_eq!(bytes, b"\x1b[79D\x1b[KYou are now wearing padded helm.\r\n");
    give(&mut w, id, QUARTERSTAFF);
    execute_input(&mut w, id, "wield quarterstaff", false);
    let bytes = take(&mut w, id);
    assert_eq!(
        bytes,
        b"\x1b[79D\x1b[KYou are now holding quarterstaff.\r\n"
    );
}

// the lines; the swap of an item on the same place
#[test]
fn wear_and_swap() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    give(&mut w, id, HELM);
    let ls = cmd(&mut w, id, "wear helm");
    assert_eq!(ls, ["You are now wearing padded helm."]);
    assert_matches(&ls, "wear_ok");
    let room = lines(&take(&mut w, bob));
    assert_eq!(room, ["Aldric wears padded helm!"]);
    assert_matches(&room, "other_wears");
    assert_eq!(plr(&w, id).worn[0], ItemNo(HELM));
    // a second helm: the first comes off
    give(&mut w, id, HELM);
    assert_eq!(
        cmd(&mut w, id, "wear helm"),
        ["You do not have helm left unequipped."],
        "every copy of a worn item counts as worn (SRCH_UNWORN)"
    );
    // wear_armour's own lookup (no item from EQUIP) says "any"
    silvermere_engine::items::equip::wear_armour(&mut w, id, "cape", None);
    let ls = lines(&take(&mut w, id));
    assert_eq!(ls, ["You do not have any cape."]);
    assert_matches(&ls, "not_have_any");
    // the swap: the same place is taken off first
    let mut hood = armour(951, "hood", 5, WornLocation::Head);
    hood.charges = -1;
    w.items.insert(hood.number, hood);
    give(&mut w, id, 951);
    take(&mut w, bob);
    assert_eq!(
        cmd(&mut w, id, "wear hood"),
        ["You have removed padded helm.", "You are now wearing hood."]
    );
    assert_eq!(
        lines(&take(&mut w, bob)),
        ["Aldric removes padded helm!", "Aldric wears hood!"]
    );
}

// a ring: the pair holds two; the same item comes off
#[test]
fn rings_share_two_fingers() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, RING);
    give(&mut w, id, CURSED_RING);
    assert_eq!(
        cmd(&mut w, id, "wear silver ring"),
        ["You are now wearing silver ring."]
    );
    assert_eq!(
        cmd(&mut w, id, "wear cursed ring"),
        ["You are now wearing cursed ring."]
    );
    let c = plr(&w, id);
    assert_eq!(&c.worn[..2], [ItemNo(RING), ItemNo(CURSED_RING)]);
    assert_eq!(
        cmd(&mut w, id, "remove cursed ring"),
        ["You cannot remove cursed ring!"]
    );
    assert_matches(&["You cannot remove cursed ring!".into()], "cant_remove");
}

// no free worn slot; not armour; not carried; refused
#[test]
fn wear_refusals() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, DAGGER);
    let ls = cmd(&mut w, id, "wear dagger");
    assert_eq!(
        ls[0], "You are now holding dagger.",
        "WEAR of a weapon readies it"
    );
    give(&mut w, id, TORCH);
    let ls = cmd(&mut w, id, "wear torch");
    assert_eq!(ls, ["torch may not be worn!"]);
    assert_matches(&ls, "wear_fail");
    let ls = cmd(&mut w, id, "wear cape");
    assert_eq!(ls, ["You do not have cape left unequipped."]);
    assert_matches(&ls, "not_have_any");
    edit(&mut w, id, |c| c.worn = [ItemNo(RING); 20]);
    give(&mut w, id, VEST);
    let ls = cmd(&mut w, id, "wear vest");
    assert_eq!(ls, ["You have no more room to wear that item!"]);
    assert_matches(&ls, "wear_fail");
    edit(&mut w, id, |c| c.worn = [ItemNo(0); 20]);
    w.items.get_mut(&ItemNo(VEST)).unwrap().armtype = ArmourType::Platemail;
    w.classes.get_mut(&ClassNo(1)).unwrap().armour = ArmourType::Robes;
    let ls = cmd(&mut w, id, "wear vest");
    assert_eq!(ls, ["You may not wear that item!"]);
    assert_matches(&ls, "wear_fail");
}

// no shield with a 2-handed weapon; the reverse
#[test]
fn off_hand_and_two_handed_weapons() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, QUARTERSTAFF);
    give(&mut w, id, BUCKLER);
    cmd(&mut w, id, "wield quarterstaff");
    let ls = cmd(&mut w, id, "wear buckler");
    assert_eq!(
        ls,
        ["You may not wear an off-hand item while you have a 2-handed weapon readied."]
    );
    assert_matches(&ls, "offhand_2h");
    cmd(&mut w, id, "unready");
    assert_eq!(
        cmd(&mut w, id, "wear buckler"),
        ["You are now wearing buckler."]
    );
    let ls = cmd(&mut w, id, "ready quarterstaff");
    assert_eq!(
        ls,
        ["You may not ready a 2-handed weapon with your buckler worn!"]
    );
    assert_matches(&ls, "offhand_2h");
}

// remove; not worn
#[test]
fn remove_worn_items() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    give(&mut w, id, VEST);
    cmd(&mut w, id, "wear vest");
    take(&mut w, bob);
    let ls = cmd(&mut w, id, "remove vest");
    assert_eq!(ls, ["You have removed padded vest."]);
    assert_matches(&ls, "removed");
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric removes padded vest!"]);
    let ls = cmd(&mut w, id, "remove vest");
    assert_eq!(ls, ["You are not wearing vest."]);
    assert_matches(&ls, "not_wearing");
}

// wield, the heavy and quick notes; unready
#[test]
fn ready_and_unready() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    give(&mut w, id, QUARTERSTAFF);
    let ls = cmd(&mut w, id, "wield quarterstaff");
    assert_eq!(ls, ["You are now holding quarterstaff."]);
    assert_matches(&ls, "wield_ok");
    let room = lines(&take(&mut w, bob));
    assert_eq!(room, ["Aldric wields quarterstaff!"]);
    assert_matches(&room, "other_wears");
    assert_eq!(plr(&w, id).weapon, ItemNo(QUARTERSTAFF));
    let ls = cmd(&mut w, id, "unready");
    assert_eq!(ls, ["You now have no weapon readied."]);
    assert_matches(&ls, "unwielded");
    assert_eq!(lines(&take(&mut w, bob)), ["Aldric removes quarterstaff."]);
    // too weak: strreq 200 against strength 50
    give(&mut w, id, GREATSWORD);
    let ls = cmd(&mut w, id, "ready greatsword");
    assert_eq!(
        ls,
        [
            "You are now holding greatsword.",
            "This weapon feels heavy in your hands."
        ]
    );
    assert_matches(&ls, "weapon_heavy");
    // a fast weapon: speed 300 makes a blow under 200 energy
    give(&mut w, id, DAGGER);
    take(&mut w, bob);
    let ls = cmd(&mut w, id, "ready dagger");
    assert_eq!(
        ls,
        [
            "You are now holding dagger.",
            "You are extremely quick and deadly with this weapon."
        ]
    );
    assert_matches(&ls, "weapon_quick");
    assert_eq!(
        lines(&take(&mut w, bob)),
        ["Aldric removes greatsword.", "Aldric wields dagger!"]
    );
    // READY falls into EQUIP, which wears what is no weapon; the
    // "not a weapon" line is ready_weapon's own
    give(&mut w, id, VEST);
    edit(&mut w, id, |c| c.weapon = ItemNo(0));
    assert_eq!(
        cmd(&mut w, id, "ready vest"),
        ["You are now wearing padded vest."]
    );
    give(&mut w, id, BOOTS);
    silvermere_engine::items::equip::ready_weapon(&mut w, id, Some(("boots", None)));
    let ls = lines(&take(&mut w, id));
    assert_eq!(ls, ["padded boots is not a weapon!"]);
    assert_matches(&ls, "weapon_forbidden");
    silvermere_engine::items::equip::ready_weapon(&mut w, id, Some(("axe", None)));
    let ls = lines(&take(&mut w, id));
    assert_eq!(ls, ["You do not have a axe."]);
    assert_matches(&ls, "not_have_any");
    edit(&mut w, id, |c| c.class = ClassNo(2));
    let ls = cmd(&mut w, id, "ready dagger");
    assert_eq!(ls, ["You may not use that weapon."]);
    assert_matches(&ls, "weapon_forbidden");
    // EXEMPT_ITEMS: the quarterstaff is one of the Limited class's three
    assert_eq!(
        cmd(&mut w, id, "ready quarterstaff"),
        ["You are now holding quarterstaff."]
    );
}

// EQUIP alone lists the inventory; a named item already on is
// "You do not have %s left unequipped."
#[test]
fn equip_reports_an_item_already_on() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, VEST);
    assert_eq!(
        cmd(&mut w, id, "equip vest"),
        ["You are now wearing padded vest."]
    );
    let ls = cmd(&mut w, id, "equip vest");
    assert_eq!(ls, ["You do not have vest left unequipped."]);
    assert_matches(&ls, "not_have_any");
    let ls = cmd(&mut w, id, "equip");
    assert_eq!(ls[0], "You are carrying padded vest (Torso)");
}

// Alter HP while worn; dropping the last copy takes
// it off
#[test]
fn worn_abilities_come_and_go() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    w.items.get_mut(&ItemNo(VEST)).unwrap().abilities[1] = (Ability::AlterHp, 10);
    give(&mut w, id, VEST);
    cmd(&mut w, id, "wear vest");
    // the hit points directly; the maximum again through the secondary-stat recalculation
    let worn = plr(&w, id).maxhp;
    assert_eq!(plr(&w, id).hp, 45);
    cmd(&mut w, id, "drop vest");
    assert_eq!(plr(&w, id).hp, 35);
    assert_eq!(worn - plr(&w, id).maxhp, 10);
    assert_eq!(plr(&w, id).worn[0], ItemNo(0));
}

// ---------------------------------------------------------------------------------------
// user_can_use.

fn check<'a>(class: &'a Class, anti_magic: bool) -> UseCheck<'a> {
    UseCheck {
        class: Some(class),
        anti_magic,
        spells: &|_| None,
    }
}

// user_can_use: class list, race list, level abilities, Anti-Magic
// against Magical, the Limited weapon class, armour weight, alignment
#[test]
fn can_use_matrix() {
    let mut c = silvermere_engine::clear_player();
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.level = 5;
    let all = class(1, WeaponClass::All);
    let limited = class(2, WeaponClass::Limited);
    let sharp = class(3, WeaponClass::OneHandSharp);
    let mut sword = weapon(
        SHORT_SWORD,
        "short sword",
        30,
        WeaponType::OneHandSharp,
        0,
        1100,
    );
    assert_eq!(can_use(&c, &sword, &check(&all, false)), Ok(()));
    // the class list
    sword.classes[0] = ClassNo(4);
    assert_eq!(
        can_use(&c, &sword, &check(&all, false)),
        Err(CantUse::Class)
    );
    sword.classes[1] = ClassNo(1);
    assert_eq!(
        can_use(&c, &sword, &check(&limited, false)),
        Ok(()),
        "named: any type"
    );
    sword.classes = [ClassNo(0); 10];
    // the race list
    sword.races[0] = RaceNo(2);
    assert_eq!(can_use(&c, &sword, &check(&all, false)), Err(CantUse::Race));
    sword.races[0] = RaceNo(0);
    // the level abilities
    sword.abilities[0] = (Ability::MinLevel, 6);
    assert_eq!(
        can_use(&c, &sword, &check(&all, false)),
        Err(CantUse::MinLevel)
    );
    sword.abilities[0] = (Ability::MaxLevel, 4);
    assert_eq!(
        can_use(&c, &sword, &check(&all, false)),
        Err(CantUse::MaxLevel)
    );
    sword.abilities[0] = (Ability::None, 0);
    // Anti Magic (the Witchunter) refuses Magical items
    let mut bow = sword.clone();
    bow.abilities[0] = (Ability::Magical, 1);
    assert_eq!(
        can_use(&c, &bow, &check(&sharp, true)),
        Err(CantUse::AntiMagic)
    );
    assert_eq!(can_use(&c, &bow, &check(&sharp, false)), Ok(()));
    assert_eq!(can_use(&c, &sword, &check(&sharp, true)), Ok(()));
    // Limited: only the three exempt items
    assert_eq!(
        can_use(&c, &sword, &check(&limited, false)),
        Err(CantUse::WeaponClass)
    );
    let qs = weapon(
        EXEMPT_ITEMS[0].0,
        "quarterstaff",
        100,
        WeaponType::TwoHandBlunt,
        30,
        1200,
    );
    assert_eq!(can_use(&c, &qs, &check(&limited, false)), Ok(()));
    // Class Item Inclusion
    sword.abilities[0] = (Ability::ClassItem, 2);
    c.class = ClassNo(2);
    assert_eq!(can_use(&c, &sword, &check(&limited, false)), Ok(()));
    // armour: heavier than the class wears; off-hand for 1-handed classes
    let mut plate = armour(1, "plate", 1, WornLocation::Torso);
    plate.armtype = ArmourType::Platemail;
    let mut robes_only = class(5, WeaponClass::All);
    robes_only.armour = ArmourType::Robes;
    assert_eq!(
        can_use(&c, &plate, &check(&robes_only, false)),
        Err(CantUse::Armour)
    );
    let shield = armour(2, "shield", 1, WornLocation::OffHand);
    assert_eq!(
        can_use(&c, &shield, &check(&sharp, false)),
        Err(CantUse::Armour)
    );
    assert_eq!(can_use(&c, &shield, &check(&all, false)), Ok(()));
    // alignment by legal level
    let mut evil_item = armour(3, "dark cloak", 1, WornLocation::Back);
    evil_item.abilities[0] = (Ability::Evil, 1);
    assert_eq!(
        can_use(&c, &evil_item, &check(&all, false)),
        Err(CantUse::Alignment)
    );
    c.evil = 40;
    assert_eq!(can_use(&c, &evil_item, &check(&all, false)), Ok(()));
}

proptest! {
    // a list that names anyone must name the character; an empty
    // list (all zero) admits everyone
    #[test]
    fn can_use_class_and_race_lists(
        classes in proptest::collection::vec(0u16..6, 10),
        races in proptest::collection::vec(0u16..6, 10),
        my_class in 1u16..6,
        my_race in 1u16..6,
    ) {
        let mut c = silvermere_engine::clear_player();
        c.class = ClassNo(my_class);
        c.race = RaceNo(my_race);
        let all = class(my_class, WeaponClass::All);
        let mut it = armour(1, "cloak", 1, WornLocation::Back);
        for (i, n) in classes.iter().enumerate() {
            it.classes[i] = ClassNo(*n);
        }
        for (i, n) in races.iter().enumerate() {
            it.races[i] = RaceNo(*n);
        }
        let class_ok = classes.iter().all(|&n| n == 0) || classes.contains(&my_class);
        let race_ok = races.iter().all(|&n| n == 0) || races.contains(&my_race);
        let got = can_use(&c, &it, &check(&all, false));
        let want = if !class_ok {
            Err(CantUse::Class)
        } else if !race_ok {
            Err(CantUse::Race)
        } else {
            Ok(())
        };
        prop_assert_eq!(got, want);
    }
}

// ---------------------------------------------------------------------------------------
// Encumbrance.

// as measured on the original (Kang, Str 79: maximum weight 3792, "520/3792 - None [13%]")
#[test]
fn encumbrance_levels() {
    let mut w = world();
    let id = player(&mut w, "Kang");
    edit(&mut w, id, |c| {
        c.str = 79;
        c.maxwgt = silvermere_rules::economy::carrying_capacity(79);
    });
    assert_eq!(plr(&w, id).maxwgt, 3792);
    for (wgt, pct, level) in [
        (0u16, 0i16, EncumbranceLevel::None),
        (520, 13, EncumbranceLevel::None),
        (1252, 33, EncumbranceLevel::Light),
        (1290, 34, EncumbranceLevel::Medium),
        (2503, 66, EncumbranceLevel::Medium),
        (2541, 67, EncumbranceLevel::Heavy),
        (3792, 100, EncumbranceLevel::Heavy),
    ] {
        edit(&mut w, id, |c| c.wgt = wgt);
        let (load, max, got) = encumbrance_of(&w, plr(&w, id));
        assert_eq!((load, max, got), (i64::from(wgt), 3792, pct), "{wgt}");
        assert_eq!(EncumbranceLevel::from_pct(got), level, "{wgt}");
    }
    edit(&mut w, id, |c| c.wgt = 520);
    let ls = cmd(&mut w, id, "i");
    assert_eq!(ls.last().unwrap(), "Encumbrance: 520/3792 - None [13%]");
    assert_matches(&ls, "inv_encumbrance");
    // three coins weigh one unit, each type on its own
    edit(&mut w, id, |c| c.coins.gold = 300);
    let (load, _, _) = encumbrance_of(&w, plr(&w, id));
    assert_eq!(load, 620);
    // Alter Encumbrance as a percentage
    w.classes.get_mut(&ClassNo(1)).unwrap().abilities[0] = (Ability::Encumbrance, 50);
    let (_, max, _) = encumbrance_of(&w, plr(&w, id));
    assert_eq!(max, 5688);
}

// ---------------------------------------------------------------------------------------
// The inventory display against the capture.

const STATUS: &[u8] = include_bytes!("../../../tests/captures/status-screens/raw.bin");

/// The capture's reply to `i` after the Newhaven starter set: from the command's ANSI_NORMAL
/// to the line end before the prompt.
fn captured_i() -> &'static [u8] {
    let start = STATUS
        .windows(4)
        .position(|w| w == b"i\r\n\x1b")
        .expect("the i command")
        + 3;
    let tail = &STATUS[start..];
    let prompt = tail.windows(6).position(|w| w == b"[HP=35").unwrap();
    // the prompt's own ANSI_CLRLINE and colour precede it
    &tail[..prompt - b"\x1b[79D\x1b[K\x1b[0;37m".len()]
}

// display_inventory_items, wrapped at 79 columns (with no coins the line starts with an
// item), byte for byte after wearing the padded set and wielding the
// quarterstaff (status-screens capture)
#[test]
fn inventory_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    for n in [VEST, PANTS, HELM, GLOVES, BOOTS, QUARTERSTAFF] {
        give(&mut w, id, n);
    }
    for c in [
        "wear padded vest",
        "wear padded pants",
        "wear padded helm",
        "wear padded gloves",
        "wear padded boots",
        "wield quarterstaff",
    ] {
        cmd(&mut w, id, c);
    }
    execute_input(&mut w, id, "i", false);
    let got = take(&mut w, id);
    let want = captured_i();
    assert_eq!(
        got.escape_ascii().to_string(),
        want.escape_ascii().to_string()
    );
}

// keys: the newest kind first, " N names" for repeats
#[test]
fn keys_list() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    assert_eq!(cmd(&mut w, id, "keys"), ["You have no keys."]);
    give(&mut w, id, IRON_KEY);
    give(&mut w, id, IRON_KEY);
    let mut gate = item(950, "gate key", ItemType::Key, 1);
    gate.charges = 1;
    w.items.insert(gate.number, gate);
    give(&mut w, id, 950);
    assert_eq!(
        cmd(&mut w, id, "keys"),
        ["You have the following keys:  gate key, 2 iron keys."]
    );
    assert_eq!(
        plr(&w, id)
            .items
            .iter()
            .filter(|&&n| n != ItemNo(0))
            .count(),
        0
    );
    assert_eq!(plr(&w, id).keyuses[0], 3);
}

// WEALTH
#[test]
fn wealth() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    assert_eq!(cmd(&mut w, id, "wealth"), ["You have No money!"]);
    edit(&mut w, id, |c| {
        c.coins.gold = 3;
        c.coins.copper = 1;
    });
    assert_eq!(
        cmd(&mut w, id, "wealth"),
        [
            "You have 3 gold crowns, 1 copper farthing.",
            "Wealth: 301 copper farthings"
        ]
    );
}

// ---------------------------------------------------------------------------------------
// Light.

// cmd_light; remove puts it out
#[test]
fn light_a_torch() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    give(&mut w, id, TORCH);
    give(&mut w, id, LANTERN);
    let before = silvermere_engine::display::light::light_at(&w, id)
        .unwrap()
        .0;
    let ls = cmd(&mut w, id, "light torch");
    assert_eq!(ls, ["You lit the torch."]);
    assert_matches(&ls, "light_ok");
    let room = lines(&take(&mut w, bob));
    assert_eq!(room, ["Aldric lights his torch."]);
    assert_matches(&room, "other_wears");
    assert_eq!((plr(&w, id).readied, plr(&w, id).light), (0, 100));
    let after = silvermere_engine::display::light::light_at(&w, id)
        .unwrap()
        .0;
    assert_eq!(after, before + 100);
    let ls = cmd(&mut w, id, "light lantern");
    assert_eq!(ls, ["You already have something lit!"]);
    assert_matches(&ls, "light_fail");
    give(&mut w, id, VEST);
    // (a vest is no light source: the search only offers lights, so the word falls
    // through)
    let ls = cmd(&mut w, id, "remove torch");
    assert_eq!(ls, ["You have removed torch and extinguished it."]);
    assert_matches(&ls, "removed");
    assert_eq!((plr(&w, id).readied, plr(&w, id).light), (-1, 0));
    edit(&mut w, id, |c| c.itemuses[1] = 0);
    assert_eq!(
        cmd(&mut w, id, "light lantern"),
        ["You must recharge that before you may light it again."]
    );
}

// medium_update_character: the lit torch burns one charge per 3 s
// pass; at 0 the item goes with its own destroy message (WCCMSG 8603) and the light level
// falls back when the personal timer runs out. Driven through the engine on TestClock.
#[test]
fn a_torch_burns_out_on_the_medium_tick() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, TORCH);
    let dark = silvermere_engine::display::light::light_at(&w, id)
        .unwrap()
        .0;
    assert_eq!(cmd(&mut w, id, "light torch"), ["You lit the torch."]);
    let lit = silvermere_engine::display::light::light_at(&w, id)
        .unwrap()
        .0;
    assert_eq!(lit, dark + 100);
    let (tx, rx) = std::sync::mpsc::channel();
    let clock = common::TestClock::new();
    let mut eng =
        silvermere_engine::Engine::new(w, clock.clone(), silvermere_engine::SeededRng::new(1), tx);
    // 799 passes leave one charge
    clock.advance_ticks(799 * 3, &mut eng);
    let p = eng.world.session(id).unwrap().player().unwrap();
    assert_eq!(
        (p.items[0], p.itemuses[0], p.readied),
        (ItemNo(TORCH), 1, 0)
    );
    let _ = rx.try_iter().count();
    clock.advance_ticks(3, &mut eng);
    let p = eng.world.session(id).unwrap().player().unwrap();
    assert_eq!((p.items[0], p.readied, p.wgt), (ItemNo(0), -1, 0));
    let said: Vec<u8> = rx
        .try_iter()
        .filter_map(|o| match o {
            EngineOutput::Write { id: to, bytes } if to == id => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        lines(&said)
            .iter()
            .any(|l| l == "Your torch flickers and goes out."),
        "{:?}",
        lines(&said)
    );
    clock.advance_ticks(3, &mut eng);
    let after = silvermere_engine::display::light::light_at(&eng.world, id)
        .unwrap()
        .0;
    assert_eq!(after, dark, "the personal light timer ran out");
}

// ---------------------------------------------------------------------------------------
// Limited items.

// handing out a limited item: a copy only while fewer than `limit` are out;
// "A strange force stops you ..."; the release
#[test]
fn limited_copies_run_out() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    let bob = player(&mut w, "Bob");
    assert!(add_item_to_inventory(&mut w, id, ItemNo(RELIC), -2));
    assert_eq!(w.items[&ItemNo(RELIC)].active, 1);
    let holders = limited::holders_of(&w, ItemNo(RELIC));
    assert_eq!(holders.len(), 1);
    assert_eq!(holders[0].userid, "ALDRIC");
    assert!(!add_item_to_inventory(&mut w, bob, ItemNo(RELIC), -2));
    assert_eq!(
        lines(&take(&mut w, bob)),
        ["A strange force stops you from getting this item."]
    );
    // dropping it moves the copy to the room; Bob picks it up
    cmd(&mut w, id, "drop relic");
    take(&mut w, bob);
    let holders = limited::holders_of(&w, ItemNo(RELIC));
    assert_eq!(holders.len(), 1);
    assert_eq!(holders[0].roomnum, room_key().room);
    assert_eq!(cmd(&mut w, bob, "get relic"), ["You took relic."]);
    assert_eq!(limited::holders_of(&w, ItemNo(RELIC))[0].userid, "BOB");
    assert_eq!(w.items[&ItemNo(RELIC)].active, 1);
}

// The writes go to the persistence worker: ownership rows, the active count, the room
fn persisted(w: &mut World) -> Vec<PersistRequest> {
    let mut out = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut eng = silvermere_engine::Engine::new(
        std::mem::replace(w, common::tiny_world(1)),
        common::TestClock::new(),
        silvermere_engine::SeededRng::new(1),
        tx,
    );
    eng.handle(silvermere_engine::EngineInput::Tick);
    for o in rx.try_iter() {
        if let EngineOutput::Persist(r) = o {
            out.push(r);
        }
    }
    out
}

#[test]
fn ownership_and_floor_writes_are_queued() {
    let mut w = world();
    let id = player(&mut w, "Aldric");
    give(&mut w, id, RELIC);
    cmd(&mut w, id, "drop relic");
    let reqs = persisted(&mut w);
    assert!(reqs.iter().any(
        |r| matches!(r, PersistRequest::ItemOwnershipInsert(o) if o.itemnum == ItemNo(RELIC))
    ));
    assert!(
        reqs.iter()
            .any(|r| matches!(r, PersistRequest::ItemOwnershipDelete(_)))
    );
    assert!(reqs.iter().any(
        |r| matches!(r, PersistRequest::ItemActive { item, active: 1 } if *item == ItemNo(RELIC))
    ));
    assert!(
        reqs.iter()
            .any(|r| matches!(r, PersistRequest::RoomState(s) if s.items[0].item == ItemNo(RELIC)))
    );
}

// Currency names and proper_currency_name
#[test]
fn currency_words() {
    assert_eq!(Currency::Gold.singular_name(), Some("gold crown"));
}
