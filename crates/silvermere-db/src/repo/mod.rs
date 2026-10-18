//! The repositories: one per table group, named lookups instead of a cursor API.
//!
//! Each module has a public repository type (`Rooms`, `Characters`, ...) handed out by
//! [`crate::Db`], and `pub(crate)` functions that run against a connection or a
//! transaction so the persistence worker can batch them. Reads return the record
//! structs of `silvermere-types`; the live parts the original kept inside the same
//! record are the state structs here ([`RoomState`], [`ShopStock`], [`MonsterState`]).
//!
//! Helpers shared by the modules: [`Args`] collects bound values, [`Get`] reads typed
//! columns with a `DbError::Decode` on overflow, and the `*_sql` functions build the
//! portable statements from column lists so a column is named once.

pub mod accounts;
pub mod active_monsters;
pub mod bank_books;
pub mod characters;
pub mod classes;
pub mod evil_points;
pub mod gangs;
pub mod item_ownership;
pub mod items;
pub mod messages;
pub mod monsters;
pub mod profiles;
pub mod races;
pub mod rooms;
pub mod shops;
pub mod spells;
pub mod text_blocks;

pub use accounts::{Account, AccountRow, Accounts};
pub use active_monsters::ActiveMonsters;
pub use bank_books::BankBooks;
pub use characters::{CharacterRow, Characters};
pub use classes::Classes;
pub use evil_points::EvilPoints;
pub use gangs::Gangs;
pub use item_ownership::ItemOwnerships;
pub use items::Items;
pub use messages::Messages;
pub use monsters::{MonsterState, MonsterStates, Monsters};
pub use profiles::Profiles;
pub use races::Races;
pub use rooms::{RoomState, RoomStates, Rooms};
pub use shops::{ShopStock, ShopStocks, Shops};
pub use spells::Spells;
pub use text_blocks::{Actions, TextBlockText, TextBlocks};

use sqlx::AnyConnection;
use sqlx::any::{Any, AnyArguments, AnyRow};
use sqlx::query::Query;
use sqlx::{AssertSqlSafe, Row};

use silvermere_types::enums::Ability;
use silvermere_types::records::{AbilitySlot, Coins};

use crate::{DbError, Result};

/// A connection or an open transaction.
pub(crate) type Conn = AnyConnection;

/// A value bound to a `$n` placeholder.
#[derive(Debug, Clone)]
pub(crate) enum Arg {
    Int(i64),
    Text(String),
    Bool(bool),
    Null,
}

macro_rules! int_arg {
    ($($t:ty),*) => { $(
        impl From<$t> for Arg {
            fn from(v: $t) -> Self {
                Self::Int(i64::from(v))
            }
        }
    )* };
}
int_arg!(i8, u8, i16, u16, i32, u32, i64);

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Self::Text(v.to_string())
    }
}
impl From<&String> for Arg {
    fn from(v: &String) -> Self {
        Self::Text(v.clone())
    }
}
impl From<String> for Arg {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
impl From<bool> for Arg {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl<T: Into<Arg>> From<Option<T>> for Arg {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

/// The bound values of one statement, in placeholder order.
#[derive(Debug, Default, Clone)]
pub(crate) struct Args(Vec<Arg>);

impl Args {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds one value.
    pub(crate) fn add(&mut self, v: impl Into<Arg>) -> &mut Self {
        self.0.push(v.into());
        self
    }

    /// Adds the five coin columns, copper first.
    pub(crate) fn coins(&mut self, c: Coins) -> &mut Self {
        self.add(c.copper)
            .add(c.silver)
            .add(c.gold)
            .add(c.platinum)
            .add(c.runic)
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn extend(&mut self, other: Args) -> &mut Self {
        self.0.extend(other.0);
        self
    }

    /// Binds the values to `sql`.
    pub(crate) fn query(self, sql: String) -> Query<'static, Any, AnyArguments> {
        let mut q = sqlx::query(AssertSqlSafe(sql));
        for a in self.0 {
            q = match a {
                Arg::Int(v) => q.bind(v),
                Arg::Text(v) => q.bind(v),
                Arg::Bool(v) => q.bind(v),
                Arg::Null => q.bind(Option::<i64>::None),
            };
        }
        q
    }
}

/// Typed reads of one row's columns.
pub(crate) struct Get<'r> {
    row: &'r AnyRow,
    table: &'static str,
}

impl<'r> Get<'r> {
    pub(crate) fn new(row: &'r AnyRow, table: &'static str) -> Self {
        Self { row, table }
    }

