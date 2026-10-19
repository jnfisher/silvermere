//! Graph and content invariants after a full import of the stock `.VIR` set into the test
//! database (`SILVERMERE_DATABASE_URL`: a temp SQLite file by default, a private schema
//! on PostgreSQL in CI), plus the content/state split and re-import idempotency.
//!
//! Counts and routes are the stock world's (its full-record export and its monster
//! alignment counts) and the harness's route check: the six `*loop.mp` routes are
//! committed under `tests/fixtures/routes/` and walked step by step as the route check walks them.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use silvermere_db::repo::Account;
use silvermere_db::{Db, TestDb};
use silvermere_import::decode::{self, Record};
use silvermere_import::importer::{
    Content, Decoded, ImportError, ImportOptions, Importer, read_content,
};
use silvermere_types::ids::{MessageNo, RoomKey};
use silvermere_types::records::Room;
use sqlx::{Column, Row};

use crate::stock as stock_data;

fn stock() -> PathBuf {
    stock_data::vir_dir()
}

async fn import(db: &Db, f: impl FnOnce(&mut ImportOptions)) -> silvermere_import::Report {
    let mut opts = ImportOptions::new(stock());
    f(&mut opts);
    Importer::new(opts, Some(db)).run().await.expect("import")
}

// ---------------------------------------------------------------------------------------
// Exits as mmud_data.Room.links() reads them.

/// `mmud_data.EXITTYPES` index -> name, for the walk rules below.
const ACTION: u16 = 5;
const HIDDEN: u16 = 6;
const DOOR: u16 = 7;
const MAP_CHANGE: u16 = 8;
const TEXT: u16 = 10;
const GATE: u16 = 11;
const KEY: u16 = 2;
const REMOTE_ACTION: u16 = 12;

/// `mmud_data.WALKABLE`: Normal, Door, Gate, Map Change, Text, Delay, Level, Class, Race,
/// Alignment, Trap, Timed, User Count, Block Guard, Action.
const WALKABLE: &[u16] = &[0, 7, 11, 8, 10, 21, 15, 13, 14, 20, 9, 16, 18, 19, 5];

