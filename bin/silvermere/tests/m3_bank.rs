//! M3 bank tests (T-048): the deposit / bankbook / withdraw sequence through a live
//! session at the Bank of Godfrey (1:297, shop 8), the lines of `cmd_deposit`,
//! `cmd_withdraw` and `cmd_bankbook` and the bank's rate card. There is no bank capture of
//! the original yet (the card asks for one recorded under MBBSEmu with
//! `tools/differential/scenarios/m3-bank.json`); the expectations here are the game's
//! texts, the same ones `crates/silvermere-engine/tests/bank.rs` pins.

mod common;

use common::m3::{
    Player, assert_classified, body_lines, classify, edit_char, place, stored, stored_balance,
    team_db,
};
use common::stock::start;

const BANK_ROOM: u32 = 297;
const BANK: i32 = 8;

/// `deposit 523`, `bankbook`, `withdraw 500`, `balance`: the registry names
/// `bank_deposit`, `bank_balance`, `bank_withdraw`; `borrow` and `repay` are no commands in
/// this build (they are spoken as `You say "borrow"`, C over the card); the book and the
/// purse are saved, and a restart shows the same `bankbook`.
#[tokio::test]
async fn bank_roundtrip() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", BANK_ROOM).await;
    edit_char(&url, "Aldric", |c| c.coins.gold = 10).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let steps = [
        (
            "deposit 523",
            "You deposit 5 gold crowns, 2 silver nobles, 3 copper farthings.",
            "bank_deposit",
        ),
        (
            "bankbook",
            "Your balance at Bank of Godfrey (#8) is:\nOn deposit: 523 copper farthings [5.23 gold crowns]",
            "bank_balance",
        ),
        (
            "withdraw 500",
            "You withdrew 500 copper farthings.",
            "bank_withdraw",
        ),
        (
            "balance",
            "Your balance at Bank of Godfrey (#8) is:\nOn deposit: 23 copper farthings [0.23 gold crowns]",
            "bank_balance",
        ),
    ];
    for (line, want, name) in steps {
        let at = p.c.position();
        let t = p.cmd(line).await;
        let mut expect = vec![line.to_owned()];
        expect.extend(want.lines().map(str::to_owned));
        assert_eq!(body_lines(&t), expect, "{line}");
        let names: Vec<_> = classify(&p.c.raw()[at..])
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.iter().any(|n| n == name), "{line}: {names:?}");
        assert_classified(line, &p.c.raw()[at..], &[line]);
    }
    for word in ["borrow", "repay"] {
        let t = p.cmd(word).await;
        assert_eq!(
            body_lines(&t),
            [word.to_owned(), format!("You say \"{word}\"")]
        );
    }
    assert!(
        p.cmd("i")
            .await
            .contains("You are carrying 9 gold crowns, 7 silver nobles, 7 copper farthings")
    );
    p.leave().await;
    r.stop().await.unwrap();
    // persistence: the book row and the purse
    assert_eq!(stored_balance(&url, "Aldric", BANK).await, Some(23));
    let rec = stored(&url, "Aldric").await;
    assert_eq!(
        (rec.coins.gold, rec.coins.silver, rec.coins.copper),
        (9, 7, 7)
    );
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let again = p.cmd("bankbook").await;
    assert!(
        again.contains("On deposit: 23 copper farthings [0.23 gold crowns]"),
        "{again}"
    );
    p.leave().await;
    r.stop().await.unwrap();
}

/// `list` in a bank prints the rate card only and makes no book.
#[tokio::test]
async fn bank_rate_card() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", BANK_ROOM).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let t = p.cmd("list").await;
    assert_eq!(
        body_lines(&t),
        [
            "list",
            "Banking services:",
            "Deposit Rate: ??",
            "Loan Rate: ??"
        ]
    );
    p.leave().await;
    r.stop().await.unwrap();
    assert_eq!(
        stored_balance(&url, "Aldric", BANK).await,
        None,
        "no book yet"
    );
}