    fn decode_err(&self, column: &str, reason: impl ToString) -> DbError {
        DbError::Decode {
            table: self.table,
            column: column.to_string(),
            reason: reason.to_string(),
        }
    }

    pub(crate) fn i64(&self, column: &str) -> Result<i64> {
        self.row
            .try_get::<i64, _>(column)
            .map_err(|e| self.decode_err(column, e))
    }

    /// An integer column converted to the field's width.
    pub(crate) fn int<T: TryFrom<i64>>(&self, column: &str) -> Result<T> {
        let v = self.i64(column)?;
        T::try_from(v).map_err(|_| self.decode_err(column, format!("{v} out of range")))
    }

    pub(crate) fn opt_i64(&self, column: &str) -> Result<Option<i64>> {
        self.row
            .try_get::<Option<i64>, _>(column)
            .map_err(|e| self.decode_err(column, e))
    }

    pub(crate) fn text(&self, column: &str) -> Result<String> {
        self.row
            .try_get::<String, _>(column)
            .map_err(|e| self.decode_err(column, e))
    }

    pub(crate) fn opt_text(&self, column: &str) -> Result<Option<String>> {
        self.row
            .try_get::<Option<String>, _>(column)
            .map_err(|e| self.decode_err(column, e))
    }

    /// The five coin columns `prefix{copper..runic}`.
    pub(crate) fn coins(&self, prefix: &str) -> Result<Coins> {
        Ok(Coins {
            copper: self.int(&format!("{prefix}copper"))?,
            silver: self.int(&format!("{prefix}silver"))?,
            gold: self.int(&format!("{prefix}gold"))?,
            platinum: self.int(&format!("{prefix}platinum"))?,
            runic: self.int(&format!("{prefix}runic"))?,
        })
    }

