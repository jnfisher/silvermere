//! `--live`: a running realm's `WCC*.DAT` set into the schema (T-107).
//!
//! The live files are built at test time by `support/btrieve_v5.rs` from the stock `.VIR`
//! files of the stock distribution (the testkit's `stock` helper; their FCR, key and ACS pages as Btrieve
//! wrote them, plus data pages holding the realm's records). The realm: one player (Mira,
//! of the gang Wolves, with a bank book and a limited item), the first three rooms of
//! `WCCMP001` with coins, an item and an owner on the first, a bank book whose player is
//! gone, and one vacation record. Every other file has no `.DAT`, so its `.VIR` is read,
//! as the host's `opnbtv` creates a missing `.DAT` from it.

use std::path::{Path, PathBuf};

use silvermere_db::{Db, TestDb};
use silvermere_import::btrieve::{BtrieveError, BtrieveFile};
use silvermere_import::decode::{
    BankbookRecord, GangRecord, ItemownRecord, PlayerRecord, Record, RoomRecord, Text,
};
use silvermere_import::{ImportError, ImportOptions, Importer};
use silvermere_types::ids::RoomKey;

use crate::support::btrieve_v5::{with_record_length, write_v5};

#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock_data;

fn stock() -> PathBuf {
    stock_data::vir_dir()
}

fn template(name: &str) -> Vec<u8> {
    std::fs::read(stock().join(name)).unwrap()
}

