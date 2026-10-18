//! The room display (T-025): `display_room_desc` and its parts, rendered to bytes and
//! compared with the vendored captures of the original (`tests/captures/`), the pattern
//! registry (`tests/fixtures/patterns.json`) and `insta` snapshots.
//!
//! The rooms are the stock records of the captures' rooms (map 1, rooms 2140 and 2150 of
//! `WCCMP001`), typed in by hand. The live direction names carry a random letter and a
//! backspace in the original's output; comparisons with a
//! capture drop every `X\b` pair on both sides.

mod common;

use common::{enter, tiny_snapshot};
use silvermere_engine::commands::info::{cmd_look_room, display_visible_exits};
use silvermere_engine::display::room_items::{search_room, users_hidden_in_room};
use silvermere_engine::display::{Assets, RoomMode, display_entry_movement, display_room};
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{SeededRng, SessionId, World, WorldSnapshot};
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{
    Alignment, DebugLevel, Direction, ExitType, GroupRole, MessageLevel, MonsterType, RoomType,
    ShopType,
};
use silvermere_types::flags::{ActiveMonsterFlags, ExitFlags, PlayerStatus};
use silvermere_types::ids::{
    ActiveMonsterId, ItemNo, MapId, MessageNo, MonsterNo, RoomId, RoomKey, ShopNo, SpellNo,
};
use silvermere_types::records::{
    ActiveMonster, ActiveSpell, Coins, Exit, Message, Room, RoomItem, Shop, ShopSlot,
};

// The stock distribution, downloaded once and checked (the engine may not depend on the
// testkit, tests/deps.rs, so its helper is included by path).
#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock;

const ENTRY: &[u8] = include_bytes!("../../../tests/captures/entry-existing/raw.bin");
const ARENA: &[u8] = include_bytes!("../../../tests/captures/fight-arena/raw.bin");
const WALK: &[u8] = include_bytes!("../../../tests/captures/room-walk/raw.bin");

// ---------------------------------------------------------------------------------------
// The world.

const ENTRANCE: u32 = 2140;
const ARENA_ROOM: u32 = 2150;
const SLUM: u32 = 3000;
const BARE: u32 = 3001;
const DOORS: u32 = 3002;
const SECRETS: u32 = 3003;
const COLLISEUM: u32 = 3004;
const FILE_ROOM: u32 = 3005;
const BANK: u32 = 3006;
const LITTER: u32 = 3007;

fn exit(target: u32, kind: ExitType, par1: u32, par2: i16) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1,
        par2,
        par3: 0,
        par4: 0,
    }
}

fn desc(lines: &[&str]) -> [String; 7] {
    std::array::from_fn(|i| lines.get(i).map_or(String::new(), |l| (*l).to_owned()))
}

fn room(n: u32, name: &str) -> Room {
    let mut r = blank::clear_room(RoomKey::new(1, n));
    r.name = name.into();
    r
}

fn item(world_items: &mut Vec<silvermere_types::records::Item>, n: i32, name: &str) {
    let mut i = blank::clear_item(ItemNo(n));
    i.name = name.into();
    i.gettable = 1;
    world_items.push(i);
}

/// Room 1/2140 of WCCMP001, as the stock database holds it.
fn village_entrance() -> Room {
    let mut r = room(ENTRANCE, "Newhaven, Village Entrance");
    r.desc = desc(&[
        "Welcome to Newhaven! You are standing at the crude wooden gates of the",
        "village entrance, in the middle of a dusty path. A low wooden palisade",
        "surrounds the village, to protect against creatures of the night and",
        "random raiders. You can see small buildings to the north and south,",
        "and a dusty path that leads to the west and southeast.",
    ]);
    r.exits[0] = exit(2141, ExitType::Normal, 0, 0);
    r.exits[1] = exit(2142, ExitType::Normal, 0, 0);
    r.exits[3] = exit(2143, ExitType::Normal, 0, 0);
    r.exits[6] = exit(2148, ExitType::Normal, 0, 0);
    r.items[0] = RoomItem {
        item: ItemNo(1098),
        uses: -1,
        count: 0,
    };
    r.items[1] = RoomItem {
        item: ItemNo(11),
        uses: -1,
        count: 0,
    };
    r
}

/// Room 1/2150 of WCCMP001, the Newhaven arena: a Door north (closed, `par1` 2), up.
fn newhaven_arena() -> Room {
    let mut r = room(ARENA_ROOM, "Newhaven, Arena");
    r.room_type = RoomType::Arena;
    r.desc = desc(&[
        "This huge room is has been carved into the earth with crude work-",
        "manship. The blood-soaked floor is covered in broken, rusted weaponry",
        "and some things better left unmentioned. A huge iron gate sits in the",
        "west wall, and behind it you can hear the growling of some",
        "unfathomable beast. A large black door is set in the northern wall.",
    ]);
    r.exits[0] = exit(2152, ExitType::Door, 2, 30);
    r.exits[8] = exit(2146, ExitType::Normal, 0, 0);
    r
}

fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    s.rooms.retain(|r| r.key.room.0 != ENTRANCE);
    s.rooms.push(village_entrance());
    s.rooms.push(newhaven_arena());
    // A slum street at -175 (dark).
    let mut slum = room(SLUM, "Slum Street");
    slum.light = -175;
    slum.exits[1] = exit(SLUM + 100, ExitType::Normal, 0, 0);
    s.rooms.push(slum);
    s.rooms.push(room(BARE, "Bare Cell"));
    let mut doors = room(DOORS, "Trap Doors");
    doors.exits[8] = exit(1, ExitType::Door, 1, 0);
    doors.exits[9] = exit(2, ExitType::Gate, 0, 0);
    doors.exits[2] = exit(3, ExitType::Key, 0, 1);
    doors.exits[3] = exit(4, ExitType::Gate, 1, 0);
    doors.exits[4] = exit(5, ExitType::Trap, 0, 2); // par2 2: not shown
    doors.exits[5] = exit(6, ExitType::Text, 0, 0); // never shown
    doors.exits[6] = exit(7, ExitType::Trap, 0, 0); // par2 not 2: shown by name
    s.rooms.push(doors);
    let mut secrets = room(SECRETS, "Secret Passages");
    for (i, e) in secrets.exits.iter_mut().enumerate() {
        *e = exit(
            10 + i as u32,
            ExitType::Hidden,
            ExitFlags::VISSEARCHED.bits(),
            0,
        );
    }
    s.rooms.push(secrets);
    let mut coll = room(COLLISEUM, "Practice Pit");
    coll.room_type = RoomType::Colliseum;
    s.rooms.push(coll);
    let mut file = room(FILE_ROOM, "Gang House");
    file.desc = desc(&["FILE DESCRIPTION", "WCC85015.HSE", ""]);
    s.rooms.push(file);
    let mut bank = room(BANK, "Newhaven, Bank");
    bank.room_type = RoomType::Shop;
    bank.shopnum = ShopNo(7);
    bank.desc = desc(&["A quiet counting house."]);
    s.rooms.push(bank);
    let mut litter = room(LITTER, "Junk Heap");
    for i in 0..12 {
        litter.items[i] = RoomItem {
            item: ItemNo(100 + i as i32),
            uses: -1,
            count: (i % 3) as i16,
        };
    }
    litter.coins = Coins {
        copper: 9,
        silver: 1,
        gold: 0,
        platinum: 0,
        runic: 2,
    };
    s.rooms.push(litter);
    item(&mut s.items, 11, "large sign");
    item(&mut s.items, 1098, "newbie manual");
    for (i, name) in [
        "rusty dagger",
        "torn cloak",
        "leather boots",
        "wooden shield",
        "silver ring",
        "copper bracelet",
        "dented helm",
        "bone whistle",
        "broken lantern",
        "coil of rope",
        "battered tin cup",
        "faded scroll",
    ]
    .iter()
    .enumerate()
    {
        item(&mut s.items, 100 + i as i32, name);
    }
    let mut shop = blank_shop(ShopNo(7));
    shop.shop_type = ShopType::Bank;
    shop.desc = [
        "A clerk waits behind iron bars.".into(),
        String::new(),
        String::new(),
    ];
    s.shops.push(shop);
    s.messages.push(Message {
        number: MessageNo(1256),
        msgs: [
            "a narrow crack leading %s".into(),
            String::new(),
            String::new(),
        ],
    });
    s
}

fn blank_shop(n: ShopNo) -> Shop {
    Shop {
        number: n,
        name: String::new(),
        desc: Default::default(),
        shop_type: ShopType::General,
        minlevel: 0,
        maxlevel: 0,
        markup: 0,
        ownable: 0,
        expandable: 0,
        trainclass: Default::default(),
        stock: std::array::from_fn(|_| ShopSlot {
            item: ItemNo(0),
            maxqty: 0,
            qty: 0,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        }),
    }
}

fn world() -> World {
    let mut w = World::load(GameConfig::default(), 16, snapshot()).expect("world loads");
    // the exits as built are their live states: drop the start-up reset laid over them
    // (T-035)
    for s in w.room_state.values_mut() {
        s.exit_overrides = Default::default();
    }
    w
}