    /// An enum code column.
    pub(crate) fn code<E: From<u16>>(&self, column: &str) -> Result<E> {
        Ok(E::from(self.int::<u16>(column)?))
    }
}

/// `$n` placeholders `from..from+count`, comma separated.
pub(crate) fn placeholders(from: usize, count: usize) -> String {
    (from..from + count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `INSERT INTO table (cols) VALUES ($1, ...)`.
pub(crate) fn insert_sql(table: &str, cols: &[&str]) -> String {
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        cols.join(", "),
        placeholders(1, cols.len())
    )
}

/// `INSERT ... ON CONFLICT (keys) DO UPDATE SET col = excluded.col` for every non-key
/// column (portable across SQLite and PostgreSQL).
pub(crate) fn upsert_sql(table: &str, cols: &[&str], keys: &[&str]) -> String {
    let updates: Vec<String> = cols
        .iter()
        .filter(|c| !keys.contains(c))
        .map(|c| format!("{c} = excluded.{c}"))
        .collect();
    if updates.is_empty() {
        return format!(
            "{} ON CONFLICT ({}) DO NOTHING",
            insert_sql(table, cols),
            keys.join(", ")
        );
    }
    format!(
        "{} ON CONFLICT ({}) DO UPDATE SET {}",
        insert_sql(table, cols),
        keys.join(", "),
        updates.join(", ")
    )
}

/// `SELECT cols FROM table suffix`.
pub(crate) fn select_sql(table: &str, cols: &[&str], suffix: &str) -> String {
    let mut sql = format!("SELECT {} FROM {table}", cols.join(", "));
    if !suffix.is_empty() {
        sql.push(' ');
        sql.push_str(suffix);
    }
    sql
}

/// Inserts `rows` (each `cols.len()` values) with multi-row `VALUES`, in chunks that keep
/// under the backends' placeholder limits. Nothing is sent for no rows.
pub(crate) async fn insert_many(
    conn: &mut Conn,
    table: &str,
    cols: &[&str],
    rows: Vec<Args>,
) -> Result<()> {
    const MAX_PARAMS: usize = 4000;
    let width = cols.len();
    let per_chunk = (MAX_PARAMS / width).max(1);
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let mut args = Args::new();
        let mut tuples = Vec::new();
        for row in rows.by_ref().take(per_chunk) {
            debug_assert_eq!(row.len(), width);
            tuples.push(format!("({})", placeholders(args.len() + 1, width)));
            args.extend(row);
        }
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES {}",
            cols.join(", "),
            tuples.join(", ")
        );
        args.query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// What a bulk content write ([`Rooms::put_many`] and the other `put_many`s) does with
/// the state stored beside the content: the importer's `--content-only`, default and
/// `--reset-state` modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateWrite {
    /// Write content only; state rows are neither created nor changed.
    Untouched,
    /// Create state from the record where a row has none (a fresh import); existing state
    /// is kept.
    CreateMissing,
    /// Rewrite state from the record for every row written.
    Reset,
}

impl StateWrite {
    /// Whether a row's state is written, given whether it already had state.
    pub fn writes(self, had_state: bool) -> bool {
        match self {
            Self::Untouched => false,
            Self::CreateMissing => !had_state,
            Self::Reset => true,
        }
    }
}

/// Bound values per statement of the bulk writers ([`upsert_many`],
/// [`insert_many_bulk`]). Measured on the stock import (26,720 rooms, SQLite, release):
/// statements of 4,000 values took 14 s, of 500 values 3 s; the cost per value grows with
/// the statement size.
const BULK_PARAMS: usize = 500;

/// Whether `conn` is on PostgreSQL.
pub(crate) fn is_postgres(conn: &Conn) -> bool {
    conn.backend_name() == crate::Dialect::Postgres.name()
}

/// Parents per chunk of a bulk writer whose chunk is `sqlite` parents on SQLite: ten
/// times that on PostgreSQL, where each chunk's statements cost round trips (T-113).
pub(crate) fn parents_per_chunk(conn: &Conn, sqlite: usize) -> usize {
    if is_postgres(conn) {
        sqlite * 10
    } else {
        sqlite
    }
}

/// [`BULK_PARAMS`] on PostgreSQL, where each statement is a network round trip: the stock
/// import against a hosted database 12 ms away took 7.5 minutes in statements of 500
/// values (T-108). PostgreSQL takes up to 65,535 values a statement.
const PG_BULK_PARAMS: usize = 16_000;

/// The bound values per bulk statement on `conn`'s backend.
fn bulk_params(conn: &Conn) -> usize {
    if is_postgres(conn) {
        PG_BULK_PARAMS
    } else {
        BULK_PARAMS
    }
}

/// [`insert_many`] in statements of at most [`bulk_params`] values, for the bulk writers.
pub(crate) async fn insert_many_bulk(
    conn: &mut Conn,
    table: &str,
    cols: &[&str],
    rows: Vec<Args>,
) -> Result<()> {
    let width = cols.len();
    let per_chunk = (bulk_params(conn) / width).max(1);
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let mut args = Args::new();
        let mut tuples = Vec::new();
        for row in rows.by_ref().take(per_chunk) {
            debug_assert_eq!(row.len(), width);
            tuples.push(format!("({})", placeholders(args.len() + 1, width)));
            args.extend(row);
        }
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES {}",
            cols.join(", "),
            tuples.join(", ")
        );
        args.query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Multi-row `INSERT ... ON CONFLICT (keys) DO UPDATE` of `rows`, in statements of at
/// most [`bulk_params`] values; with `returning` (a column list) the returned rows of
/// every statement. The rows must not repeat a key (PostgreSQL refuses to update a row
/// twice).
pub(crate) async fn upsert_many(
    conn: &mut Conn,
    table: &str,
    cols: &[&str],
    keys: &[&str],
    rows: Vec<Args>,
    returning: Option<&str>,
) -> Result<Vec<AnyRow>> {
    let width = cols.len();
    let per_chunk = (bulk_params(conn) / width).max(1);
    let single = upsert_sql(table, cols, keys);
    // The single-row statement's tail: everything after its one VALUES tuple.
    let values_one = format!("({})", placeholders(1, width));
    let at = single
        .find(&values_one)
        .expect("upsert_sql writes one VALUES tuple");
    let (head, tail) = (&single[..at], &single[at + values_one.len()..]);
    let mut out = Vec::new();
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let mut args = Args::new();
        let mut tuples = Vec::new();
        for row in rows.by_ref().take(per_chunk) {
            debug_assert_eq!(row.len(), width);
            tuples.push(format!("({})", placeholders(args.len() + 1, width)));
            args.extend(row);
        }
        let mut sql = format!("{head}{}{tail}", tuples.join(", "));
        match returning {
            Some(cols) => {
                sql.push_str(" RETURNING ");
                sql.push_str(cols);
                out.extend(args.query(sql).fetch_all(&mut *conn).await?);
            }
            None => {
                args.query(sql).execute(&mut *conn).await?;
            }
        }
    }
    Ok(out)
}

/// Writes for many parents, sent as bulk statements by [`Batch::flush`] (T-108): every
/// upsert, then every child delete, then every insert, each table in the order it was first
/// written. A per-row write of the stock world was 37,000 statements, minutes over a WAN.
///
/// A key upserted twice keeps the last row, as the per-row writes did (PostgreSQL refuses
/// one statement that updates a row twice). Children are not keyed: a caller replacing a
/// parent's children writes each parent once, after [`last_per_key`].
#[derive(Default)]
pub(crate) struct Batch {
    upserts: Vec<Upserts>,
    deletes: Vec<(String, String, Vec<i64>)>,
    inserts: Vec<(String, Vec<String>, Vec<Args>)>,
}

/// One table's rows for [`Batch::upsert`], with the index of each key's row.
struct Upserts {
    table: String,
    cols: &'static [&'static str],
    keys: &'static [&'static str],
    rows: Vec<Args>,
    at: std::collections::HashMap<String, usize>,
}

