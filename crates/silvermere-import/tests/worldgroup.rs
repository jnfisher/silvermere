//! A Worldgroup 3.x realm (T-135): the `*2` file names and the 4-byte-aligned record layouts.
//!
//! The backbone: every record of every stock `.VIR` file, re-packed member by member from its
//! byte-packed layout into the aligned one, decodes to the same values. Then the state records
//! (player, gang, bank book, item ownership, active monster) put their known values at the
//! aligned offsets, the set is told apart by its names, and a whole live realm written in the
//! aligned layout under the `*2` names (built from the DOS live realm of `live_dat.rs`, plus
//! active monsters and the stock patch file re-packed) imports to the same rows, table for
//! table, as the realm it was built from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use silvermere_db::{Db, TestDb};
use silvermere_import::btrieve::OpenOptions;
use silvermere_import::decode::{
    ActmonRecord, BankbookRecord, GangRecord, ItemownRecord, PlayerRecord, Record, Text,
};
use silvermere_import::importer::{
    ImportError, ImportOptions, Importer, TABLE_FILES, read_content, set_file_name,
};
use silvermere_types::records::layout;
use silvermere_types::records::{Layout, LayoutSet};
use sqlx::Row;

use crate::live_dat::{dir, realm, stock, template};
use crate::support::btrieve_v5::{template_for, write_v5, write_v5_variable};

const WG: LayoutSet = LayoutSet::Worldgroup;

/// `raw` (a record of `dos`) with every member moved to its offset in `wg`, padding zero.
fn repack(raw: &[u8], dos: &Layout, wg: &Layout) -> Vec<u8> {
    assert_eq!(raw.len(), dos.len, "struct {}", dos.decoder);
    let mut out = vec![0; wg.len];
    for f in dos.fields {
        let w = wg.field(f.name).unwrap();
        let n = f.size.min(w.size);
        out[w.offset..w.offset + n].copy_from_slice(&raw[f.offset..f.offset + n]);
    }
    out
}

/// The byte-packed layout of the records in the file named `vir` (a DOS name).
fn dos_layout(vir: &str) -> &'static Layout {
    match vir.split('.').next().unwrap() {
        "WCCMP001" => &layout::ROOM,
        "WCCITEMS" => &layout::ITEM,
        "WCCKNMSR" => &layout::MONSTER,
        "WCCACMSR" => &layout::ACTMON,
        "WCCSPELS" => &layout::SPELL,
        "WCCCLASS" => &layout::CLASS,
        "WCCRACE" => &layout::RACE,
        "WCCSHOPS" => &layout::SHOP,
        "WCCMSG" => &layout::MUDMSG,
        "WCCTEXT" => &layout::TEXTBLK,
        "WCCACTS" => &layout::ACTION,
        "WCCGANGS" => &layout::GANG,
        "WCCBANKS" => &layout::BANKBOOK,
        "WCCITOWN" => &layout::ITEMOWN,
        "WCCUSERS" => &layout::PLAYER,
        other => panic!("no layout for {other}"),
    }
}

/// Every record of `path`, raw (a variable-length file's fixed part plus its tail).
fn raw_records(path: &Path) -> Vec<Vec<u8>> {
    let mut f = OpenOptions::new().variable_length(true).open(path).unwrap();
    let mut out = Vec::new();
    let mut records = f.records();
    while let Some(r) = records.next_record() {
        out.push(r.unwrap().data.to_vec());
    }
    out
}

/// A text block record re-packed: zero-filled to the 2022-byte buffer, re-packed, and cut to
/// the aligned fixed part plus as many text bytes as it held.
fn repack_text(raw: &[u8]) -> Vec<u8> {
    let mut full = raw.to_vec();
    full.resize(layout::TEXTBLK.len, 0);
    let mut out = repack(&full, &layout::TEXTBLK, &layout::WG_TEXTBLK);
    out.truncate(raw.len() + 2);
    out
}

