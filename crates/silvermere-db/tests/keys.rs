//! Key semantics: the Btrieve ACS ALLCAPS keys are case-insensitive, duplicates are
//! refused, the top list is descending and the name scan is "at or after".

use crate::common;

use common::*;
use silvermere_db::DbError;
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::TextBlockNo;
use silvermere_types::records::TextBlock;

/// WCCUSERS key 0 `userid[30]` is ACS ALLCAPS: loading a player finds the record
/// whatever the case typed.
#[tokio::test]
async fn by_userid_is_case_insensitive() {
    let db = db().await;
    let mut seq = Seq::new(20);
    let acct = new_account(&db, &mut seq, "alice").await;
    let c = character(&mut seq, "alice", "Alice");
    let id = db.characters().insert(acct, &c).await.unwrap();
    let found = db.characters().by_userid("ALICE").await.unwrap().unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.character, c);
    assert_eq!(
        db.characters()
            .by_first_name("aLiCe")
            .await
            .unwrap()
            .map(|r| r.id),
        Some(id)
    );
    assert_eq!(db.characters().by_userid("alicia").await.unwrap(), None);
    db.close().await;
}

/// WCCUSERS key 1 `firstname[11]` is unique and ACS ALLCAPS: a second "alice" in another
/// case is a duplicate (Btrieve's duplicate-key error).
#[tokio::test]
async fn duplicate_first_name_in_another_case_is_refused() {
    let db = db().await;
    let mut seq = Seq::new(21);
    let a = new_account(&db, &mut seq, "a1").await;
    let b = new_account(&db, &mut seq, "b1").await;
    db.characters()
        .insert(a, &character(&mut seq, "a1", "Alice"))
        .await
        .unwrap();
    let err = db
        .characters()
        .insert(b, &character(&mut seq, "b1", "ALICE"))
        .await;
    assert!(matches!(err, Err(DbError::Duplicate(_))), "{err:?}");
    // the same userid in another case is a duplicate too
    let err = db
        .characters()
        .insert(b, &character(&mut seq, "A1", "Bob"))
        .await;
    assert!(matches!(err, Err(DbError::Duplicate(_))), "{err:?}");
    // and the refused insert left nothing behind
    assert_eq!(db.characters().count().await.unwrap(), 1);
    db.close().await;
}

/// A character taken off the top list is left out before the top `n` are taken, so the
/// list is still full.
#[tokio::test]
async fn top_by_exp_leaves_out_notopten_first() {
    let db = db().await;
    let mut seq = Seq::new(23);
    for (i, (name, exp, off)) in [
        ("Low", 10u32, false),
        ("High", 1000, true),
        ("Mid", 500, false),
        ("Top", 2000, false),
    ]
    .iter()
    .enumerate()
    {
        let acct = new_account(&db, &mut seq, &format!("u{i}")).await;
        let mut c = character(&mut seq, &format!("u{i}"), name);
        c.exp = *exp;
        c.settings.set(PlayerSettings::NOTOPTEN, *off);
        db.characters().insert(acct, &c).await.unwrap();
    }
    let top: Vec<String> = db
        .characters()
        .top_by_exp(3)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.character.firstname)
        .collect();
    assert_eq!(top, ["Top", "Mid", "Low"]);
    db.close().await;
}

/// Of two characters with equal experience the later by user-id comes first, whatever
/// order they were stored in.
#[tokio::test]
async fn top_by_exp_ties_later_userid_first() {
    let db = db().await;
    let mut seq = Seq::new(24);
    for (userid, name) in [("ABLE", "Able"), ("ZED", "Zed"), ("mid", "Mid")] {
        let acct = new_account(&db, &mut seq, userid).await;
        let mut c = character(&mut seq, userid, name);
        c.exp = 700;
        c.settings.remove(PlayerSettings::NOTOPTEN);
        db.characters().insert(acct, &c).await.unwrap();
    }
    let top: Vec<String> = db
        .characters()
        .top_by_exp(3)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.character.firstname)
        .collect();
    assert_eq!(top, ["Zed", "Mid", "Able"]);
    db.close().await;
}

