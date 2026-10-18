//! Text blocks (`text_blocks`, WCCTEXT) and social actions (`actions`, `action_texts`,
//! WCCACTS).
//!
//! One part of a block is read on key 0 `(part, number)`, checking the number matched; a
//! long text is shown by reading parts 0, 1, 2.. in turn until the record found belongs to
//! another block, and the first part's `link` is kept for texts that chain on.
//! Actions are scanned whole at startup into the action list and read by name (key 0)
//! on use.

use std::collections::HashMap;

use silvermere_types::flags::ActionTargets;
use silvermere_types::ids::TextBlockNo;
use silvermere_types::records::{Action, TextBlock};
use silvermere_types::sizes::NACTTEXTS;

use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, delete_children, delete_children_in, insert_many,
    insert_many_bulk, insert_sql, is_postgres, select_sql, upsert_many, upsert_sql,
};
use crate::{DbError, Result};

const TEXT_COLS: &[&str] = &["part", "number", "link", "text"];

/// A whole text block: its parts in order, and the first part's link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBlockText {
    /// The block number.
    pub number: TextBlockNo,
    /// The parts' texts concatenated in part order.
    pub text: String,
    /// The first part's `link`: another block to continue with, 0 for none.
    pub link: TextBlockNo,
    /// How many parts were read.
    pub parts: usize,
}

fn text_args(t: &TextBlock) -> Args {
    let mut a = Args::new();
    a.add(t.part).add(t.number.0).add(t.link.0).add(&t.text);
    a
}

/// Inserts one part.
pub(crate) async fn insert_text(conn: &mut Conn, t: &TextBlock) -> Result<()> {
    text_args(t)
        .query(insert_sql("text_blocks", TEXT_COLS))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Upserts one part.
pub(crate) async fn put_text(conn: &mut Conn, t: &TextBlock) -> Result<()> {
    text_args(t)
        .query(upsert_sql("text_blocks", TEXT_COLS, &["part", "number"]))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Loads the parts in `scope` (over `number`) in `(number, part)` order.
pub(crate) async fn load_texts(conn: &mut Conn, scope: &Scope) -> Result<Vec<TextBlock>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "text_blocks",
            TEXT_COLS,
            &format!("{where_} ORDER BY number, part"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| {
            let g = Get::new(row, "text_blocks");
            Ok(TextBlock {
                part: g.int("part")?,
                number: TextBlockNo(g.int("number")?),
                link: TextBlockNo(g.int("link")?),
                text: g.text("text")?,
            })
        })
        .collect()
}

/// Text blocks (WCCTEXT).
#[derive(Debug, Clone, Copy)]
pub struct TextBlocks<'a> {
    db: &'a Db,
}

impl<'a> TextBlocks<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Part `part` of block `number`.
    pub async fn part(&self, part: i16, number: TextBlockNo) -> Result<Option<TextBlock>> {
        let mut a = Args::new();
        a.add(part).add(number.0);
        let row = a
            .query(select_sql(
                "text_blocks",
                TEXT_COLS,
                "WHERE part = $1 AND number = $2",
            ))
            .fetch_optional(&self.db.pool())
            .await?;
        row.map(|r| {
            let g = Get::new(&r, "text_blocks");
            Ok(TextBlock {
                part: g.int("part")?,
                number: TextBlockNo(g.int("number")?),
                link: TextBlockNo(g.int("link")?),
                text: g.text("text")?,
            })
        })
        .transpose()
    }

    /// A long text as the game shows it: the parts of `number`
    /// from `part` upward, concatenated in order, with the first part's `link`. `None` when
    /// the block has no part at or after `part`.
    pub async fn block(&self, part: i16, number: TextBlockNo) -> Result<Option<TextBlockText>> {
        let mut a = Args::new();
        a.add(number.0).add(part);
        let rows = a
            .query(select_sql(
                "text_blocks",
                TEXT_COLS,
                "WHERE number = $1 AND part >= $2 ORDER BY part",
            ))
            .fetch_all(&self.db.pool())
            .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut text = String::new();
        let mut link = TextBlockNo(0);
        for (i, row) in rows.iter().enumerate() {
            let g = Get::new(row, "text_blocks");
            if i == 0 {
                link = TextBlockNo(g.int("link")?);
            }
            text.push_str(&g.text("text")?);
        }
        Ok(Some(TextBlockText {
            number,
            text,
            link,
            parts: rows.len(),
        }))
    }

    /// Every part in `(number, part)` order.
    pub async fn all(&self) -> Result<Vec<TextBlock>> {
        let mut conn = self.db.pool().acquire().await?;
        load_texts(&mut conn, &Scope::All).await
    }

    /// Inserts one part; `DbError::Duplicate` when `(part, number)` exists.
    pub async fn insert(&self, t: &TextBlock) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        insert_text(&mut conn, t).await
    }

    /// Writes many text block parts in one transaction (upsert by `(part, number)`, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[TextBlock]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut b = Batch::default();
        for r in rows {
            b.upsert(
                "text_blocks",
                TEXT_COLS,
                &["part", "number"],
                (r.part, r.number.0),
                text_args(r),
            );
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts one part.
    pub async fn put(&self, t: &TextBlock) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put_text(&mut conn, t).await
    }
}