/// `(direction, destination, exit type)` for each exit with a destination; a Map Change
/// exit with a non-zero `par1` leads to that map (`Room.links`).
fn links(room: &Room) -> Vec<(usize, RoomKey, u16)> {
    room.exits
        .iter()
        .enumerate()
        .filter(|(_, e)| e.target.0 != 0)
        .map(|(d, e)| {
            let kind = e.kind.code();
            let map = if kind == MAP_CHANGE && e.par1 != 0 {
                e.par1
            } else {
                room.key.map.0
            };
            (d, RoomKey::new(map, e.target.0), kind)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// routecheck.py

const DIRS: [&str; 10] = [
    "north",
    "south",
    "east",
    "west",
    "northeast",
    "northwest",
    "southeast",
    "southwest",
    "up",
    "down",
];
const DIRCMD: [&str; 10] = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"];

fn long_dir(c: &str) -> Option<usize> {
    DIRS.iter()
        .position(|d| *d == c)
        .or_else(|| DIRCMD.iter().position(|d| *d == c))
        .or(match c {
            "above" => Some(8),
            "below" => Some(9),
            _ => None,
        })
}

struct World {
    rooms: BTreeMap<RoomKey, Room>,
    msgs: HashMap<MessageNo, String>,
}

impl World {
    fn exit_command(&self, room: &Room, d: usize) -> String {
        let e = &room.exits[d];
        if e.kind.code() == TEXT
            && let Some(m) = self.msgs.get(&MessageNo(e.par1 as i32))
            && !m.is_empty()
        {
            return m.clone();
        }
        DIRCMD[d].to_string()
    }

    fn match_exit(&self, room: &Room, cmd: &str) -> Option<(usize, RoomKey, u16)> {
        let c = cmd.trim().to_lowercase();
        let links = links(room);
        for &(d, k, t) in &links {
            if t == TEXT && self.exit_command(room, d).to_lowercase() == c {
                return Some((d, k, t));
            }
        }
        if let Some(d) = long_dir(&c) {
            return links.iter().copied().find(|l| l.0 == d);
        }
        if let Some(phrase) = c.strip_prefix("go ") {
            for &(d, k, t) in &links {
                if t == TEXT && self.exit_command(room, d).to_lowercase().contains(phrase) {
                    return Some((d, k, t));
                }
            }
        }
        None
    }
}

fn title_hash(s: &str) -> u32 {
    s.chars()
        .enumerate()
        .map(|(i, c)| (i as u32 + 1) * c as u32)
        .sum()
}

/// `routecheck.EXIT_BITS`: doors, gates and keyed exits show as doors, hidden, text and
/// action exits not at all, everything else as an open exit.
fn exit_bits(room: &Room) -> u32 {
    links(room).iter().fold(0, |bits, &(d, _, t)| {
        let code = match t {
            DOOR | GATE | KEY => 2,
            HIDDEN | TEXT | REMOTE_ACTION | ACTION => 0,
            _ => 1,
        };
        bits | code << (2 * d)
    })
}

fn megamud_id(room: &Room) -> u32 {
    ((title_hash(&room.name) & 0xFFF) << 20) | exit_bits(room)
}

struct Mp {
    from: u32,
    steps: Vec<(u32, String)>,
}

fn load_mp(path: &Path) -> Mp {
    let bytes = std::fs::read(path).unwrap();
    let text: String = bytes.iter().map(|&b| char::from(b)).collect(); // latin-1
    let mut lines = text
        .lines()
        .map(|l| l.trim_end_matches(['\r', '\n']))
        .filter(|l| !l.trim().is_empty())
        .skip_while(|l| l.starts_with('['));
    let head: Vec<&str> = lines.next().unwrap().split(':').collect();
    let from = u32::from_str_radix(head[0], 16).unwrap();
    let steps = lines
        .map(|l| {
            let mut f = l.splitn(3, ':');
            let rid = u32::from_str_radix(f.next().unwrap(), 16).unwrap();
            let _flags = f.next();
            (rid, f.next().unwrap().to_string())
        })
        .collect();
    Mp { from, steps }
}

fn split_cmd(cmd: &str) -> &str {
    match (cmd.find('['), cmd.ends_with(']')) {
        (Some(i), true) => cmd[..i].trim(),
        _ => cmd.trim(),
    }
}

fn is_move(cmd: &str) -> bool {
    let c = cmd.trim().to_lowercase();
    long_dir(&c).is_some() || c.starts_with("go ") || c == "climb" || c == "enter"
}

struct Walk {
    end: RoomKey,
    broke: bool,
    steps: usize,
    idhits: usize,
}

fn walk(w: &World, start: RoomKey, steps: &[(u32, String)]) -> Walk {
    let mut cur = start;
    let (mut n, mut idhits) = (0, 0);
    for (rid, cmd) in steps {
        let room = &w.rooms[&cur];
        if megamud_id(room) == *rid {
            idhits += 1;
        }
        let mv = split_cmd(cmd);
        let Some((_, next, _)) = w.match_exit(room, mv) else {
            if !is_move(mv) {
                n += 1;
                continue;
            }
            return Walk {
                end: cur,
                broke: true,
                steps: n + 1,
                idhits,
            };
        };
        n += 1;
        if !w.rooms.contains_key(&next) {
            return Walk {
                end: cur,
                broke: true,
                steps: n,
                idhits,
            };
        }
        cur = next;
    }
    Walk {
        end: cur,
        broke: false,
        steps: n,
        idhits,
    }
}

/// `routecheck.best_start`'s ranking: not broken, steps walked, id matches, closed.
type Score = (bool, usize, usize, bool);

/// `routecheck.best_start`: every room with the start id (exact, then the same title
/// hash), keeping the walk that gets furthest with the most id matches.
fn best_start(w: &World, mp: &Mp) -> Option<(RoomKey, Walk)> {
    let exact: Vec<RoomKey> = w
        .rooms
        .values()
        .filter(|r| megamud_id(r) == mp.from)
        .map(|r| r.key)
        .collect();
    let near = w
        .rooms
        .values()
        .filter(|r| megamud_id(r) >> 20 == mp.from >> 20 && !exact.contains(&r.key))
        .map(|r| r.key);
    let mut best: Option<(Score, RoomKey, Walk)> = None;
    for k in exact.clone().into_iter().chain(near) {
        let rep = walk(w, k, &mp.steps);
        let score = (
            !rep.broke,
            rep.steps,
            rep.idhits,
            !rep.broke && rep.end == k,
        );
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, k, rep));
        }
    }
    best.map(|(_, k, rep)| (k, rep))
}