/// The rows of `rows` that are the last with their key, in their order (T-108): what a
/// per-row write left when a key repeats.
pub(crate) fn last_per_key<T, K: std::hash::Hash + Eq>(
    rows: &[T],
    key: impl Fn(&T) -> K,
) -> Vec<&T> {
    let mut last = std::collections::HashMap::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        last.insert(key(r), i);
    }
    let mut keep: Vec<usize> = last.into_values().collect();
    keep.sort_unstable();
    keep.into_iter().map(|i| &rows[i]).collect()
}

impl Batch {
    /// `INSERT ... ON CONFLICT (keys) DO UPDATE` of `row`, whose key values are `key`; a row
    /// already batched with the same key is replaced.
    pub(crate) fn upsert(
        &mut self,
        table: &str,
        cols: &'static [&'static str],
        keys: &'static [&'static str],
        key: impl std::fmt::Debug,
        row: Args,
    ) {
        let key = format!("{key:?}");
        let u = match self.upserts.iter().position(|u| u.table == table) {
            Some(i) => &mut self.upserts[i],
            None => {
                self.upserts.push(Upserts {
                    table: table.to_string(),
                    cols,
                    keys,
                    rows: Vec::new(),
                    at: Default::default(),
                });
                self.upserts.last_mut().expect("just pushed")
            }
        };
        match u.at.get(&key) {
            Some(&i) => u.rows[i] = row,
            None => {
                u.at.insert(key, u.rows.len());
                u.rows.push(row);
            }
        }
    }

    /// `DELETE FROM table WHERE parent_col = parent`, before any insert.
    pub(crate) fn delete_children(&mut self, table: &str, parent_col: &str, parent: i64) {
        match self
            .deletes
            .iter_mut()
            .find(|d| d.0 == table && d.1 == parent_col)
        {
            Some(d) => d.2.push(parent),
            None => self
                .deletes
                .push((table.to_string(), parent_col.to_string(), vec![parent])),
        }
    }

    /// `INSERT INTO table (cols)` of `rows`.
    pub(crate) fn insert(&mut self, table: &str, cols: &[&str], rows: Vec<Args>) {
        if rows.is_empty() {
            return;
        }
        match self.inserts.iter_mut().find(|i| i.0 == table) {
            Some(i) => {
                // values bound against another column list would land in the wrong columns
                assert!(i.1.iter().eq(cols.iter()), "{table}: one column list");
                i.2.extend(rows);
            }
            None => self.inserts.push((
                table.to_string(),
                cols.iter().map(|c| c.to_string()).collect(),
                rows,
            )),
        }
    }

    /// The used slots of `abilities` into a `(parent, slot, ability, value)` table.
    pub(crate) fn abilities(
        &mut self,
        table: &str,
        parent_col: &str,
        parent: i64,
        abilities: &[AbilitySlot],
    ) {
        self.insert(
            table,
            &[parent_col, "slot", "ability", "value"],
            ability_rows(parent, abilities),
        );
    }

    /// Sends everything collected, on `conn` (a transaction when it must be atomic).
    pub(crate) async fn flush(self, conn: &mut Conn) -> Result<()> {
        for u in self.upserts {
            upsert_many(conn, &u.table, u.cols, u.keys, u.rows, None).await?;
        }
        for (table, col, parents) in self.deletes {
            delete_children_in(conn, &table, &col, &parents).await?;
        }
        for (table, cols, rows) in self.inserts {
            let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
            insert_many_bulk(conn, &table, &cols, rows).await?;
        }
        Ok(())
    }
}