fn monster(world: &mut World, n: i32, room: u32, name: &str, alignment: Alignment) {
    let m = ActiveMonster {
        number: ActiveMonsterId(n),
        knownnum: MonsterNo(1),
        level: 1,
        map: MapId(1),
        room: RoomId(room),
        poison: 0,
        energy: 0,
        hp: 5,
        following: String::new(),
        trail: std::array::from_fn(|_| RoomId::default()),
        hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
        hunting: ActiveMonsterId::default(),
        name: name.into(),
        wornitem: ItemNo::default(),
        weapon: ItemNo::default(),
        items: std::array::from_fn(|_| (ItemNo::default(), 0)),
        cash: Coins::default(),
        maxhp: 5,
        alignment,
        followpct: 0,
        dr: 0,
        ac: 0,
        exp: 0,
        maxenergy: 0,
        friendly: 0,
        genroom: RoomId(room),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: MonsterType::Lair,
        target: 0,
        hpregen: 0,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::Solo,
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: SpellNo::default(),
            value: 0,
            rounds: 0,
        }),
        downrounds: 0,
        created: 0,
    };
    world.active_monsters.insert(m);
    let state = world
        .room_state
        .get_mut(&RoomKey::new(1, room))
        .expect("room state");
    let slot = state.monsters.iter().position(|m| m.0 == 0).unwrap();
    state.monsters[slot] = ActiveMonsterId(n);
}

fn player(world: &mut World, name: &str, room: u32) -> SessionId {
    enter(world, name, room, MessageLevel::High)
}

fn edit(
    world: &mut World,
    id: SessionId,
    f: impl FnOnce(&mut silvermere_types::records::Character),
) {
    f(world.session_mut(id).unwrap().character.as_mut().unwrap());
}

/// The bytes the session would receive now (prompts rendered empty), and the batch
/// emptied.
fn take(world: &mut World, id: SessionId) -> Vec<u8> {
    let palette = palette_of(world, id);
    let s = world.session_mut(id).unwrap();
    let out = s.output.render(palette, b"");
    s.output.clear();
    out
}

fn show(world: &mut World, id: SessionId, mode: RoomMode) -> Vec<u8> {
    display_room(world, id, mode);
    take(world, id)
}

/// Every `X\b` pair dropped: the anti-script letter of `dirnames[]`.
fn drop_backspace_pairs(bytes: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == 8 && out.pop().is_some() {
            continue;
        }
        out.push(b);
    }
    out
}

fn esc(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> usize {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
        .unwrap_or_else(|| panic!("{} not in the capture", esc(needle)))
}

/// The capture from the first `start` (included) to the first `end` after it (included).
fn slice<'a>(cap: &'a [u8], start: &[u8], end: &[u8]) -> &'a [u8] {
    let a = find(cap, start, 0);
    let b = find(cap, end, a) + end.len();
    &cap[a..b]
}

/// The screen lines of `bytes`: ANSI and `X\b` pairs removed, split at `\r\n`.
fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(&drop_backspace_pairs(bytes));
    String::from_utf8_lossy(&plain)
        .split("\r\n")
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
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

fn assert_line_matches(bytes: &[u8], name: &str) {
    let rx = pattern(name);
    let ls = lines(bytes);
    assert!(ls.iter().any(|l| rx.is_match(l)), "{name}: {ls:?}");
}

// ---------------------------------------------------------------------------------------
// Against the captures.

// The room shown on entering the game, as display_room_desc draws it in the player's mode
#[test]
fn entry_room_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ENTRANCE);
    let got = show(&mut w, id, RoomMode::LONG);
    let want = slice(
        ENTRY,
        b"\x1b[79D\x1b[K\x1b[1;36mNewhaven, Village",
        b"outheast\r\n",
    );
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(want))
    );
    assert_line_matches(&got, "room_items");
    assert_line_matches(&got, "room_exits");
    insta::assert_snapshot!("entry_long", esc(&got));
}

// The short display on an empty line
#[test]
fn short_room_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ENTRANCE);
    let got = show(&mut w, id, RoomMode::SHORT);
    let a = find(ENTRY, b"]:\r\n\x1b[79D\x1b[K\x1b[1;36mNewhaven", 0) + 4;
    let b = find(ENTRY, b"outheast\r\n", a) + 10;
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(&ENTRY[a..b]))
    );
}

// A closed door; an evil monster in COL_HERENAMES
#[test]
fn arena_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ARENA_ROOM);
    monster(&mut w, 1, ARENA_ROOM, "large giant rat", Alignment::Evil);
    let got = show(&mut w, id, RoomMode::LONG);
    let want = slice(
        ARENA,
        b"\x1b[79D\x1b[K\x1b[1;36mNewhaven, Arena",
        b"\r\n\x1b[79D\x1b[K\x1b[0;37m[HP",
    );
    let want = &want[..want.len() - b"\x1b[79D\x1b[K\x1b[0;37m[HP".len()];
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(want))
    );
    let ls = lines(&got);
    assert!(
        ls.contains(&"Obvious exits: closed door north, up".to_string()),
        "{ls:?}"
    );
    assert_line_matches(&got, "room_also_here");
    insta::assert_snapshot!("arena_long", esc(&got));
}