/// Decodes `raw` (a record of `vir`) in the DOS layout and its re-pack in the aligned one, and
/// compares them as `--json` writes them.
fn same_values(vir: &str, raw: &[u8]) {
    use silvermere_import::decode::*;
    fn both<R: Record + std::fmt::Debug + PartialEq>(raw: &[u8]) {
        let dos = R::decode(raw).unwrap();
        let wg_raw = repack(raw, R::LAYOUT, R::layout(WG));
        let wg = R::decode_as(&wg_raw, WG).unwrap();
        assert_eq!(dos, wg, "struct {}", R::LAYOUT.decoder);
        assert_eq!(R::decode_as(&wg.encode_as(WG), WG).unwrap(), wg);
    }
    match dos_layout(vir).decoder {
        "room" => both::<RoomRecord>(raw),
        "item" => both::<ItemRecord>(raw),
        "monster" => both::<MonsterRecord>(raw),
        "spell" => both::<SpellRecord>(raw),
        "class" => both::<ClassRecord>(raw),
        "race" => both::<RaceRecord>(raw),
        "shop" => both::<ShopRecord>(raw),
        "mudmsg" => both::<MudmsgRecord>(raw),
        "action" => both::<ActionRecord>(raw),
        "textblk" => {
            let dos = decode_textblk(raw).unwrap();
            let wg = decode_textblk_as(&repack_text(raw), WG).unwrap();
            assert_eq!(dos, wg, "struct textblk");
        }
        other => panic!("{other}"),
    }
}

// The backbone: every stock content record, re-packed into the aligned layout, decodes equal.
#[test]
fn every_stock_record_repacks_to_the_same_values() {
    let files = [
        "WCCMP001.VIR",
        "WCCITEMS.VIR",
        "WCCKNMSR.VIR",
        "WCCSPELS.VIR",
        "WCCCLASS.VIR",
        "WCCRACE.VIR",
        "WCCSHOPS.VIR",
        "WCCMSG.VIR",
        "WCCTEXT.VIR",
        "WCCACTS.VIR",
    ];
    for vir in files {
        let raws = raw_records(&stock().join(vir));
        assert!(!raws.is_empty(), "{vir}");
        for raw in &raws {
            same_values(vir, raw);
        }
    }
}

/// `n` bytes at `at` as a little-endian integer.
fn int(b: &[u8], at: usize, n: usize) -> u64 {
    b[at..at + n]
        .iter()
        .rev()
        .fold(0, |v, &x| v << 8 | u64::from(x))
}

fn text(b: &[u8], at: usize) -> String {
    let end = b[at..].iter().position(|&c| c == 0).unwrap();
    String::from_utf8(b[at..at + end].to_vec()).unwrap()
}