/// A fresh directory for one test.
fn dir(name: &str) -> PathBuf {
    let d =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Links every stock `.VIR` into `d` (the realm's virgin files); copies them on Windows, where a
/// symlink needs a privilege (T-109).
fn link_stock(d: &Path) {
    for e in std::fs::read_dir(stock()).unwrap().flatten() {
        let name = e.file_name();
        if name.to_string_lossy().ends_with(".VIR") {
            #[cfg(unix)]
            std::os::unix::fs::symlink(e.path(), d.join(&name)).unwrap();
            #[cfg(not(unix))]
            std::fs::copy(e.path(), d.join(&name)).unwrap();
        }
    }
}

fn player() -> PlayerRecord {
    let mut p = PlayerRecord::decode(&[0; 1998]).unwrap();
    p.userid = Text("Mira".into());
    p.firstname = Text("Mira".into());
    p.lastname = Text("Vale".into());
    p.race = 1;
    p.class = 1;
    p.level = 5;
    p.exp = 12_345;
    p.maxhp = 40;
    p.hp = 33;
    p.map = 1;
    p.room = 2;
    p.gold = 42;
    p.gang = Text("Wolves".into());
    p
}

fn gang() -> GangRecord {
    let mut g = GangRecord::decode(&[0; 253]).unwrap();
    g.key = Text("Wolves".into());
    g.name = Text("Wolves".into());
    g.leader = Text("Mira".into());
    g.exp = 500;
    g.members = 1;
    g
}

fn book(userid: &str, balance: u32) -> BankbookRecord {
    let mut b = BankbookRecord::decode(&[0; 72]).unwrap();
    b.userid = Text(userid.into());
    b.bank = 1;
    b.balance = balance;
    b
}

fn owned() -> ItemownRecord {
    let mut o = ItemownRecord::decode(&[0; 65]).unwrap();
    o.id = 1;
    o.itemnum = 7;
    o.owntype = 1;
    o.userid = Text("Mira".into());
    o
}

/// The first `n` records of the stock map, raw.
fn stock_rooms(n: usize) -> Vec<Vec<u8>> {
    let mut f = BtrieveFile::open(&stock().join("WCCMP001.VIR")).unwrap();
    let mut out = Vec::new();
    let mut records = f.records();
    while out.len() < n {
        out.push(records.next_record().unwrap().unwrap().data.to_vec());
    }
    out
}

/// The realm described at the top.
fn realm(name: &str) -> PathBuf {
    let d = dir(name);
    link_stock(&d);
    let mut rooms = stock_rooms(3);
    let mut first = RoomRecord::decode(&rooms[0]).unwrap();
    first.gold = 99;
    first.items[0] = 7;
    first.itemuses[0] = 3;
    first.itemcounts[0] = 1;
    first.owner = Text("Mira".into());
    rooms[0] = first.encode();
    let write = |file: &str, tpl: Vec<u8>, records: Vec<Vec<u8>>| {
        std::fs::write(d.join(file), write_v5(&tpl, &records)).unwrap();
    };
    write("WCCMP001.DAT", template_of_map(), rooms);
    write(
        "WCCUSERS.DAT",
        template("WCCUSERS.VIR"),
        vec![player().encode()],
    );
    write(
        "WCCGANGS.DAT",
        template("WCCGANGS.VIR"),
        vec![gang().encode()],
    );
    write(
        "WCCBANKS.DAT",
        template("WCCBANKS.VIR"),
        vec![book("Mira", 1_000).encode(), book("Ghost", 5).encode()],
    );
    write(
        "WCCITOWN.DAT",
        template("WCCITOWN.VIR"),
        vec![owned().encode()],
    );
    // the vacation file: no stock template, so WCCUSERS' pages with a 1740-byte record
    let mut vacation = vec![0u8; 1740];
    vacation[..4].copy_from_slice(b"Mira");
    vacation[100] = 1;
    write(
        "WCCVACN.DAT",
        with_record_length(&template("WCCUSERS.VIR"), 1740),
        vec![vacation],
    );
    d
}

/// The stock map's FCR and key pages with no records: WCCMP001.VIR's first pages, the
/// record count set to 0 (its data pages are dropped).
fn template_of_map() -> Vec<u8> {
    let vir = template("WCCMP001.VIR");
    let page = usize::from(u16::from_le_bytes([vir[8], vir[9]]));
    // FCR, then the pages before the first data page (bit 0x80 of byte 5)
    let first_data = (1..vir.len() / page)
        .find(|p| vir[p * page + 5] & 0x80 != 0)
        .unwrap();
    let mut t = vir[..first_data * page].to_vec();
    t[0x10..0x14].copy_from_slice(&[0xFF; 4]); // no deleted records
    t
}

fn live(d: &Path) -> ImportOptions {
    ImportOptions {
        no_patches: true,
        ..ImportOptions::live(d)
    }
}

#[tokio::test]
async fn live_dat_hand_set_fields_round_trip() {
    let d = realm("round-trip");
    let tdb = TestDb::from_env().await.unwrap();
    let db: &Db = &tdb;

    // the bank book of a player who is gone is skipped and reported, not an error (John)
    let report = Importer::new(live(&d), Some(db)).run().await.unwrap();
    println!("{report}");
    // the .DAT where there is one, the .VIR elsewhere; NEWMP001 is not read
    for (file, read) in [
        ("WCCMP001.DAT", 3),
        ("WCCUSERS.DAT", 1),
        ("WCCGANGS.DAT", 1),
        ("WCCBANKS.DAT", 2),
        ("WCCITOWN.DAT", 1),
        ("WCCITEMS.VIR", 1950),
    ] {
        assert_eq!(report.count(file).map(|c| c.read), Some(read), "{file}");
    }
    assert!(report.count("NEWMP001.VIR").is_none());
    assert!(report.count("WCCMP001.VIR").is_none());
    let lines: Vec<String> = report.uncarried.iter().map(|n| n.to_string()).collect();
    assert_eq!(
        lines,
        [
            "not carried [WCCVACN.DAT] record 1: silvermere has no vacation table (the feature \
             is a stub)",
            "not carried [WCCBANKS.DAT] record 2 (userid Ghost, bank 1): its owner Ghost has \
             no account and no character",
        ]
    );

    let account = db.accounts().by_username("Mira").await.unwrap().unwrap().id;
    let c = db
        .characters()
        .by_account(account)
        .await
        .unwrap()
        .unwrap()
        .character;
    assert_eq!(
        (c.firstname.as_str(), c.lastname.as_str()),
        ("Mira", "Vale")
    );
    assert_eq!((c.level, c.exp, c.hp, c.maxhp), (5, 12_345, 33, 40));
    assert_eq!((c.map.0, c.room.0, c.gang.as_str()), (1, 2, "Wolves"));
    assert_eq!(c.coins.gold, 42);

    let gangs = db.gangs().all().await.unwrap();
    assert_eq!(gangs.len(), 1);
    assert_eq!(
        (gangs[0].name.as_str(), gangs[0].leader.as_str()),
        ("Wolves", "Mira")
    );
    assert_eq!((gangs[0].exp, gangs[0].members), (500, 1));

    let books = db.bank_books().all().await.unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!((books[0].0, books[0].1.balance), (account, 1_000));

    let owns = db.item_ownership().all().await.unwrap();
    assert_eq!(owns.len(), 1);
    assert_eq!((owns[0].itemnum.0, owns[0].userid.as_str()), (7, "Mira"));

    let first = RoomRecord::decode(&stock_rooms(1)[0]).unwrap();
    let state = db
        .room_state()
        .get(RoomKey::new(first.mapnum, first.roomnum))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.coins.gold, 99);
    assert_eq!(state.owner, "Mira");
    assert_eq!(
        (
            state.items[0].item.0,
            state.items[0].uses,
            state.items[0].count
        ),
        (7, 3, 1)
    );
}

/// A dry run, with no database, still reports the orphaned bank book.
#[tokio::test]
async fn dry_run_reports_the_orphaned_book() {
    let d = realm("dry");
    let opts = ImportOptions {
        dry_run: true,
        ..live(&d)
    };
    let report = Importer::new(opts, None).run().await.unwrap();
    let c = report.count("WCCBANKS.DAT").unwrap();
    assert_eq!(
        (c.read, c.written, c.skipped, c.skip_is_error),
        (2, 1, 1, false)
    );
    assert!(
        report
            .uncarried
            .iter()
            .any(|n| n.file == "WCCBANKS.DAT" && n.key == "userid Ghost, bank 1"),
        "{report}"
    );
}

/// Without `--live` the same directory imports the virgin set: the `.DAT`s are not read.
#[tokio::test]
async fn without_live_the_vir_set() {
    let d = realm("virgin");
    let opts = ImportOptions {
        dry_run: true,
        no_patches: true,
        ..ImportOptions::new(&d)
    };
    let report = Importer::new(opts, None).run().await.unwrap();
    assert_eq!(report.count("WCCMP001.VIR").unwrap().read, 26_720);
    assert_eq!(report.count("WCCUSERS.VIR").unwrap().read, 0);
    assert!(report.count("WCCUSERS.DAT").is_none());
    assert!(report.uncarried.is_empty());
}

async fn dry_run_error(d: &Path) -> ImportError {
    let opts = ImportOptions {
        dry_run: true,
        ..live(d)
    };
    Importer::new(opts, None).run().await.unwrap_err()
}

/// A pre-image file beside a data file: uncommitted changes, refused.
#[tokio::test]
async fn pre_image_refused() {
    let d = realm("pre");
    std::fs::write(d.join("wccusers.pre"), b"").unwrap();
    let err = dry_run_error(&d).await;
    assert!(matches!(err, ImportError::PreImage { .. }), "{err}");
    let msg = err.to_string();
    assert!(
        msg.contains("WCCUSERS.DAT") && msg.contains("wccusers.pre"),
        "{msg}"
    );
    assert!(msg.contains("uncommitted changes"), "{msg}");
}

/// An extension file, or a file shorter than its header says: refused.
#[tokio::test]
async fn extended_and_truncated_refused() {
    let d = realm("ext");
    std::fs::write(d.join("WCCGANGS.^01"), b"").unwrap();
    let err = dry_run_error(&d).await;
    assert!(matches!(err, ImportError::Extended { .. }), "{err}");
    assert!(
        err.to_string().contains("continues in WCCGANGS.^01"),
        "{err}"
    );

    let d = realm("short");
    let path = d.join("WCCUSERS.DAT");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() - 2048]).unwrap();
    let err = dry_run_error(&d).await;
    assert!(matches!(err, ImportError::Extended { .. }), "{err}");
    assert!(
        err.to_string()
            .contains("the header counts 6 pages of 2048 bytes, the file holds 5"),
        "{err}"
    );
}