/// `DELETE FROM table WHERE parent_col IN (parents)`, chunked.
pub(crate) async fn delete_children_in(
    conn: &mut Conn,
    table: &str,
    parent_col: &str,
    parents: &[i64],
) -> Result<()> {
    for chunk in parents.chunks(1000) {
        let mut args = Args::new();
        for p in chunk {
            args.add(*p);
        }
        let sql = format!(
            "DELETE FROM {table} WHERE {parent_col} IN ({})",
            placeholders(1, chunk.len())
        );
        args.query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Which parent rows a child-table read covers.
#[derive(Debug, Clone)]
pub(crate) enum Scope {
    /// One parent, by its key column value.
    One(i64),
    /// Every parent.
    All,
    /// The parents matching a subquery (`WHERE parent IN (subquery)`), with its arguments.
    In(String, Args),
}

impl Scope {
    /// `WHERE parent_col ...` for this scope, with the bound arguments.
    pub(crate) fn child_where(&self, parent_col: &str) -> (String, Args) {
        match self {
            Scope::One(id) => {
                let mut args = Args::new();
                args.add(*id);
                (format!("WHERE {parent_col} = $1"), args)
            }
            Scope::All => (String::new(), Args::new()),
            Scope::In(subquery, args) => {
                (format!("WHERE {parent_col} IN ({subquery})"), args.clone())
            }
        }
    }
}

/// Reads a `(parent, slot, ability, value)` child table into per-parent arrays of `N`
/// ability slots (empty slots are `ABL_NONE`, 0).
pub(crate) async fn read_abilities<const N: usize>(
    conn: &mut Conn,
    table: &'static str,
    parent_col: &str,
    scope: &Scope,
) -> Result<std::collections::HashMap<i64, [AbilitySlot; N]>> {
    let (where_, args) = scope.child_where(parent_col);
    let sql = format!(
        "SELECT {parent_col} AS parent, slot, ability, value FROM {table} {where_} ORDER BY parent, slot"
    );
    let rows = args.query(sql).fetch_all(&mut *conn).await?;
    let mut out: std::collections::HashMap<i64, [AbilitySlot; N]> = Default::default();
    for row in &rows {
        let g = Get::new(row, table);
        let parent = g.i64("parent")?;
        let slot: usize = g.int("slot")?;
        let entry = out.entry(parent).or_insert([(Ability::None, 0); N]);
        if slot < N {
            entry[slot] = (g.code("ability")?, g.int("value")?);
        }
    }
    Ok(out)
}

/// Writes the used slots of `abilities` to a `(parent, slot, ability, value)` table.
pub(crate) async fn write_abilities(
    conn: &mut Conn,
    table: &str,
    parent_col: &str,
    parent: i64,
    abilities: &[AbilitySlot],
) -> Result<()> {
    let rows = ability_rows(parent, abilities);
    insert_many(conn, table, &[parent_col, "slot", "ability", "value"], rows).await
}

/// The `(parent, slot, ability, value)` rows of the used slots of `abilities`.
pub(crate) fn ability_rows(parent: i64, abilities: &[AbilitySlot]) -> Vec<Args> {
    abilities
        .iter()
        .enumerate()
        .filter(|(_, (ability, value))| *ability != Ability::None || *value != 0)
        .map(|(slot, (ability, value))| {
            let mut a = Args::new();
            a.add(parent)
                .add(slot as u16)
                .add(ability.code())
                .add(*value);
            a
        })
        .collect()
}

/// `DELETE FROM table WHERE parent_col = parent`.
pub(crate) async fn delete_children(
    conn: &mut Conn,
    table: &str,
    parent_col: &str,
    parent: i64,
) -> Result<()> {
    let mut args = Args::new();
    args.add(parent);
    args.query(format!("DELETE FROM {table} WHERE {parent_col} = $1"))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_per_key_keeps_the_last_row_of_each_key_in_order() {
        let rows = [(1, "a"), (2, "b"), (1, "c"), (3, "d"), (2, "e")];
        let kept: Vec<_> = last_per_key(&rows, |r| r.0)
            .into_iter()
            .map(|r| r.1)
            .collect();
        assert_eq!(kept, ["c", "d", "e"]);
    }

    #[test]
    fn statement_builders() {
        assert_eq!(
            insert_sql("t", &["a", "b"]),
            "INSERT INTO t (a, b) VALUES ($1, $2)"
        );
        assert_eq!(
            upsert_sql("t", &["k", "a", "b"], &["k"]),
            "INSERT INTO t (k, a, b) VALUES ($1, $2, $3) ON CONFLICT (k) DO UPDATE SET a = excluded.a, b = excluded.b"
        );
        assert_eq!(
            upsert_sql("t", &["k", "s"], &["k", "s"]),
            "INSERT INTO t (k, s) VALUES ($1, $2) ON CONFLICT (k, s) DO NOTHING"
        );
        assert_eq!(
            select_sql("t", &["a"], "WHERE a = $1"),
            "SELECT a FROM t WHERE a = $1"
        );
        assert_eq!(placeholders(3, 2), "$3, $4");
    }
}