// The state records: known values at the aligned offsets (the offsets written out, not looked
// up), and back.
#[test]
fn state_records_put_known_values_at_the_aligned_offsets() {
    let mut p = PlayerRecord::decode_as(&[0; 2028], WG).unwrap();
    p.userid = Text("Mira".into());
    p.level = 5;
    p.hp = 33;
    p.items[0] = 7;
    p.keys[1] = 9;
    p.qflags2 = 0x0102_0304;
    p.gold = 42;
    p.gang = Text("Wolves".into());
    p.medtick = 3;
    let b = p.encode_as(WG);
    assert_eq!(b.len(), 2028);
    assert_eq!(text(&b, 0x000), "Mira");
    assert_eq!(int(&b, 0x094, 2), 5);
    assert_eq!(int(&b, 0x0B0, 2), 33);
    assert_eq!(int(&b, 0x0D8, 4), 7);
    assert_eq!(int(&b, 0x334 + 4, 4), 9);
    assert_eq!(int(&b, 0x460, 4), 0x0102_0304);
    assert_eq!(int(&b, 0x618, 4), 42);
    assert_eq!(text(&b, 0x6C8), "Wolves");
    assert_eq!(b[0x7EB], 3);
    assert_eq!(PlayerRecord::decode_as(&b, WG).unwrap(), p);

    let mut a = ActmonRecord::decode_as(&[0; 400], WG).unwrap();
    a.number = 11;
    a.exp = 1234;
    a.friendly = 1;
    a.genroom = 77;
    a.flags = 0x8001;
    a.lastupd = 99;
    a.montype = 7;
    a.hpregen = 4;
    a.created = 0x1234;
    let b = a.encode_as(WG);
    assert_eq!(int(&b, 0x000, 4), 11);
    assert_eq!(int(&b, 0x110, 4), 1234);
    assert_eq!(b[0x116], 1);
    assert_eq!(int(&b, 0x120, 4), 77);
    assert_eq!(int(&b, 0x128, 2), 0x8001);
    assert_eq!(int(&b, 0x12C, 2), 7);
    assert_eq!(int(&b, 0x130, 2), 4);
    assert_eq!(int(&b, 0x144, 4), 99);
    assert_eq!(int(&b, 0x16A, 2), 0x1234);
    assert_eq!(&b[0x125..0x128], &[0; 3]);
    assert_eq!(ActmonRecord::decode_as(&b, WG).unwrap(), a);

    let mut g = GangRecord::decode_as(&[0; 256], WG).unwrap();
    g.exp = 500;
    g.leader = Text("Mira".into());
    g.overexp = 6;
    g.overflows = 2;
    let b = g.encode_as(WG);
    assert_eq!(int(&b, 0x028, 4), 500);
    assert_eq!(text(&b, 0x02C), "Mira");
    assert_eq!(int(&b, 0x058, 4), 6);
    assert_eq!(int(&b, 0x05C, 2), 2);
    assert_eq!(GangRecord::decode_as(&b, WG).unwrap(), g);

    let mut k = BankbookRecord::decode_as(&[0; 76], WG).unwrap();
    k.bank = 1;
    k.balance = 1000;
    k.loan = 50;
    let b = k.encode_as(WG);
    assert_eq!(
        (int(&b, 0x020, 4), int(&b, 0x024, 4), int(&b, 0x028, 4)),
        (1, 1000, 50)
    );
    assert_eq!(BankbookRecord::decode_as(&b, WG).unwrap(), k);

    let mut o = ItemownRecord::decode_as(&[0; 72], WG).unwrap();
    o.mapnum = 1;
    o.roomnum = 2;
    o.userid = Text("Mira".into());
    o.monnum = 4;
    let b = o.encode_as(WG);
    assert_eq!((int(&b, 0x00C, 4), int(&b, 0x010, 4)), (1, 2));
    assert_eq!(text(&b, 0x014), "Mira");
    assert_eq!(int(&b, 0x034, 4), 4);
    assert_eq!(ItemownRecord::decode_as(&b, WG).unwrap(), o);
}

#[test]
fn file_names_of_the_set() {
    assert_eq!(set_file_name("WCCITEMS.VIR", WG), "WCCITEM2.VIR");
    assert_eq!(set_file_name("WCCMP001.VIR", WG), "WCCMP002.VIR");
    assert_eq!(set_file_name("wccupdat.dat", WG), "WCCUPDA2.dat");
    assert_eq!(set_file_name("NEWMP001.VIR", WG), "NEWMP001.VIR");
    assert_eq!(
        set_file_name("WCCITEMS.VIR", LayoutSet::Dos),
        "WCCITEMS.VIR"
    );
}