// The glue of survey 1.4: the room follows the prompt with no line end of its own,
// because the room display starts with ANSI_CLRLINE.
#[test]
fn arena_follows_a_glued_prompt() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ARENA_ROOM);
    edit(&mut w, id, |c| c.hp = 48);
    let s = w.session_mut(id).unwrap();
    s.push_prompt();
    display_room(&mut w, id, RoomMode::SHORT);
    let palette = palette_of(&w, id);
    let prompt = silvermere_engine::tell::prompt_bytes(&w, id);
    let got = w.session(id).unwrap().output.render(palette, &prompt);
    let text = String::from_utf8_lossy(&drop_backspace_pairs(&got)).into_owned();
    assert!(
        text.contains("[HP=48\x1b[0;37m]:\x1b[79D\x1b[K\x1b[1;36mNewhaven, Arena\r\n"),
        "{}",
        esc(text.as_bytes())
    );
    assert!(text.ends_with("Obvious exits: closed door north, up\r\n"));
}

// can_see: below LIGHT_DARK only the darkness line, after CLRLINE
#[test]
fn dark_room_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric", SLUM);
    let got = show(&mut w, id, RoomMode::SHORT);
    let want = slice(
        WALK,
        b"\x1b[79D\x1b[KThe room is very dark",
        b"can't see anything\r\n",
    );
    assert_eq!(esc(&got), esc(want));
    assert_line_matches(&got, "room_dark");
}

// The light level: a lit torch (+100, kept in player.light) brings
// -175 to -75; the light line in grey below 0
#[test]
fn dark_room_with_a_lit_torch() {
    let mut w = world();
    let id = player(&mut w, "Aldric", SLUM);
    edit(&mut w, id, |c| c.light = 100);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert!(
        esc(&got).ends_with("\\x1b[0;37mThe room is dimly lit\\r\\n"),
        "{}",
        esc(&got)
    );
    assert_line_matches(&got, "room_light");
    insta::assert_snapshot!("slum_torch", esc(&got));
    // a second player's torch counts too
    let other = player(&mut w, "Bob", SLUM);
    edit(&mut w, other, |c| c.light = 0);
    edit(&mut w, id, |c| c.light = 0);
    assert!(lines(&show(&mut w, id, RoomMode::SHORT))[0].starts_with("The room is very dark"));
    edit(&mut w, other, |c| c.roomlight = 30);
    edit(&mut w, id, |c| c.light = 30);
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls.last().unwrap(), "The room is barely visible");
}

// can_see: blind
#[test]
fn blind_viewer() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ENTRANCE);
    edit(&mut w, id, |c| c.status |= PlayerStatus::BLIND);
    let got = show(&mut w, id, RoomMode::LONG);
    assert_eq!(esc(&got), "\\x1b[79D\\x1b[KYou are blind.\\r\\n");
    assert_line_matches(&got, "blind");
}

// can_see: over LIGHT_BLIND, the description alone
#[test]
fn blinding_light() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ENTRANCE);
    edit(&mut w, id, |c| c.light = 950);
    // capped at 900, which is not over LIGHT_BLIND: the room shows
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls[0], "Newhaven, Village Entrance");
}

// ---------------------------------------------------------------------------------------
// Exits.

#[test]
fn no_visible_exit() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&got),
        "\\x1b[79D\\x1b[K\\x1b[1;36mBare Cell\\r\\n\\x1b[0;32mObvious exits: NONE!!!\\r\\n"
    );
    assert_line_matches(&got, "room_exits");
}

/// The exits line, unwrapped.
fn exits_line(got: &[u8]) -> String {
    let ls = lines(got);
    let i = ls
        .iter()
        .position(|l| l.starts_with("Obvious exits: "))
        .unwrap();
    ls[i..].join(" ")
}

// Up and down doors, keys and gates are trap doors; a Text exit is
// never listed, a Trap exit only while its second parameter is not 2
#[test]
fn trap_doors() {
    let mut w = world();
    let id = player(&mut w, "Aldric", DOORS);
    assert_eq!(
        exits_line(&show(&mut w, id, RoomMode::SHORT)),
        "Obvious exits: closed door east, closed gate west, southeast, closed trap door above, \
         open trap door below"
    );
}