// ---------------------------------------------------------------------------------------
// Checksums of the content tables, for the idempotency check.

const CONTENT_TABLES: &[&str] = &[
    "rooms",
    "room_exits",
    "room_placed_items",
    "items",
    "item_abilities",
    "item_classes",
    "item_races",
    "item_negates",
    "monsters",
    "monster_abilities",
    "monster_attacks",
    "monster_items",
    "monster_spells",
    "spells",
    "spell_abilities",
    "classes",
    "class_abilities",
    "races",
    "race_abilities",
    "shops",
    "shop_items",
    "messages",
    "text_blocks",
    "actions",
    "action_texts",
];

/// A hash of every row of `table` in a stable order, every column but a child table's
/// surrogate `id` (children are deleted and re-inserted on a content write) and the
/// state column `items.active`.
async fn checksum(db: &Db, table: &str) -> (usize, u64) {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}")))
        .fetch_all(&db.pool())
        .await
        .unwrap();
    let parent = matches!(table, "rooms" | "actions");
    // `items.active` is state kept on the content row (copies in play).
    let skip = |name: &str| (!parent && name == "id") || (table == "items" && name == "active");
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| {
            row.columns()
                .iter()
                .filter(|c| !skip(c.name()))
                .map(|c| {
                    let i = c.ordinal();
                    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                        format!("{v:?}")
                    } else if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                        format!("{v:?}")
                    } else if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
                        format!("{v:?}")
                    } else {
                        panic!("{table}.{}: no text, integer or boolean", c.name())
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    lines.sort();
    let mut h = DefaultHasher::new();
    lines.hash(&mut h);
    (lines.len(), h.finish())
}

async fn content_checksums(db: &Db) -> Vec<(&'static str, usize, u64)> {
    let mut out = Vec::new();
    for t in CONTENT_TABLES {
        let (n, h) = checksum(db, t).await;
        out.push((*t, n, h));
    }
    out
}

// ---------------------------------------------------------------------------------------