// John, 2026-10-21: a complete Worldgroup set wins; the DOS-named strays are reported.
#[tokio::test]
async fn dos_named_files_beside_a_worldgroup_set_are_reported_not_read() {
    let dos = dos_realm("wg-strays-dos");
    let wg = worldgroup_realm(&dos, "wg-strays-wg");
    std::fs::copy(stock().join("WCCMSG.VIR"), wg.join("WCCMSG.VIR")).unwrap();
    std::fs::write(wg.join("wccusers.dat"), b"not a Btrieve file").unwrap();
    let content = read_content(&ImportOptions::live(&wg)).unwrap();
    assert_eq!(content.set, WG);
    assert_eq!(content.ignored, ["WCCMSG.VIR", "wccusers.dat"]);
    let opts = ImportOptions {
        dry_run: true,
        ..ImportOptions::live(&wg)
    };
    let report = Importer::new(opts, None).run().await.unwrap();
    assert!(report.count("WCCMSG2.VIR").is_some());
    assert!(report.count("WCCMSG.VIR").is_none());
    let text = report.to_string();
    assert!(
        text.contains(
            "not read [WCCMSG.VIR]: a DOS 1.11p data file beside the Worldgroup 3.x set\n\
             not read [wccusers.dat]: a DOS 1.11p data file beside the Worldgroup 3.x set"
        ),
        "{text}"
    );
}

/// The `not read` line of `file` beside a set of `set`.
fn stray_line(file: &str, set: LayoutSet) -> String {
    format!(
        "not read [{file}]: {}",
        silvermere_import::importer::stray_reason(set)
    )
}

async fn dry_run(d: &Path) -> silvermere_import::Report {
    let opts = ImportOptions {
        dry_run: true,
        ..ImportOptions::live(d)
    };
    Importer::new(opts, None).run().await.unwrap()
}

// T-136: a complete DOS set wins over a `*2` vacation file, which never decides the set.
#[tokio::test]
async fn a_worldgroup_vacation_file_beside_a_dos_set_is_reported_not_read() {
    let d = dos_realm("dos-stray-vacn");
    std::fs::write(d.join("WCCVACN2.DAT"), b"").unwrap();
    let content = read_content(&ImportOptions::live(&d)).unwrap();
    assert_eq!(content.set, LayoutSet::Dos);
    assert_eq!(content.ignored, ["WCCVACN2.DAT"]);
    let report = dry_run(&d).await;
    assert!(report.count("WCCUSERS.DAT").is_some());
    let text = report.to_string();
    assert!(
        text.contains(&stray_line("WCCVACN2.DAT", LayoutSet::Dos)),
        "{text}"
    );
    assert!(
        text.contains("a Worldgroup 3.x data file beside the DOS 1.11p set"),
        "{text}"
    );
}

// T-136: a complete DOS set wins over a stray `*2` table file too.
#[tokio::test]
async fn a_worldgroup_table_file_beside_a_dos_set_is_reported_not_read() {
    let d = dos_realm("dos-stray-item2");
    std::fs::write(d.join("wccitem2.vir"), b"not a Btrieve file").unwrap();
    let content = read_content(&ImportOptions::live(&d)).unwrap();
    assert_eq!(content.set, LayoutSet::Dos);
    assert_eq!(content.ignored, ["wccitem2.vir"]);
    let report = dry_run(&d).await;
    assert_eq!(report.count("WCCITEMS.VIR").map(|c| c.read), Some(1950));
    assert!(
        report
            .to_string()
            .contains(&stray_line("wccitem2.vir", LayoutSet::Dos))
    );
}

// T-136: the DOS patch file beside a complete Worldgroup set is a stray; WCCUPDA2 is applied.
#[tokio::test]
async fn a_dos_patch_file_beside_a_worldgroup_set_is_reported_not_read() {
    let dos = dos_realm("wg-stray-updat-dos");
    let wg = worldgroup_realm(&dos, "wg-stray-updat-wg");
    std::fs::copy(dos.join("WCCUPDAT.DAT"), wg.join("WCCUPDAT.DAT")).unwrap();
    let content = read_content(&ImportOptions::live(&wg)).unwrap();
    assert_eq!(content.set, WG);
    assert_eq!(content.ignored, ["WCCUPDAT.DAT"]);
    let report = dry_run(&wg).await;
    assert_eq!(report.patches.as_ref().unwrap().file, "WCCUPDA2.DAT");
    assert!(report.to_string().contains(&stray_line("WCCUPDAT.DAT", WG)));
}