// Each direction's secret passage wording, with the sic capitals
// for north and east; a found exit with a message prints its first text
#[test]
fn secret_passages_per_direction() {
    let mut w = world();
    let id = player(&mut w, "Aldric", SECRETS);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        exits_line(&got),
        "Obvious exits: Secret passage north, secret passage south, secret passage East, \
         secret passage west, secret passage northeast, secret passage northwest, secret \
         passage southeast, secret passage southwest, secret passage above, secret passage \
         below"
    );
    insta::assert_snapshot!("secret_passages", esc(&got));
    // a hidden exit with a message, and one not found yet
    let key = RoomKey::new(1, SECRETS);
    let room = w.rooms.get_mut(&key).unwrap();
    room.exits[0].par4 = 1256;
    room.exits[1].par1 = 0;
    for e in &mut room.exits[2..] {
        e.target = RoomId(0);
        e.kind = ExitType::Normal;
    }
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls[1], "Obvious exits: a narrow crack leading north");
}

// The plain names are the live dirnames[] bytes
#[test]
fn exits_print_the_live_direction_names() {
    let mut w = world();
    w.display.dirnames.set_letter(0, 'J');
    let id = player(&mut w, "Aldric", ENTRANCE);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert!(
        esc(&got).contains("Obvious exits: nJ\\x08orth, s \\x08outh"),
        "{}",
        esc(&got)
    );
}

// ---------------------------------------------------------------------------------------
// Also here.

// Players first, in channel order; `*` for an outlaw; `(Hidden)`
// with no space for a hidden player the viewer can see
#[test]
fn also_here_players() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    let bob = player(&mut w, "Bob", BARE);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&lines_with_also(&got)),
        "\\x1b[0;35mAlso here: \\x1b[1;35mBob\\x1b[0;35m.\\r\\n\\x1b[0m"
    );
    assert_line_matches(&got, "room_also_here");
    // an outlaw and a hidden player: unseen without See Hidden
    edit(&mut w, bob, |c| c.evil = 40);
    let ann = player(&mut w, "Ann", BARE);
    edit(&mut w, ann, |c| c.hidden = 1);
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls[1], "Also here: Bob*.");
    // seen with See Hidden
    edit(&mut w, id, |c| c.status |= PlayerStatus::SEEHIDDEN);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(lines(&got)[1], "Also here: Bob*, Ann(Hidden).");
    insta::assert_snapshot!("also_here_players", esc(&got));
}

/// The `Also here:` message's bytes in `got`.
fn lines_with_also(got: &[u8]) -> Vec<u8> {
    let a = find(got, b"\x1b[0;35mAlso here: ", 0);
    let b = find(got, b".\r\n\x1b[0m", a) + 7;
    got[a..b].to_vec()
}

// Good and neutral in cyan, lawful good in grey under 40 evil
// points, evil in COL_HERENAMES; each name then ANSI_RESET, the next after COL_HERENAMES
// and a comma
#[test]
fn also_here_monsters_by_alignment() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    monster(&mut w, 1, BARE, "ferryman", Alignment::Good);
    monster(&mut w, 2, BARE, "Nathaniel", Alignment::LawfulGood);
    monster(&mut w, 3, BARE, "large giant rat", Alignment::Evil);
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&lines_with_also(&got)),
        "\\x1b[0;35mAlso here: \\x1b[0;36mferryman\\x1b[0m\\x1b[1;35m, \\x1b[0;37mNathaniel\
         \\x1b[0m\\x1b[1;35m, \\x1b[1;35mlarge giant rat\\x1b[0m\\x1b[0;35m.\\r\\n\\x1b[0m"
    );
    insta::assert_snapshot!("also_here_monsters", esc(&got));
    // an outlaw viewer: lawful good in COL_HERENAMES, lawful evil in grey
    edit(&mut w, id, |c| c.evil = 40);
    w.active_monsters
        .values_mut()
        .find(|m| m.number == ActiveMonsterId(3))
        .unwrap()
        .alignment = Alignment::LawfulEvil;
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&lines_with_also(&got)),
        "\\x1b[0;35mAlso here: \\x1b[0;36mferryman\\x1b[0m\\x1b[1;35m, \\x1b[1;35mNathaniel\
         \\x1b[0m\\x1b[1;35m, \\x1b[0;37mlarge giant rat\\x1b[0m\\x1b[0;35m.\\r\\n\\x1b[0m"
    );
}

// " (Charmed)" for a charmed monster following the viewer; the
// room-walk capture's monsters (cyan ferryman, grey Nathaniel) render the same
#[test]
fn charmed_monster_and_player_then_monster() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    let _bob = player(&mut w, "Bob", BARE);
    monster(&mut w, 1, BARE, "kobold", Alignment::ChaoticEvil);
    let m = w
        .active_monsters
        .values_mut()
        .find(|m| m.number == ActiveMonsterId(1))
        .unwrap();
    m.flags |= ActiveMonsterFlags::CHARMED;
    m.following = "aldric".into();
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&lines_with_also(&got)),
        "\\x1b[0;35mAlso here: \\x1b[1;35mBob\\x1b[1;35m, \\x1b[1;35mkobold (Charmed)\\x1b[0m\
         \\x1b[0;35m.\\r\\n\\x1b[0m"
    );
    // Bob sees no "(Charmed)": it follows Aldric
    let got = show(&mut w, _bob, RoomMode::SHORT);
    assert_eq!(lines(&got)[1], "Also here: Aldric, kobold.");
}