// The special rooms: start room 2140, reincarnation room 2189, evil reincarnation room 142,
// jail room 376; the game checks at start-up that they exist on map 1.
#[tokio::test]
async fn stock_world_invariants() {
    let tdb = TestDb::from_env().await.unwrap();
    let db: &Db = &tdb;
    let report = import(db, |_| {}).await;
    println!("{report}");
    println!("read {:?}, write {:?}", report.read_time, report.write_time);
    assert_eq!(report.count("WCCMP001.VIR").unwrap().written, 26_720);
    assert!(report.counts.iter().all(|c| !c.skip_is_error));

    let rooms: BTreeMap<RoomKey, Room> = db
        .rooms()
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.key, r))
        .collect();
    assert_eq!(rooms.len(), 26_720);

    let names: HashSet<&str> = rooms.values().map(|r| r.name.as_str()).collect();
    assert_eq!(names.len(), 1_754, "distinct room names");

    for room in [2140, 2189, 142, 376] {
        assert!(
            rooms.contains_key(&RoomKey::new(1, room)),
            "special room 1:{room}"
        );
    }

    // The stock world: 7,745 rooms reachable from Newhaven 1:2140 over WALKABLE exits.
    let start = RoomKey::new(1, 2140);
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(cur) = queue.pop_front() {
        for (_, next, t) in links(&rooms[&cur]) {
            if WALKABLE.contains(&t) && rooms.contains_key(&next) && seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    assert_eq!(seen.len(), 7_745, "rooms reachable from 1:2140");

    // The stock monsters: 688 Chaotic Evil, 164 Neutral, 84 Lawful Good, 71 Good, 45 Neutral
    // Evil, 34 Evil, 15 Lawful Evil.
    let mut align: BTreeMap<u16, usize> = BTreeMap::new();
    for m in db.monsters().all().await.unwrap() {
        *align.entry(m.alignment.code()).or_default() += 1;
    }
    let mut hist: Vec<usize> = align.values().copied().collect();
    hist.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(hist, [688, 164, 84, 71, 45, 34, 15]);
    assert_eq!(
        (
            align[&2], align[&3], align[&4], align[&0], align[&5], align[&1], align[&6]
        ),
        (688, 164, 84, 71, 45, 34, 15),
        "by alignment code"
    );

    // routecheck.py: the six loop routes close.
    let msgs = db
        .messages()
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.number, m.msgs[0].clone()))
        .collect();
    let world = World { rooms, msgs };
    let routes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routes");
    for (file, start, steps) in [
        ("cry1loop.mp", 1028, 66),
        ("labeloop.mp", 1832, 118),
        ("madwloop.mp", 2323, 201),
        ("narnloop.mp", 2150, 2),
        ("slm2loop.mp", 1076, 82),
        ("slmsloop.mp", 2307, 198),
    ] {
        let mp = load_mp(&routes.join(file));
        let (from, rep) = best_start(&world, &mp).unwrap_or_else(|| panic!("{file}: no start"));
        assert_eq!(from, RoomKey::new(1, start), "{file}: start room");
        assert!(!rep.broke, "{file}: broke after {} steps", rep.steps);
        assert_eq!(rep.steps, steps, "{file}: steps walked");
        assert_eq!(rep.end, from, "{file}: does not close");
    }
    tdb.close().await;
}

// The .VIR is the virgin state: after import, state equals the record (04 "Schema rules").
#[tokio::test]
async fn content_equals_initial_state() {
    let tdb = TestDb::from_env().await.unwrap();
    let db: &Db = &tdb;
    import(db, |_| {}).await;
    let content = read_content(&ImportOptions::new(stock())).unwrap();

    let want: Vec<Room> = content
        .rooms
        .iter()
        .map(|d| decode::split_room(&d.record).0)
        .collect();
    let mut want = want;
    want.sort_by_key(|r| r.key);
    // Empty floor slots are not stored: their `uses` (-1 in an empty slot; -2 in some
    // virgin hidden slots) is never read while the item
    // number is 0, and comes back as 0.
    for room in &mut want {
        for slot in room.items.iter_mut().chain(room.hiditems.iter_mut()) {
            if slot.item.0 == 0 {
                slot.uses = 0;
                slot.count = 0;
            }
        }
    }
    let got = db.rooms().all().await.unwrap();
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w, "room {}/{}", w.key.map, w.key.room);
    }
    for d in &content.shops {
        let (shop, stock) = decode::split_shop(&d.record);
        assert_eq!(
            db.shops().get(shop.number).await.unwrap().as_ref(),
            Some(&shop)
        );
        let got = db.shop_stock().get(shop.number).await.unwrap().unwrap();
        assert_eq!(got.qty, stock.qty, "shop {} qty", shop.number);
    }
    let monsters = db.monsters().all().await.unwrap();
    let mut known = content.monsters.clone();
    known.sort_by_key(|d| d.record.number);
    assert_eq!(monsters.len(), known.len());
    for (d, got) in known.iter().zip(&monsters) {
        let (mut m, state) =
            decode::split_monster(&d.record, &decode::LegacyClock::utc_now()).unwrap();
        // Empty item slots are not stored; their uses and drop percentage are only read
        // while the item number is non-zero (when the monster is generated).
        for slot in m.items.iter_mut().filter(|s| s.item.0 == 0) {
            slot.uses = 0;
            slot.pct = 0;
        }
        assert_eq!(got, &m);
        assert_eq!(state.curalive, 0, "monster {} curalive", m.number);
    }
    let items = db.items().all().await.unwrap();
    let mut records = content.items.clone();
    records.sort_by_key(|d| d.record.number);
    assert_eq!(items.len(), records.len());
    for (d, got) in records.iter().zip(&items) {
        let item = decode::to_item(&d.record);
        assert_eq!(got, &item);
        assert_eq!(item.active, 0);
    }
    let mut spells: Vec<_> = content
        .spells
        .iter()
        .map(|d| decode::to_spell(&d.record))
        .collect();
    spells.sort_by_key(|s| s.number);
    assert_eq!(db.spells().all().await.unwrap(), spells);
    let mut texts: Vec<_> = content
        .text_blocks
        .iter()
        .map(|d| decode::to_text_block(&d.record))
        .collect();
    texts.sort_by_key(|t| (t.number, t.part));
    assert_eq!(db.text_blocks().all().await.unwrap(), texts);
    // T-108: written in bulk statements; the same rows as one by one
    let mut messages: Vec<_> = content
        .messages
        .iter()
        .map(|d| decode::to_message(&d.record))
        .collect();
    messages.sort_by_key(|m| m.number);
    assert_eq!(db.messages().all().await.unwrap(), messages);
    tdb.close().await;
}