// T-136: a complete set of each build is still an error naming both.
#[test]
fn two_complete_sets_are_an_error_naming_both() {
    let dos = dos_realm("both-complete-dos");
    let wg = worldgroup_realm(&dos, "both-complete-wg");
    for e in std::fs::read_dir(&wg).unwrap().flatten() {
        std::fs::copy(e.path(), dos.join(e.file_name())).unwrap();
    }
    let e = read_content(&ImportOptions::live(&dos)).unwrap_err();
    let msg = e.to_string();
    assert!(matches!(e, ImportError::MixedSets { .. }), "{msg}");
    assert!(
        msg.contains("WCCUSERS.DAT") && msg.contains("WCCUSER2.DAT"),
        "{msg}"
    );
}

#[test]
fn a_partial_set_is_an_error_naming_what_was_found() {
    let d = dir("wg-partial");
    for f in [
        "WCCUSER2.DAT",
        "WCCMP002.DAT",
        "wccupda2.dat",
        "WCCITEMS.VIR",
    ] {
        std::fs::write(d.join(f), b"").unwrap();
    }
    let e = read_content(&ImportOptions::live(&d)).unwrap_err();
    let msg = e.to_string();
    assert!(matches!(e, ImportError::PartialSet { .. }), "{msg}");
    assert!(
        msg.contains("WCCMP002.DAT, WCCUSER2.DAT, wccupda2.dat"),
        "{msg}"
    );
    assert!(
        msg.contains("WCCITEM2") && !msg.contains("WCCUSER2,"),
        "{msg}"
    );
}

/// Two active monsters of the first stock monster, in the realm's first room.
fn active_monsters() -> Vec<Vec<u8>> {
    let monster = raw_records(&stock().join("WCCKNMSR.VIR")).remove(0);
    let m = silvermere_import::decode::MonsterRecord::decode(&monster).unwrap();
    let room = raw_records(&stock().join("WCCMP001.VIR")).remove(0);
    let r = silvermere_import::decode::RoomRecord::decode(&room).unwrap();
    (1..=2)
        .map(|n| {
            let mut a = ActmonRecord::decode(&[0; 386]).unwrap();
            a.number = n;
            a.knownnum = m.number;
            a.level = m.level;
            a.map = r.mapnum as i32;
            a.room = r.roomnum as i32;
            a.hp = 10 + n as i16;
            a.name = m.name.clone();
            a.maxhp = m.maxhp;
            a.exp = m.exp;
            a.genroom = r.roomnum as i32;
            a.flags = 0x0004;
            a.target = -1;
            a.lastdir = -1;
            a.spells[0] = 3;
            a.spelldurs[0] = 5;
            a.unk_0x15E.0[0] = 0x5A;
            a.encode()
        })
        .collect()
}

/// The DOS live realm of `live_dat.rs` plus active monsters and the stock patch file.
fn dos_realm(name: &str) -> PathBuf {
    let d = realm(name);
    std::fs::write(
        d.join("WCCACMSR.DAT"),
        write_v5(&template("WCCACMSR.VIR"), &active_monsters()),
    )
    .unwrap();
    std::fs::copy(crate::stock::file("WCCUPDAT.DAT"), d.join("WCCUPDAT.DAT")).unwrap();
    d
}

/// The DOS patch file's records re-packed: `seqnum`, `type`, 3 bytes of padding, and the
/// image re-packed into its aligned record, cut to cover what the DOS record carried.
fn repack_patches(dos: &Path) -> Vec<Vec<u8>> {
    let kinds: [&Layout; 10] = [
        &layout::RACE,
        &layout::CLASS,
        &layout::ROOM,
        &layout::SHOP,
        &layout::ITEM,
        &layout::SPELL,
        &layout::MONSTER,
        &layout::MUDMSG,
        &layout::TEXTBLK,
        &layout::ACTION,
    ];
    raw_records(dos)
        .iter()
        .map(|raw| {
            assert_eq!(raw.len(), 2015, "the stock record");
            let kind = kinds[usize::from(raw[4]) - 1];
            let mut image = raw[5..].to_vec();
            image.resize(kind.len.max(image.len()), 0);
            let mut out = raw[..5].to_vec();
            out.extend_from_slice(&[0; 3]);
            out.extend_from_slice(&repack(&image[..kind.len], kind, WG.layout(kind)));
            // the 2010 bytes carried cover the text block's text[..1988]: 2012 aligned
            out.resize(8 + 2012, 0);
            out
        })
        .collect()
}

