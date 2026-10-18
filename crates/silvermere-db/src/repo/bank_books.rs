//! Bank books: `bank_books` (WCCBANKS, key 0 `(userid, bank)`): reading and saving one
//! book (update if it exists, insert otherwise) and the range scan over one user's
//! accounts.

use silvermere_types::ids::{AccountId, ShopNo};
use silvermere_types::records::BankBook;

use crate::Result;
use crate::pool::Db;
use crate::repo::{Args, Conn, Get, last_per_key, select_sql, upsert_many, upsert_sql};

const COLS: &[&str] = &["account_id", "userid", "bank", "balance", "loan"];

fn read(row: &sqlx::any::AnyRow) -> Result<BankBook> {
    let g = Get::new(row, "bank_books");
    Ok(BankBook {
        userid: g.text("userid")?,
        bank: ShopNo(g.int("bank")?),
        balance: g.int("balance")?,
        loan: g.int("loan")?,
    })
}

fn args(account: AccountId, book: &BankBook) -> Args {
    let mut a = Args::new();
    a.add(account.0)
        .add(&book.userid)
        .add(book.bank.0)
        .add(book.balance)
        .add(book.loan);
    a
}

/// [`put`] for many books in multi-row statements (T-113)..
pub(crate) async fn put_many_of(conn: &mut Conn, rows: &[(AccountId, &BankBook)]) -> Result<()> {
    // a repeated key keeps its last record, as one upsert per write would
    let rows = last_per_key(rows, |(a, b)| (a.0, b.bank.0))
        .into_iter()
        .map(|(a, b)| args(*a, b))
        .collect();
    upsert_many(
        conn,
        "bank_books",
        COLS,
        &["account_id", "bank"],
        rows,
        None,
    )
    .await?;
    Ok(())
}

/// Upserts the book of `account` at `book.bank`.
pub(crate) async fn put(conn: &mut Conn, account: AccountId, book: &BankBook) -> Result<()> {
    args(account, book)
        .query(upsert_sql("bank_books", COLS, &["account_id", "bank"]))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Deletes the book of `account` at `bank`.
pub(crate) async fn delete(conn: &mut Conn, account: AccountId, bank: ShopNo) -> Result<bool> {
    let mut a = Args::new();
    a.add(account.0).add(bank.0);
    let done = a
        .query("DELETE FROM bank_books WHERE account_id = $1 AND bank = $2".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Bank books.
#[derive(Debug, Clone, Copy)]
pub struct BankBooks<'a> {
    db: &'a Db,
}

impl<'a> BankBooks<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Every book of `account` in bank order (the `agebtv` key 0 range scan).
    pub async fn for_account(&self, account: AccountId) -> Result<Vec<BankBook>> {
        let mut a = Args::new();
        a.add(account.0);
        let rows = a
            .query(select_sql(
                "bank_books",
                COLS,
                "WHERE account_id = $1 ORDER BY bank",
            ))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// Every book of every account, by account then bank. The engine holds all books in
    /// memory, so the snapshot loads them at start-up like `item_ownership`.
    pub async fn all(&self) -> Result<Vec<(AccountId, BankBook)>> {
        let rows = Args::new()
            .query(select_sql("bank_books", COLS, "ORDER BY account_id, bank"))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter()
            .map(|row| {
                let account = Get::new(row, "bank_books").int("account_id")?;
                Ok((AccountId(account), read(row)?))
            })
            .collect()
    }

    /// The book of `account` at `bank`.
    pub async fn get(&self, account: AccountId, bank: ShopNo) -> Result<Option<BankBook>> {
        let mut a = Args::new();
        a.add(account.0).add(bank.0);
        let row = a
            .query(select_sql(
                "bank_books",
                COLS,
                "WHERE account_id = $1 AND bank = $2",
            ))
            .fetch_optional(&self.db.pool())
            .await?;
        row.as_ref().map(read).transpose()
    }

    /// Writes `book` for `account` (insert or update).
    pub async fn put(&self, account: AccountId, book: &BankBook) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put(&mut conn, account, book).await
    }

    /// Deletes the book of `account` at `bank`.
    pub async fn delete(&self, account: AccountId, bank: ShopNo) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, account, bank).await
    }
}