const ACTION_COLS: &[&str] = &["name", "targets"];

/// Writes `action` (upsert by name) and replaces its texts.
/// [`put_action`] for many actions in multi-row statements (T-113): the existing ids by
/// name in one read, the existing actions upserted by id and the new ones inserted, then
/// every action's texts replaced. A repeated name (in any case) keeps its last row, as
/// one [`put_action`] per row would.
async fn put_actions(conn: &mut Conn, actions: &[Action]) -> Result<()> {
    // the backend's case folding: PostgreSQL's `lower()` (the `actions_name` index),
    // SQLite's NOCASE (ASCII only)
    let pg = is_postgres(conn);
    let fold = move |name: &str| {
        if pg {
            name.to_lowercase()
        } else {
            name.to_ascii_lowercase()
        }
    };
    let mut ids: HashMap<String, i64> = HashMap::new();
    for row in Args::new()
        .query("SELECT id, name FROM actions".to_string())
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "actions");
        ids.insert(fold(&g.text("name")?), g.i64("id")?);
    }
    let mut last: HashMap<String, usize> = HashMap::new();
    for (i, a) in actions.iter().enumerate() {
        last.insert(fold(&a.name), i);
    }
    let kept: Vec<&Action> = actions
        .iter()
        .enumerate()
        .filter(|(i, a)| last[&fold(&a.name)] == *i)
        .map(|(_, a)| a)
        .collect();
    let (old, new): (Vec<&Action>, Vec<&Action>) =
        kept.iter().partition(|a| ids.contains_key(&fold(&a.name)));
    let updates: Vec<Args> = old
        .iter()
        .map(|a| {
            let mut args = Args::new();
            args.add(ids[&fold(&a.name)])
                .add(&a.name)
                .add(a.targets.bits());
            args
        })
        .collect();
    upsert_many(
        conn,
        "actions",
        &["id", "name", "targets"],
        &["id"],
        updates,
        None,
    )
    .await?;
    for chunk in new.chunks(1000) {
        let mut args = Args::new();
        let mut tuples = Vec::with_capacity(chunk.len());
        for a in chunk {
            tuples.push(format!("(${}, ${})", args.len() + 1, args.len() + 2));
            args.add(&a.name).add(a.targets.bits());
        }
        let sql = format!(
            "INSERT INTO actions (name, targets) VALUES {} RETURNING id, name",
            tuples.join(", ")
        );
        for row in args.query(sql).fetch_all(&mut *conn).await? {
            let g = Get::new(&row, "actions");
            ids.insert(fold(&g.text("name")?), g.i64("id")?);
        }
    }
    let all: Vec<i64> = kept.iter().map(|a| ids[&fold(&a.name)]).collect();
    delete_children_in(conn, "action_texts", "action_id", &all).await?;
    let rows: Vec<Args> = kept
        .iter()
        .zip(&all)
        .flat_map(|(a, &id)| text_rows(id, a))
        .collect();
    insert_many_bulk(conn, "action_texts", &["action_id", "slot", "text"], rows).await
}

/// The `action_texts` rows of `action` (id `id`).
fn text_rows(id: i64, action: &Action) -> Vec<Args> {
    action
        .texts
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.is_empty())
        .map(|(slot, t)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(t);
            a
        })
        .collect()
}