#[tokio::test]
async fn content_only_reimport_is_idempotent_and_keeps_state() {
    let tdb = TestDb::from_env().await.unwrap();
    let db: &Db = &tdb;
    import(db, |_| {}).await;
    let first = content_checksums(db).await;

    // Play changes state: coins on a floor, a shop's stock, a monster alive, an item active.
    let key = RoomKey::new(1, 2140);
    let mut state = db.room_state().get(key).await.unwrap().unwrap();
    state.coins.copper = 999;
    db.room_state().put(&state).await.unwrap();
    let shop = db.shops().all().await.unwrap()[0].number;
    let mut stock = db.shop_stock().get(shop).await.unwrap().unwrap();
    stock.qty[0] = 77;
    db.shop_stock().put(&stock).await.unwrap();
    let item = db.items().all().await.unwrap()[0].number;
    db.items().set_active(item, 5).await.unwrap();

    import(db, |o| o.content_only = true).await;
    let second = content_checksums(db).await;
    import(db, |o| o.content_only = true).await;
    let third = content_checksums(db).await;
    assert_eq!(first, second, "content changed on the first --content-only");
    assert_eq!(
        second, third,
        "content changed on the second --content-only"
    );
    assert_eq!(
        db.room_state()
            .get(key)
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        999
    );
    assert_eq!(db.shop_stock().get(shop).await.unwrap().unwrap().qty[0], 77);
    assert_eq!(db.items().get(item).await.unwrap().unwrap().active, 5);

    // A plain re-import keeps existing state too; --reset-state rewrites it.
    import(db, |_| {}).await;
    assert_eq!(
        db.room_state()
            .get(key)
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        999
    );
    import(db, |o| o.reset_state = true).await;
    assert_eq!(
        db.room_state()
            .get(key)
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        0
    );
    assert_ne!(db.shop_stock().get(shop).await.unwrap().unwrap().qty[0], 77);
    assert_eq!(db.items().get(item).await.unwrap().unwrap().active, 0);
    assert_eq!(content_checksums(db).await, first);
    tdb.close().await;
}