/// The realm at `dos` in the Worldgroup 3.x set: each file the DOS live import reads
/// re-packed under its `*2` name, with the same extension (the `.DAT` where the realm has
/// one, the `.VIR` elsewhere), plus the vacation and patch files.
fn worldgroup_realm(dos: &Path, name: &str) -> PathBuf {
    let d = dir(name);
    let read = read_content(&ImportOptions::live(dos)).unwrap();
    for vir in std::iter::once("WCCMP001.VIR").chain(TABLE_FILES) {
        let file = read.file(vir);
        let ext = file.split('.').nth(1).unwrap();
        let wg_name = set_file_name(&format!("{}.{ext}", vir.split('.').next().unwrap()), WG);
        let dos_layout = dos_layout(vir);
        let raws = raw_records(&dos.join(&file));
        let tpl = std::fs::read(stock().join(vir)).unwrap();
        let bytes = if dos_layout.decoder == "textblk" {
            let recs: Vec<Vec<u8>> = raws.iter().map(|r| repack_text(r)).collect();
            write_v5_variable(&template_for(&tpl, 24), &recs)
        } else {
            let wg = WG.layout(dos_layout);
            let recs: Vec<Vec<u8>> = raws.iter().map(|r| repack(r, dos_layout, wg)).collect();
            write_v5(&template_for(&tpl, wg.len), &recs)
        };
        std::fs::write(d.join(wg_name), bytes).unwrap();
    }
    std::fs::copy(dos.join("WCCVACN.DAT"), d.join("WCCVACN2.DAT")).unwrap();
    let tpl = std::fs::read(dos.join("WCCUPDAT.DAT")).unwrap();
    std::fs::write(
        d.join("wccupda2.dat"),
        write_v5(
            &template_for(&tpl, 2020),
            &repack_patches(&dos.join("WCCUPDAT.DAT")),
        ),
    )
    .unwrap();
    d
}