pub(crate) async fn put_action(conn: &mut Conn, db: &Db, action: &Action) -> Result<()> {
    let d = db.dialect();
    let mut a = Args::new();
    a.add(&action.name);
    let existing = a
        .query(format!(
            "SELECT id FROM actions WHERE {}",
            d.ci_eq("name", "$1")
        ))
        .fetch_optional(&mut *conn)
        .await?;
    let id = match existing {
        Some(row) => {
            let id = Get::new(&row, "actions").i64("id")?;
            let mut a = Args::new();
            a.add(&action.name).add(action.targets.bits()).add(id);
            a.query("UPDATE actions SET name = $1, targets = $2 WHERE id = $3".to_string())
                .execute(&mut *conn)
                .await?;
            delete_children(conn, "action_texts", "action_id", id).await?;
            id
        }
        None => {
            let mut a = Args::new();
            a.add(&action.name).add(action.targets.bits());
            let row = a
                .query(format!(
                    "{} RETURNING id",
                    insert_sql("actions", ACTION_COLS)
                ))
                .fetch_one(&mut *conn)
                .await?;
            Get::new(&row, "actions").i64("id")?
        }
    };
    let rows: Vec<Args> = action
        .texts
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.is_empty())
        .map(|(slot, t)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(t);
            a
        })
        .collect();
    insert_many(conn, "action_texts", &["action_id", "slot", "text"], rows).await
}

/// Inserts `action`; `DbError::Duplicate` when the name exists (case-insensitive).
pub(crate) async fn insert_action(conn: &mut Conn, db: &Db, action: &Action) -> Result<()> {
    let mut a = Args::new();
    a.add(&action.name);
    let exists = a
        .query(format!(
            "SELECT id FROM actions WHERE {}",
            db.dialect().ci_eq("name", "$1")
        ))
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    if exists {
        return Err(DbError::Duplicate(format!("action {}", action.name)));
    }
    put_action(conn, db, action).await
}

/// Loads the actions in `scope` (over `id`) in record order: `id` is the order the importer
/// wrote WCCACTS in, and the game walks the file in that physical order
/// (Btrieve Step First/Next).
pub(crate) async fn load_actions(conn: &mut Conn, scope: &Scope) -> Result<Vec<Action>> {
    let (where_, a) = scope.child_where("id");
    let rows = a
        .query(select_sql(
            "actions",
            &["id", "name", "targets"],
            &format!("{where_} ORDER BY id"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    let mut index: HashMap<i64, usize> = HashMap::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "actions");
        index.insert(g.i64("id")?, out.len());
        out.push(Action {
            name: g.text("name")?,
            texts: std::array::from_fn(|_| String::new()),
            targets: ActionTargets::from_bits_retain(g.int("targets")?),
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let (where_, a) = scope.child_where("action_id");
    let sql = format!("SELECT action_id, slot, text FROM action_texts {where_}");
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "action_texts");
        let Some(&i) = index.get(&g.i64("action_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NACTTEXTS {
            out[i].texts[slot] = g.text("text")?;
        }
    }
    Ok(out)
}

/// Social actions (WCCACTS).
#[derive(Debug, Clone, Copy)]
pub struct Actions<'a> {
    db: &'a Db,
}

impl<'a> Actions<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Every action in name order: the startup scan into the action list.
    pub async fn all(&self) -> Result<Vec<Action>> {
        let mut conn = self.db.pool().acquire().await?;
        load_actions(&mut conn, &Scope::All).await
    }

    /// The action named `name`, case-insensitive (acqbtv key 0).
    pub async fn by_name(&self, name: &str) -> Result<Option<Action>> {
        let mut conn = self.db.pool().acquire().await?;
        let mut a = Args::new();
        a.add(name);
        let scope = Scope::In(
            format!(
                "SELECT id FROM actions WHERE {}",
                self.db.dialect().ci_eq("name", "$1")
            ),
            a,
        );
        Ok(load_actions(&mut conn, &scope).await?.pop())
    }

    /// Inserts `action`; `DbError::Duplicate` when the name exists in any case.
    pub async fn insert(&self, action: &Action) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert_action(&mut tx, self.db, action).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many actions in one transaction (upsert by name, case-insensitive, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[Action]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put_actions(&mut tx, rows).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `action` by name.
    pub async fn put(&self, action: &Action) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put_action(&mut tx, self.db, action).await?;
        tx.commit().await?;
        Ok(())
    }
}
