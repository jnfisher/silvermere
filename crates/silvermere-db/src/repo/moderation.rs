//! Moderation: `account_sanctions` (bans, jail, mutes), `ip_bans` and `realm_closed`. These
//! are written by the sysop's tools (the admin API, the CLI), never by the engine's
//! write-behind, so each call is awaited and takes effect at once.

use jiff::Timestamp;
use silvermere_types::ids::AccountId;

use crate::pool::Db;
use crate::repo::{Args, Get, select_sql, upsert_sql};
use crate::{DbError, Result};

/// What a sanction stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SanctionKind {
    /// No login.
    Ban,
    /// Held in the jail room.
    Jail,
    /// No gossip or auction.
    Mute,
}

impl SanctionKind {
    /// Every kind.
    pub const ALL: [SanctionKind; 3] = [Self::Ban, Self::Jail, Self::Mute];

    /// The `account_sanctions.kind` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ban => "ban",
            Self::Jail => "jail",
            Self::Mute => "mute",
        }
    }

    /// The kind a stored value names.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One `account_sanctions` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanction {
    /// The account.
    pub account: AccountId,
    /// What it stops.
    pub kind: SanctionKind,
    /// When it ends; `None` until lifted.
    pub until: Option<Timestamp>,
    /// Told to the player.
    pub reason: String,
    /// The sysop or token that set it.
    pub by_whom: String,
    /// When it was set.
    pub created: Timestamp,
}

impl Sanction {
    /// Whether it is in force at `now`.
    pub fn active_at(&self, now: Timestamp) -> bool {
        self.until.is_none_or(|u| u > now)
    }
}

/// One `ip_bans` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpBan {
    /// The ban id.
    pub id: i64,
    /// An address or CIDR block, canonical (`203.0.113.0/24`).
    pub network: String,
    /// When it ends; `None` until lifted.
    pub until: Option<Timestamp>,
    /// For the sysop.
    pub reason: String,
    /// The sysop or token that set it.
    pub by_whom: String,
    /// When it was set.
    pub created: Timestamp,
}

impl IpBan {
    /// Whether it is in force at `now`.
    pub fn active_at(&self, now: Timestamp) -> bool {
        self.until.is_none_or(|u| u > now)
    }
}

/// The `realm_closed` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmClosed {
    /// Told to whoever is refused.
    pub reason: String,
    /// The sysop or token that closed it.
    pub by_whom: String,
    /// When it was closed.
    pub created: Timestamp,
}

fn parse_ts(table: &'static str, column: &str, text: &str) -> Result<Timestamp> {
    text.parse().map_err(|e: jiff::Error| DbError::Decode {
        table,
        column: column.to_string(),
        reason: e.to_string(),
    })
}

fn opt_ts(table: &'static str, column: &str, text: Option<String>) -> Result<Option<Timestamp>> {
    text.map(|t| parse_ts(table, column, &t)).transpose()
}

const SANCTION_COLS: &[&str] = &[
    "account_id",
    "kind",
    "until",
    "reason",
    "by_whom",
    "created",
];

const IP_COLS: &[&str] = &["network", "until", "reason", "by_whom", "created"];

const REALM_COLS: &[&str] = &["id", "reason", "by_whom", "created"];

fn sanction_of(row: &sqlx::any::AnyRow) -> Result<Sanction> {
    let g = Get::new(row, "account_sanctions");
    let kind = g.text("kind")?;
    Ok(Sanction {
        account: AccountId(g.i64("account_id")?),
        kind: SanctionKind::parse(&kind).ok_or_else(|| DbError::Decode {
            table: "account_sanctions",
            column: "kind".into(),
            reason: format!("unknown kind {kind:?}"),
        })?,
        until: opt_ts("account_sanctions", "until", g.opt_text("until")?)?,
        reason: g.text("reason")?,
        by_whom: g.text("by_whom")?,
        created: parse_ts("account_sanctions", "created", &g.text("created")?)?,
    })
}

fn ip_ban_of(row: &sqlx::any::AnyRow) -> Result<IpBan> {
    let g = Get::new(row, "ip_bans");
    Ok(IpBan {
        id: g.i64("id")?,
        network: g.text("network")?,
        until: opt_ts("ip_bans", "until", g.opt_text("until")?)?,
        reason: g.text("reason")?,
        by_whom: g.text("by_whom")?,
        created: parse_ts("ip_bans", "created", &g.text("created")?)?,
    })
}

/// Bans, jail, mutes, IP bans and the closed realm.
#[derive(Debug, Clone, Copy)]
pub struct Moderation<'a> {
    db: &'a Db,
}

