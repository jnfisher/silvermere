//! Character profiles: `character_profiles`, the genbb `permrec` ("WCC PERMANENT USER")
//! written when a character's permanent standing changes and read back on entry and for the
//! status line.

use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::AccountId;
use silvermere_types::records::PermanentRecord;

use crate::Result;
use crate::pool::Db;
use crate::repo::{Args, Conn, Get, last_per_key, select_sql, upsert_many, upsert_sql};

const COLS: &[&str] = &[
    "account_id",
    "userid",
    "deathdate",
    "palette",
    "msglevel",
    "settings",
    "pagemax",
    "hairlen",
    "haircol",
    "eyecol",
    "statline",
    "fasttalk",
    "verbose_mode",
    "exp",
    "firstname",
    "lastname",
    "flags",
    "statfmt",
    "gossmax",
    "auctmax",
    "daybyte",
    "expbil",
    "exprem",
];

fn args(account: AccountId, p: &PermanentRecord) -> Args {
    let mut a = Args::new();
    a.add(account.0)
        .add(&p.userid)
        .add(p.deathdate)
        .add(p.palette)
        .add(p.msglevel.code())
        .add(p.settings.bits())
        .add(p.pagemax)
        .add(p.hairlen)
        .add(p.haircol)
        .add(p.eyecol)
        .add(p.statline.code())
        .add(p.fasttalk)
        .add(p.verbose)
        .add(p.exp)
        .add(&p.firstname)
        .add(&p.lastname)
        .add(p.flags.bits())
        .add(&p.statfmt)
        .add(p.gossmax)
        .add(p.auctmax)
        .add(p.daybyte)
        .add(p.expbil)
        .add(p.exprem);
    a
}

/// [`put`] for many accounts in multi-row statements (T-113). The accounts.
pub(crate) async fn put_many_of(
    conn: &mut Conn,
    rows: &[(AccountId, &PermanentRecord)],
) -> Result<()> {
    // a repeated key keeps its last record, as one upsert per write would
    let rows = last_per_key(rows, |(a, _)| a.0)
        .into_iter()
        .map(|(a, p)| args(*a, p))
        .collect();
    upsert_many(
        conn,
        "character_profiles",
        COLS,
        &["account_id"],
        rows,
        None,
    )
    .await?;
    Ok(())
}

/// Upserts the profile of `account`.
pub(crate) async fn put(conn: &mut Conn, account: AccountId, p: &PermanentRecord) -> Result<()> {
    args(account, p)
        .query(upsert_sql("character_profiles", COLS, &["account_id"]))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Character profiles.
#[derive(Debug, Clone, Copy)]
pub struct Profiles<'a> {
    db: &'a Db,
}

impl<'a> Profiles<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The profile of `account`.
    pub async fn get(&self, account: AccountId) -> Result<Option<PermanentRecord>> {
        let mut a = Args::new();
        a.add(account.0);
        let row = a
            .query(select_sql(
                "character_profiles",
                COLS,
                "WHERE account_id = $1",
            ))
            .fetch_optional(&self.db.pool())
            .await?;
        row.map(|r| {
            let g = Get::new(&r, "character_profiles");
            Ok(PermanentRecord {
                userid: g.text("userid")?,
                deathdate: g.int("deathdate")?,
                palette: g.int("palette")?,
                msglevel: g.code("msglevel")?,
                settings: PlayerSettings::from_bits_retain(g.int("settings")?),
                pagemax: g.int("pagemax")?,
                hairlen: g.int("hairlen")?,
                haircol: g.int("haircol")?,
                eyecol: g.int("eyecol")?,
                statline: g.code("statline")?,
                fasttalk: g.int("fasttalk")?,
                verbose: g.int("verbose_mode")?,
                exp: g.int("exp")?,
                firstname: g.text("firstname")?,
                lastname: g.text("lastname")?,
                flags: PlayerFlags::from_bits_retain(g.int("flags")?),
                statfmt: g.text("statfmt")?,
                gossmax: g.int("gossmax")?,
                auctmax: g.int("auctmax")?,
                daybyte: g.int("daybyte")?,
                expbil: g.int("expbil")?,
                exprem: g.int("exprem")?,
            })
        })
        .transpose()
    }

    /// Writes the profile of `account`.
    pub async fn put(&self, account: AccountId, p: &PermanentRecord) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put(&mut conn, account, p).await
    }

    /// Deletes the profile of `account`.
    pub async fn delete(&self, account: AccountId) -> Result<bool> {
        let mut a = Args::new();
        a.add(account.0);
        let done = a
            .query("DELETE FROM character_profiles WHERE account_id = $1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