#[test]
fn capture_monster_colours() {
    for (name, alignment, colour) in [
        ("ferryman", Alignment::Neutral, "\x1b[0;36m"),
        ("Nathaniel", Alignment::LawfulGood, "\x1b[0;37m"),
    ] {
        let mut w = world();
        let id = player(&mut w, "Aldric", BARE);
        monster(&mut w, 1, BARE, name, alignment);
        let got = show(&mut w, id, RoomMode::SHORT);
        let want = format!("\x1b[0;35mAlso here: {colour}{name}\x1b[0m\x1b[0;35m.\r\n\x1b[0m");
        assert!(find(WALK, want.as_bytes(), 0) > 0);
        assert_eq!(lines_with_also(&got), want.as_bytes());
    }
}

// A monster listed here but recorded in another room that lists it
// is dropped from this room; one no room lists is moved here (and not shown this time);
// a monster that is gone is dropped
#[test]
fn monster_list_repair() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    monster(&mut w, 1, BARE, "stray", Alignment::Good);
    monster(&mut w, 2, BARE, "lost", Alignment::Good);
    w.room_state
        .get_mut(&RoomKey::new(1, BARE))
        .unwrap()
        .monsters[2] = ActiveMonsterId(99);
    monster(&mut w, 3, SLUM, "", Alignment::Good);
    // 1 is recorded in the slum, which lists it too (slot 1 of the slum)
    let slum = RoomKey::new(1, SLUM);
    w.room_state.get_mut(&slum).unwrap().monsters[1] = ActiveMonsterId(1);
    for m in w.active_monsters.values_mut() {
        if m.number.0 == 1 || m.number.0 == 2 {
            m.room = RoomId(SLUM);
        }
    }
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls[1], "Obvious exits: NONE!!!");
    let here = w.room_state[&RoomKey::new(1, BARE)].monsters;
    assert_eq!(
        &here[..3],
        &[ActiveMonsterId(0), ActiveMonsterId(2), ActiveMonsterId(0)]
    );
    let lost = w
        .active_monsters
        .values()
        .find(|m| m.number.0 == 2)
        .unwrap();
    assert_eq!(lost.room, RoomId(BARE));
    // next time the corrected monster shows
    assert_eq!(
        lines(&show(&mut w, id, RoomMode::SHORT))[1],
        "Also here: lost."
    );
}

// ---------------------------------------------------------------------------------------
// The rest of the screen.

// A colliseum while arenasafe is off
#[test]
fn colliseum_warning() {
    let mut w = world();
    let id = player(&mut w, "Aldric", COLLISEUM);
    assert_eq!(lines(&show(&mut w, id, RoomMode::SHORT)).len(), 2);
    w.display.arena_safe = false;
    let got = show(&mut w, id, RoomMode::SHORT);
    assert_eq!(
        esc(&got),
        "\\x1b[79D\\x1b[K\\x1b[1;36mPractice Pit\\r\\n\\x1b[79D\\x1b[K\\x1b[1;31mNOTE: This room is \
         *not* a death-free zone right now!\\r\\nTreat this room as you would any other combat \
         zone.\\r\\n\\x1b[1;36m\\x1b[0;32mObvious exits: NONE!!!\\r\\n"
    );
    assert_line_matches(&got, "arena_warning");
    // an Arena room (type 2) is not a colliseum
    let id2 = player(&mut w, "Bob", ARENA_ROOM);
    assert!(
        !lines(&show(&mut w, id2, RoomMode::SHORT))
            .iter()
            .any(|l| l.starts_with("NOTE"))
    );
}

// cmd_look: the room with its description; the others are told
#[test]
fn look_adds_the_description_and_tells_the_room() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ENTRANCE);
    let bob = player(&mut w, "Bob", ENTRANCE);
    cmd_look_room(&mut w, id);
    let got = take(&mut w, id);
    let ls = lines(&got);
    assert_eq!(ls[0], "Newhaven, Village Entrance");
    assert!(ls[1].starts_with("    Welcome to Newhaven!"));
    assert_eq!(
        ls.last().unwrap(),
        "Obvious exits: north, south, west, southeast"
    );
    let told = take(&mut w, bob);
    assert_eq!(
        esc(&told),
        "\\x1b[79D\\x1b[KAldric is looking around the room.\\r\\n"
    );
    // hidden: nobody is told; blind: only the reason
    edit(&mut w, id, |c| c.hidden = 1);
    cmd_look_room(&mut w, id);
    take(&mut w, id);
    assert!(take(&mut w, bob).is_empty());
    edit(&mut w, id, |c| c.status |= PlayerStatus::BLIND);
    cmd_look_room(&mut w, id);
    assert_eq!(esc(&take(&mut w, id)), "You are blind.\\r\\n");
}