impl<'a> Moderation<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Puts `s` on its account, replacing one of the same kind.
    pub async fn set(&self, s: &Sanction) -> Result<()> {
        let mut a = Args::new();
        a.add(s.account.0)
            .add(s.kind.as_str())
            .add(s.until.map(|t| t.to_string()))
            .add(&s.reason)
            .add(&s.by_whom)
            .add(s.created.to_string());
        a.query(upsert_sql(
            "account_sanctions",
            SANCTION_COLS,
            &["account_id", "kind"],
        ))
        .execute(&self.db.pool())
        .await?;
        Ok(())
    }

    /// Lifts the account's sanction of `kind`; `false` when it had none.
    pub async fn lift(&self, account: AccountId, kind: SanctionKind) -> Result<bool> {
        let mut a = Args::new();
        a.add(account.0).add(kind.as_str());
        let done = a
            .query("DELETE FROM account_sanctions WHERE account_id = $1 AND kind = $2".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Lifts the account's sanction of `kind` only while it is still the one that ends at
    /// `until` (a newer one set meanwhile stays); `false` when nothing was deleted.
    pub async fn lift_if(
        &self,
        account: AccountId,
        kind: SanctionKind,
        until: Option<Timestamp>,
    ) -> Result<bool> {
        let mut a = Args::new();
        a.add(account.0).add(kind.as_str());
        let sql = match until {
            Some(u) => {
                a.add(u.to_string());
                "DELETE FROM account_sanctions WHERE account_id = $1 AND kind = $2 AND until = $3"
            }
            None => {
                "DELETE FROM account_sanctions WHERE account_id = $1 AND kind = $2 AND until IS NULL"
            }
        };
        let done = a.query(sql.to_string()).execute(&self.db.pool()).await?;
        Ok(done.rows_affected() > 0)
    }

    /// The account's sanctions, those over included, by kind.
    pub async fn of(&self, account: AccountId) -> Result<Vec<Sanction>> {
        let mut a = Args::new();
        a.add(account.0);
        a.query(select_sql(
            "account_sanctions",
            SANCTION_COLS,
            "WHERE account_id = $1 ORDER BY kind",
        ))
        .fetch_all(&self.db.pool())
        .await?
        .iter()
        .map(sanction_of)
        .collect()
    }

    /// Every sanction in force at `now`, by account then kind.
    pub async fn active(&self, now: Timestamp) -> Result<Vec<Sanction>> {
        let rows = Args::new()
            .query(select_sql(
                "account_sanctions",
                SANCTION_COLS,
                "ORDER BY account_id, kind",
            ))
            .fetch_all(&self.db.pool())
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let s = sanction_of(row)?;
            if s.active_at(now) {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// Bans `network` (canonical), replacing a ban of the same network; its id.
    pub async fn ban_ip(
        &self,
        network: &str,
        until: Option<Timestamp>,
        reason: &str,
        by_whom: &str,
        created: Timestamp,
    ) -> Result<i64> {
        let mut a = Args::new();
        a.add(network)
            .add(until.map(|t| t.to_string()))
            .add(reason)
            .add(by_whom)
            .add(created.to_string());
        let row = a
            .query(format!(
                "{} RETURNING id",
                upsert_sql("ip_bans", IP_COLS, &["network"])
            ))
            .fetch_one(&self.db.pool())
            .await?;
        Get::new(&row, "ip_bans").i64("id")
    }

    /// Lifts the ban of `network` (canonical); `false` when there was none.
    pub async fn unban_ip(&self, network: &str) -> Result<bool> {
        let mut a = Args::new();
        a.add(network);
        let done = a
            .query("DELETE FROM ip_bans WHERE network = $1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Every IP ban, those over included, by id.
    pub async fn ip_bans(&self) -> Result<Vec<IpBan>> {
        let mut cols = vec!["id"];
        cols.extend_from_slice(IP_COLS);
        Args::new()
            .query(select_sql("ip_bans", &cols, "ORDER BY id"))
            .fetch_all(&self.db.pool())
            .await?
            .iter()
            .map(ip_ban_of)
            .collect()
    }

    /// The closed realm's row; `None` while it is open.
    pub async fn realm_closed(&self) -> Result<Option<RealmClosed>> {
        let row = Args::new()
            .query(select_sql("realm_closed", REALM_COLS, "WHERE id = 1"))
            .fetch_optional(&self.db.pool())
            .await?;
        let Some(row) = row else { return Ok(None) };
        let g = Get::new(&row, "realm_closed");
        Ok(Some(RealmClosed {
            reason: g.text("reason")?,
            by_whom: g.text("by_whom")?,
            created: parse_ts("realm_closed", "created", &g.text("created")?)?,
        }))
    }

    /// Closes the realm (or replaces the reason of a closed one).
    pub async fn close_realm(&self, closed: &RealmClosed) -> Result<()> {
        let mut a = Args::new();
        a.add(1_i64)
            .add(&closed.reason)
            .add(&closed.by_whom)
            .add(closed.created.to_string());
        a.query(upsert_sql("realm_closed", REALM_COLS, &["id"]))
            .execute(&self.db.pool())
            .await?;
        Ok(())
    }

    /// Opens the realm; `false` when it was open.
    pub async fn open_realm(&self) -> Result<bool> {
        let done = Args::new()
            .query("DELETE FROM realm_closed WHERE id = 1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