/// Every table's rows, each row its columns as text (the times the import itself stamps, such
/// as an account's creation, masked), sorted.
async fn dump(db: &Db) -> BTreeMap<String, Vec<String>> {
    let pool = db.pool();
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<String, _>(0))
    .collect();
    let mut out = BTreeMap::new();
    for t in tables {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM \"{t}\"")))
            .fetch_all(&pool)
            .await
            .unwrap();
        let mut lines: Vec<String> = rows
            .iter()
            .map(|r| {
                (0..r.len())
                    .map(|i| {
                        if let Ok(v) = r.try_get::<Option<i64>, _>(i) {
                            format!("{v:?}")
                        } else if let Ok(v) = r.try_get::<Option<String>, _>(i) {
                            match v {
                                Some(t) if t.parse::<jiff::Timestamp>().is_ok() => {
                                    "(a time of the import)".to_string()
                                }
                                v => format!("{v:?}"),
                            }
                        } else if let Ok(v) = r.try_get::<Option<f64>, _>(i) {
                            format!("{v:?}")
                        } else {
                            format!("{:?}", r.try_get::<Option<Vec<u8>>, _>(i).unwrap())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect();
        lines.sort();
        out.insert(t, lines);
    }
    out
}

// Done-when: a Worldgroup-layout live realm imports to the same rows as the same data in the
// DOS layout, every table, the patch file applied.
#[tokio::test]
async fn a_worldgroup_realm_imports_the_same_rows_as_its_dos_twin() {
    let dos = dos_realm("wg-twin-dos");
    let wg = worldgroup_realm(&dos, "wg-twin-wg");

    let content = read_content(&ImportOptions::live(&wg)).unwrap();
    assert_eq!(content.set, WG);
    assert_eq!(content.active_monsters.len(), 2);

    let dos_db = TestDb::sqlite_temp().await.unwrap();
    let wg_db = TestDb::sqlite_temp().await.unwrap();
    let dos_report = Importer::new(ImportOptions::live(&dos), Some(&dos_db))
        .run()
        .await
        .unwrap();
    let wg_report = Importer::new(ImportOptions::live(&wg), Some(&wg_db))
        .run()
        .await
        .unwrap();
    println!("{wg_report}");

    for (dos_file, wg_file, read) in [
        ("WCCMP001.DAT", "WCCMP002.DAT", 3),
        ("WCCUSERS.DAT", "WCCUSER2.DAT", 1),
        ("WCCGANGS.DAT", "WCCGANG2.DAT", 1),
        ("WCCBANKS.DAT", "WCCBANK2.DAT", 2),
        ("WCCITOWN.DAT", "WCCITOW2.DAT", 1),
        ("WCCACMSR.DAT", "WCCACMS2.DAT", 2),
        ("WCCITEMS.VIR", "WCCITEM2.VIR", 1950),
        ("WCCTEXT.VIR", "WCCTEXT2.VIR", 3467),
    ] {
        assert_eq!(dos_report.count(dos_file).map(|c| c.read), Some(read));
        assert_eq!(
            wg_report.count(wg_file).map(|c| c.read),
            Some(read),
            "{wg_file}"
        );
    }
    let (dp, wp) = (
        dos_report.patches.as_ref().unwrap(),
        wg_report.patches.as_ref().unwrap(),
    );
    assert_eq!(wp.file, "WCCUPDA2.DAT");
    assert_eq!(wp.total(), 38_754);
    assert_eq!(
        (
            &dp.applied_by_kind,
            &dp.rewritten_by_kind,
            &dp.inserted_by_kind
        ),
        (
            &wp.applied_by_kind,
            &wp.rewritten_by_kind,
            &wp.inserted_by_kind
        )
    );
    assert_eq!(wg_report.uncarried.len(), dos_report.uncarried.len());

    let (a, b) = (dump(&dos_db).await, dump(&wg_db).await);
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    let mut rows = 0;
    for (table, rows_dos) in &a {
        let rows_wg = &b[table];
        assert_eq!(rows_dos.len(), rows_wg.len(), "{table}: rows");
        for (x, y) in rows_dos.iter().zip(rows_wg) {
            assert_eq!(x, y, "{table}");
        }
        rows += rows_dos.len();
    }
    for t in [
        "active_monsters",
        "characters",
        "gangs",
        "bank_books",
        "item_ownership",
    ] {
        assert!(!a[t].is_empty(), "{t} holds rows");
    }
    assert!(rows > 30_000, "{rows}");
    dos_db.close().await;
    wg_db.close().await;
}

// The dry run's checks read the same on both twins.
#[tokio::test]
async fn the_dry_run_checks_read_the_same_on_both_twins() {
    let dos = dos_realm("wg-checks-dos");
    let wg = worldgroup_realm(&dos, "wg-checks-wg");
    let run = |d: PathBuf| async move {
        let opts = ImportOptions {
            dry_run: true,
            ..ImportOptions::live(d)
        };
        Importer::new(opts, None).run().await.unwrap().checks
    };
    let (a, b) = (run(dos).await, run(wg).await);
    assert_eq!(a[0], "check layouts: DOS");
    assert_eq!(b[0], "check layouts: Worldgroup 3.x");
    assert_eq!(a[1..], b[1..]);
    let actmon = b.iter().find(|l| l.contains("equal to it")).unwrap();
    assert!(
        actmon
            .starts_with("check active monsters: of 2 with a known monster, equal to it: level 2,"),
        "{actmon}"
    );
    assert!(
        actmon.contains("name 2,") && actmon.contains("exp 2,"),
        "{actmon}"
    );
}