/// Malformed files: not Btrieve, marked for recovery, or records of another length.
#[tokio::test]
async fn malformed_dat_refused() {
    let d = realm("garbage");
    std::fs::write(d.join("WCCGANGS.DAT"), vec![0x55; 4096]).unwrap();
    let err = dry_run_error(&d).await;
    assert!(
        matches!(err, ImportError::Btrieve(BtrieveError::NotBtrieve { .. })),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains("WCCGANGS.DAT: not a Btrieve 5.x/6.x file"),
        "{err}"
    );

    let d = realm("recovery");
    let path = d.join("WCCBANKS.DAT");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0x22..0x24].copy_from_slice(&[0xFF, 0xFF]);
    std::fs::write(&path, bytes).unwrap();
    let err = dry_run_error(&d).await;
    assert!(
        matches!(
            err,
            ImportError::Btrieve(BtrieveError::NeedsRecovery { .. })
        ),
        "{err}"
    );

    let d = realm("length");
    std::fs::write(
        d.join("WCCGANGS.DAT"),
        write_v5(&template("WCCUSERS.VIR"), &[player().encode()]),
    )
    .unwrap();
    let err = dry_run_error(&d).await;
    assert!(
        matches!(
            err,
            ImportError::Btrieve(BtrieveError::RecordLengthMismatch { .. })
        ),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains("record length is 1998, expected 253"),
        "{err}"
    );
}