#[tokio::test]
async fn dry_run_counts_every_file_and_writes_nothing() {
    let opts = ImportOptions {
        dry_run: true,
        ..ImportOptions::new(stock())
    };
    let report = Importer::new(opts, None).run().await.unwrap();
    let got: Vec<(&str, u32)> = report
        .counts
        .iter()
        .map(|c| (c.file.as_str(), c.read))
        .collect();
    assert_eq!(
        got,
        [
            ("WCCMP001.VIR", 26_720),
            ("NEWMP001.VIR", 0),
            ("WCCITEMS.VIR", 1_950),
            ("WCCKNMSR.VIR", 1_101),
            ("WCCACMSR.VIR", 0),
            ("WCCSPELS.VIR", 1_379),
            ("WCCCLASS.VIR", 15),
            ("WCCRACE.VIR", 13),
            ("WCCSHOPS.VIR", 178),
            ("WCCMSG.VIR", 3_867),
            ("WCCTEXT.VIR", 3_467),
            ("WCCACTS.VIR", 64),
            ("WCCGANGS.VIR", 0),
            ("WCCBANKS.VIR", 0),
            ("WCCITOWN.VIR", 0),
            ("WCCUSERS.VIR", 0),
        ]
    );
    assert!(report.counts.iter().all(|c| c.skipped == 0));
}

/// A WCCBANKS record for `userid` at bank 5 with balance 100.
fn bank_book(userid: &str) -> Decoded<decode::BankbookRecord> {
    let layout = decode::BankbookRecord::LAYOUT;
    let mut raw = vec![0u8; layout.len];
    let at = |f| layout.field(f).unwrap().offset;
    raw[at("userid")..at("userid") + userid.len()].copy_from_slice(userid.as_bytes());
    raw[at("bank")..at("bank") + 4].copy_from_slice(&5i32.to_le_bytes());
    raw[at("balance")..at("balance") + 4].copy_from_slice(&100u32.to_le_bytes());
    Decoded {
        file: "WCCBANKS.VIR".to_string(),
        ordinal: 1,
        record: decode::BankbookRecord::decode(&raw).unwrap(),
    }
}

// Card "Counts": any skip is an error unless --allow-duplicates; a bank book whose
// userid has no account is a skip, found before anything is written.
#[tokio::test]
async fn a_bank_book_without_an_account_fails_the_import_before_writing() {
    let tdb = TestDb::from_env().await.unwrap();
    let db: &Db = &tdb;
    let content = Content {
        bank_books: vec![bank_book("NOBODY")],
        messages: read_content(&ImportOptions::new(stock())).unwrap().messages,
        ..Content::default()
    };
    let opts = ImportOptions {
        with_users: true,
        ..ImportOptions::new(stock())
    };
    let err = Importer::new(opts.clone(), Some(db))
        .run_content(&content, Instant::now())
        .await
        .unwrap_err();
    assert!(matches!(err, ImportError::Skipped(1)), "{err}");
    assert!(
        db.messages().all().await.unwrap().is_empty(),
        "wrote before failing"
    );

    // --allow-duplicates writes the rest and counts the book as skipped.
    let report = Importer::new(
        ImportOptions {
            allow_duplicates: true,
            ..opts.clone()
        },
        Some(db),
    )
    .run_content(&content, Instant::now())
    .await
    .unwrap();
    let c = report.count("WCCBANKS.VIR").unwrap();
    assert_eq!((c.read, c.written, c.skipped), (1, 0, 1));
    assert_eq!(c.reason.as_deref(), Some("no account for the userid"));
    assert_eq!(db.messages().all().await.unwrap().len(), 3_867);

    // With the account in place the book is written under it.
    let account = db
        .accounts()
        .insert(&Account::new("nobody", "", jiff::Timestamp::UNIX_EPOCH))
        .await
        .unwrap();
    let report = Importer::new(opts, Some(db))
        .run_content(&content, Instant::now())
        .await
        .unwrap();
    assert_eq!(report.count("WCCBANKS.VIR").unwrap().written, 1);
    let books = db.bank_books().for_account(account).await.unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!((books[0].bank.0, books[0].balance), (5, 100));
    tdb.close().await;
}