// A sysop with debug on
#[test]
fn debug_line() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BARE);
    w.session_mut(id).unwrap().flags.debug = DebugLevel::On;
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls[0], format!("DBG: {BARE} Map: 1"));
}

// A FILE DESCRIPTION room shows its file's
// first line as the title and the whole file for LOOK, each line without its trailing
// blanks (the text-file scanner's `tfsrdl`, as MBBSEmu has it; T-107)
#[test]
fn file_description_room() {
    let mut w = world();
    let dir = stock::assets_dir();
    w.display.assets = Assets::load_dir(&dir).expect("the stock assets");
    let file = std::fs::read(dir.join("WCC85015.HSE")).unwrap();
    let id = player(&mut w, "Aldric", FILE_ROOM);
    let got = show(&mut w, id, RoomMode::SHORT);
    let text = String::from_utf8(file).unwrap();
    let trimmed: Vec<&str> = text.lines().map(str::trim_end).collect();
    assert!(
        text.lines().any(|l| l != l.trim_end()),
        "the stock file has trailing blanks"
    );
    let mut want = b"\x1b[79D\x1b[K\x1b[1;36m".to_vec();
    want.extend_from_slice(trimmed[0].as_bytes());
    want.extend_from_slice(b"\r\n\x1b[0;32mObvious exits: NONE!!!\r\n");
    assert_eq!(esc(&got), esc(&want));
    let got = show(&mut w, id, RoomMode::LONG);
    let mut body = b"\x1b[79D\x1b[K\x1b[0;37;40m".to_vec();
    body.extend_from_slice(trimmed.join("\r\n").as_bytes());
    body.extend_from_slice(b"\r\n");
    assert!(
        got.windows(body.len()).any(|w| w == body.as_slice()),
        "{}",
        esc(&got)
    );
    insta::assert_snapshot!("file_description", esc(&got));
    // a missing file: no title at all
    w.display.assets = Assets::default();
    assert_eq!(
        lines(&show(&mut w, id, RoomMode::SHORT)),
        ["Obvious exits: NONE!!!"]
    );
}

// A bank shows its shop text and the conversion rates on LOOK
#[test]
fn bank_description() {
    let mut w = world();
    let id = player(&mut w, "Aldric", BANK);
    let got = show(&mut w, id, RoomMode::LONG);
    insta::assert_snapshot!("bank_long", esc(&got));
    let ls = lines(&got);
    assert_eq!(ls[1], "    A quiet counting house. ");
    assert_eq!(ls[2], "A clerk waits behind iron bars. ");
    assert_eq!(ls[3], "The currency conversion rates are:");
}

// Coins first (runic under the sysop's name), then items counted
// and in reverse slot order of first appearance; wrapped at 79 columns
#[test]
fn many_items_wrap_at_79() {
    let mut w = world();
    let id = player(&mut w, "Aldric", LITTER);
    let got = show(&mut w, id, RoomMode::SHORT);
    let ls = lines(&got);
    assert!(ls[1].starts_with("You notice 2 runic coins, 1 silver noble, 9 copper farthings, "));
    assert!(ls.iter().all(|l| l.chars().count() <= 79), "{ls:?}");
    let joined = ls[1..ls.len() - 1].join(" ");
    assert!(pattern("room_items").is_match(&joined), "{joined}");
    insta::assert_snapshot!("items_wrapped", esc(&got));
}

// An item whose record is gone is cleared from the room
#[test]
fn vanished_item_is_cleared() {
    let mut w = world();
    let key = RoomKey::new(1, BARE);
    w.room_state.get_mut(&key).unwrap().items[0] = RoomItem {
        item: ItemNo(5555),
        uses: -1,
        count: 0,
    };
    let id = player(&mut w, "Aldric", BARE);
    let ls = lines(&show(&mut w, id, RoomMode::SHORT));
    assert_eq!(ls.len(), 2);
    assert_eq!(w.room_state[&key].items[0].item, ItemNo(0));
}

// ---------------------------------------------------------------------------------------
// Noise and search.