/// The top list orders by key 2 `exp` descending.
#[tokio::test]
async fn top_by_exp_is_descending() {
    let db = db().await;
    let mut seq = Seq::new(22);
    for (i, (name, exp)) in [
        ("Low", 10u32),
        ("High", 1000),
        ("Mid", 500),
        ("Zero", 0),
        ("Top", 4_000_000_000),
    ]
    .iter()
    .enumerate()
    {
        let acct = new_account(&db, &mut seq, &format!("u{i}")).await;
        let mut c = character(&mut seq, &format!("u{i}"), name);
        c.exp = *exp;
        // on the list (the filled record may have any settings)
        c.settings.remove(PlayerSettings::NOTOPTEN);
        db.characters().insert(acct, &c).await.unwrap();
    }
    let top: Vec<(String, u32)> = db
        .characters()
        .top_by_exp(3)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.character.firstname, r.character.exp))
        .collect();
    assert_eq!(
        top,
        vec![
            ("Top".to_string(), 4_000_000_000),
            ("High".to_string(), 1000),
            ("Mid".to_string(), 500)
        ]
    );
    // scan walks every record in id order
    let mut names = Vec::new();
    let n = db
        .characters()
        .scan(|r| names.push(r.character.firstname))
        .await
        .unwrap();
    assert_eq!(n, 5);
    assert_eq!(names, vec!["Low", "High", "Mid", "Zero", "Top"]);
    db.close().await;
}

/// The key 1 name scan: the first name at or after the one typed;
/// "Kas" finds Kasimir before Kasper, and the caller checks the prefix.
#[tokio::test]
async fn by_first_name_prefix_finds_the_next_name() {
    let db = db().await;
    let mut seq = Seq::new(23);
    for (i, name) in ["Kasper", "Zed", "Kasimir", "Alice", "kali"]
        .iter()
        .enumerate()
    {
        let acct = new_account(&db, &mut seq, &format!("p{i}")).await;
        db.characters()
            .insert(acct, &character(&mut seq, &format!("p{i}"), name))
            .await
            .unwrap();
    }
    let hit = |name: &'static str| async {
        db.characters()
            .by_first_name_prefix(name)
            .await
            .unwrap()
            .map(|r| r.character.firstname)
    };
    assert_eq!(hit("Kas").await, Some("Kasimir".to_string()));
    assert_eq!(hit("kasp").await, Some("Kasper".to_string()));
    assert_eq!(hit("KAL").await, Some("kali".to_string()));
    assert_eq!(hit("a").await, Some("Alice".to_string()));
    // past the last name: nothing, as in the original
    assert_eq!(hit("zz").await, None);
    db.close().await;
}

/// A long text reads parts 0, 1, 2 of one block in order and keeps the first part's link.
#[tokio::test]
async fn text_block_concatenates_three_parts_in_order() {
    let db = db().await;
    let parts = [
        TextBlock {
            part: 2,
            number: TextBlockNo(40),
            link: TextBlockNo(0),
            text: "third.".to_string(),
        },
        TextBlock {
            part: 0,
            number: TextBlockNo(40),
            link: TextBlockNo(41),
            text: "first, ".to_string(),
        },
        TextBlock {
            part: 1,
            number: TextBlockNo(40),
            link: TextBlockNo(0),
            text: "second, ".to_string(),
        },
        // a neighbouring block that must not be joined
        TextBlock {
            part: 0,
            number: TextBlockNo(41),
            link: TextBlockNo(0),
            text: "other".to_string(),
        },
    ];
    for p in &parts {
        db.text_blocks().insert(p).await.unwrap();
    }
    let block = db
        .text_blocks()
        .block(0, TextBlockNo(40))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.text, "first, second, third.");
    assert_eq!(block.link, TextBlockNo(41));
    assert_eq!(block.parts, 3);
    let tail = db
        .text_blocks()
        .block(1, TextBlockNo(40))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tail.text, "second, third.");
    assert_eq!(
        db.text_blocks().block(0, TextBlockNo(42)).await.unwrap(),
        None
    );
    db.close().await;
}

/// WCCGANGS key 0 `key[20]` is the upper-cased name; `get_gang` matches any case.
#[tokio::test]
async fn gang_by_name_is_case_insensitive_and_top_is_descending() {
    let db = db().await;
    let mut seq = Seq::new(24);
    for (name, exp) in [("Shadows", 300u32), ("Wolves", 900), ("Owls", 1)] {
        let mut g = gang(&mut seq, name);
        g.exp = exp;
        db.gangs().insert(&g).await.unwrap();
    }
    assert_eq!(
        db.gangs().by_name("shadows").await.unwrap().map(|g| g.name),
        Some("Shadows".to_string())
    );
    assert!(matches!(
        db.gangs().insert(&gang(&mut seq, "WOLVES")).await,
        Err(DbError::Duplicate(_))
    ));
    let top: Vec<String> = db
        .gangs()
        .top_by_exp(2)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(top, vec!["Wolves", "Shadows"]);
    db.close().await;
}