// display_entry_movement: the neighbours beyond the arena's up exit
// hear movement below them, with their prompt
#[test]
fn neighbours_hear_movement() {
    let mut w = world();
    let mut road = room(2146, "Newhaven, Narrow Road");
    road.exits[9] = exit(ARENA_ROOM, ExitType::Normal, 0, 0);
    w.rooms.insert(road.key, road);
    let listener = player(&mut w, "Bob", 2146);
    let behind = player(&mut w, "Cid", 2152);
    display_entry_movement(&mut w, Some(Direction::North), RoomKey::new(1, ARENA_ROOM));
    let palette = palette_of(&w, listener);
    let prompt = silvermere_engine::tell::prompt_bytes(&w, listener);
    let got = w.session(listener).unwrap().output.render(palette, &prompt);
    let want = slice(
        ARENA,
        b"\x1b[79D\x1b[K\x1b[0;35mYou hear movement below",
        b"]:",
    );
    assert_eq!(esc(&got[..want.len() - 12]), esc(&want[..want.len() - 12]));
    assert_line_matches(&got, "noise");
    // nothing through the exit the newcomer came by
    assert!(take(&mut w, behind).is_empty());
    display_entry_movement(&mut w, Some(Direction::Up), RoomKey::new(1, ARENA_ROOM));
    assert_eq!(
        lines(&take(&mut w, behind))[0],
        "You hear movement to the south."
    );
}

// The hidden players shown to a searcher; the search's end
#[test]
fn search_spots_and_reports() {
    let mut w = world();
    w.rng = SeededRng::new(3);
    let id = player(&mut w, "Aldric", BARE);
    edit(&mut w, id, |c| c.status |= PlayerStatus::SEEHIDDEN);
    assert!(!search_room(&mut w, id));
    assert_eq!(
        esc(&take(&mut w, id)),
        "\\x1b[0;36mYour search revealed nothing.\\r\\n"
    );
    let bob = player(&mut w, "Bob", BARE);
    let ann = player(&mut w, "Ann", BARE);
    edit(&mut w, bob, |c| {
        c.hidden = 1;
        c.status |= PlayerStatus::SNEAKING;
    });
    edit(&mut w, ann, |c| c.hidden = 1);
    assert_eq!(users_hidden_in_room(&mut w, id), 2);
    let got = take(&mut w, id);
    assert_eq!(
        esc(&got),
        "\\x1b[1;31mYou see Bob, Ann hiding in the shadows.\\r\\n"
    );
    assert_line_matches(&got, "hiding_seen");
    let b = w.session(bob).unwrap().player().unwrap();
    assert_eq!(b.hidden, 0);
    assert!(b.status.contains(PlayerStatus::SPOTTED));
    assert!(!b.status.contains(PlayerStatus::SNEAKING));
    // hidden coins are always found
    w.room_state
        .get_mut(&RoomKey::new(1, BARE))
        .unwrap()
        .hidcoins
        .gold = 3;
    assert!(search_room(&mut w, id));
    assert_eq!(lines(&take(&mut w, id)), ["You notice 3 gold crowns here."]);
}

// CMD_EXITS: can_see, then the exits line alone
#[test]
fn exits_command_equals_the_capture() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ARENA_ROOM);
    display_visible_exits(&mut w, id);
    let got = take(&mut w, id);
    // after the echo comes the ANSI_NORMAL every command's output starts with (the
    // pipeline's, also before the movement screens), then the line itself
    let a = find(WALK, b"]:exits\r\n\x1b[0;37;40m", 0) + 9 + 10;
    let b = find(WALK, b"\r\n", a) + 2;
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(&WALK[a..b]))
    );
    edit(&mut w, id, |c| c.status |= PlayerStatus::BLIND);
    display_visible_exits(&mut w, id);
    assert_eq!(esc(&take(&mut w, id)), "You are blind.\\r\\n");
}

// Through MBBSEmu's text-file scanner (T-107): trailing blanks dropped
// (`tfsrdl`); a line whose prefix is followed only by blanks fails `tfspfx`, which ends the
// block of prefixed lines; the blanks after the prefix are skipped (`tfspst`)
#[test]
fn file_description_scanner_rules() {
    let mut w = world();
    w.display.assets.insert(
        "PFX.TXT",
        b"other\r\nA:  one  \r\nA:two\r\nA:   \r\nA:three\r\n".to_vec(),
    );
    w.display.assets.insert(
        "BLANK.TXT",
        b"first \r\nsecond\t\r\n\r\nafter the blank\r\n".to_vec(),
    );
    let got = |w: &World, f: &str, p: &str, once: bool| {
        silvermere_engine::display::room::desc_from_file(w, f, p, once).unwrap()
    };
    assert_eq!(got(&w, "PFX.TXT", "A:", false), ["one", "two"]);
    assert_eq!(got(&w, "PFX.TXT", "A:", true), ["one"]);
    assert_eq!(got(&w, "BLANK.TXT", "", false), ["first", "second"]);
    assert_eq!(got(&w, "blank.txt", "", true), ["first"]);
    // ASCII blanks only: a trailing CP437 0xFF (U+00A0) is kept
    w.display
        .assets
        .insert("NBSP.TXT", b"art\xff\r\nmore \xff \r\n".to_vec());
    assert_eq!(got(&w, "NBSP.TXT", "", false), ["art\u{a0}", "more \u{a0}"]);
}
