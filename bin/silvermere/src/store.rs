//! The engine's persistence requests and name checks, answered from `silvermere-db`.
//!
//! The engine never touches the database (`docs/engine.md`): it emits
//! `EngineOutput::Persist` and `EngineOutput::NameCheck`. The bridge forwards both here,
//! in order, and one task works through them:
//!
//! - `PersistRequest::Character` goes to the persistence worker as `Write::Character`,
//!   keyed by the account's character row (`characters_per_account` is 1); the first save
//!   of a new character inserts the row directly, since the worker only updates
//!   (`docs/engine/player-record.md`, "Persistence").
//! - `DeleteCharacter` flushes the worker, then deletes the row (`delete_player`).
//! - Each `Character` and `DeleteCharacter` is counted; once the job queue is empty the
//!   count goes to a task of its own (T-113) that flushes the worker and sends the count
//!   committed so far back as `EngineInput::Custom`
//!   running `player::on_characters_written`, which drops the engine's queued records
//!   the store now holds (T-088). After a failed character request nothing more is
//!   acknowledged, so the engine keeps its records until the next login takes them.
//! - `Profile` and `EvilPoints` go to the worker as `Write::Profile` and
//!   `Write::EvilPoints`.
//! - `Flush` flushes the worker.
//! - `Roster` (T-055) reads every character for the sysop's user checks
//!   and report; `Reload` hands SYSOP RELOAD's re-read game options to the engine.
//! - `NameCheck` is the new-character name check's walk of the character file:
//!   `characters.by_first_name`; the answer goes back
//!   as `EngineInput::Custom` running `validate_name::on_checked`.
//!
//! - `Gang` goes to the worker as `Write::Gang` (T-044). `GangShop` (a gang shop's slots
//!   and markup, which STOCK, UNSTOCK and MARKUP change) flushes the worker, then rewrites
//!   the shop with its stock.
//! - `GangQuery` reads the characters a gang command needs that are not in the game:
//!   the gang's members lowest experience first, or one by first name;
//!   the answer goes back as `EngineInput::Custom` running `gangs::on_query_answer`.
//!
//! - `DeleteGang` goes to the worker as `Write::GangDelete`. `CleanupScan` (the daily
//!   cleanup, T-056) reads every stored character with its account; the
//!   answer goes back as `EngineInput::Custom` running `cleanup::on_scan`.
//!
//! - `RoomState` (floor items, hidden items, coins: what changed in the room), `ItemOwnershipInsert`,
//!   `ItemOwnershipDelete` and `ItemActive` (limited items, T-030) go to the worker as the
//!   `Write` of the same name, write-behind.
//!
//! - `AccountDeleteScan` (`DELETE /v1/accounts/{id}`, T-069) reads the account and its
//!   character; the answer goes back as `EngineInput::Custom` running
//!   `session::on_account_scan`. `DeleteAccount` flushes the worker, then deletes the
//!   character and the account row (`silvermere_auth::cli::delete_account`, audited as
//!   `auth.account_deleted`).
//!
//! The reads of characters (`Roster`, `NameCheck`, `GangQuery`, `CleanupScan`,
//! `AccountDeleteScan`) do not flush the worker (T-113): they read the rows and lay the
//! queued character writes over them ([`Persist::pending_characters`]), which is what a
//! flush and the same read would give. They read only characters (and accounts, which
//! are written directly), so the room, monster and shop writes queued before them do not
//! change their answer, and over a slow database a name check does not wait for them.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;

use silvermere_db::Db;
use silvermere_db::persist::{Persist, Write};
use silvermere_db::repo::CharacterRow;
use silvermere_engine::gangs::{self, GangAnswer, GangQuery};
use silvermere_engine::session::{self, AccountScan};
use silvermere_engine::state::newchar::validate_name;
use silvermere_engine::{EngineInput, PersistRequest, SessionId};
use silvermere_types::ids::{AccountId, CharacterId};
use silvermere_types::records::Character;
use tokio::sync::mpsc as tmpsc;

/// One job for the store task, in the order the engine emitted it.
#[derive(Debug)]
pub enum Job {
    Persist(PersistRequest),
    NameCheck {
        id: SessionId,
        name: String,
    },
    /// Every stored character, for the sysop's user checks and report (T-055).
    Roster {
        id: SessionId,
    },
    /// SYSOP RELOAD: the game options read again (`None` when the file did not load).
    Reload {
        id: SessionId,
        game: Option<Box<silvermere_types::config::GameConfig>>,
    },
    GangQuery {
        id: SessionId,
        query: GangQuery,
    },
    CleanupScan,
    /// An account being deleted through the engine (T-069).
    AccountDeleteScan {
        account: AccountId,
    },
}

/// Runs until every sender of `jobs` is gone (the bridge stops at `EngineOutput::Stopped`),
/// then flushes the worker and returns.
pub async fn run(
    db: Arc<Db>,
    persist: Persist,
    worker: tokio::task::JoinHandle<()>,
    mut jobs: tmpsc::UnboundedReceiver<Job>,
    to_engine: mpsc::Sender<EngineInput>,
    flushed: tokio::sync::watch::Sender<u64>,
) {
    let mut rows: HashMap<AccountId, CharacterId> = HashMap::new();
    // T-088: character requests handled, the count acknowledged to the engine, and
    // whether acknowledging still holds (a failed one may have left its row stale)
    let (mut written, mut acked, mut acking) = (0u64, 0u64, true);
    // T-113: the acknowledgements wait for their flush on a task of their own, in order,
    // so the jobs behind them (a name check) do not wait behind the whole queue
    let (ack_tx, ack_rx) = tmpsc::unbounded_channel::<u64>();
    let acks = tokio::spawn(acknowledge(persist.clone(), ack_rx, to_engine.clone()));
    while let Some(job) = jobs.recv().await {
        let barrier = matches!(job, Job::Persist(PersistRequest::Flush));
        let character = matches!(
            job,
            Job::Persist(PersistRequest::Character { .. } | PersistRequest::DeleteCharacter { .. })
        );
        match handle(&db, &persist, &mut rows, &to_engine, job).await {
            Ok(()) => {
                written += u64::from(character);
                // Shutdown waits on this: everything queued before the barrier is committed.
                if barrier {
                    flushed.send_modify(|n| *n += 1);
                }
            }
            Err(e) => {
                tracing::error!(%e, "persistence request failed");
                if character {
                    acking = false;
                }
            }
        }
        if acking && written > acked && jobs.is_empty() && ack_tx.send(written).is_ok() {
            acked = written;
        }
    }
    drop(ack_tx);
    let _ = acks.await;
    if let Err(e) = persist.flush().await {
        tracing::error!(%e, "final persistence flush failed");
    }
    drop(persist);
    let _ = worker.await;
}

/// Acknowledges character requests to the engine (T-088) once the worker has committed
/// them: for each count received, in order, a flush, then `on_characters_written`. Counts
/// that queued up meanwhile are acknowledged together by the latest.
async fn acknowledge(
    persist: Persist,
    mut counts: tmpsc::UnboundedReceiver<u64>,
    to_engine: mpsc::Sender<EngineInput>,
) {
    while let Some(mut written) = counts.recv().await {
        while let Ok(later) = counts.try_recv() {
            written = later;
        }
        match persist.flush().await {
            Ok(()) => {
                let _ = to_engine.send(EngineInput::Custom {
                    session: None,
                    run: Box::new(move |w| {
                        silvermere_engine::player::on_characters_written(w, written);
                    }),
                });
            }
            Err(e) => {
                tracing::error!(%e, "persistence flush failed");
                return;
            }
        }
    }
}

/// Every stored character, as a flush and [`silvermere_db::repo::Characters::all`] would
/// read them: the queued character writes laid over the rows (T-113), so the read does
/// not wait behind the room and monster writes queued before it.
async fn all_characters(db: &Db, persist: &Persist) -> silvermere_db::Result<Vec<CharacterRow>> {
    let pending = persist.pending_characters();
    let mut rows = db.characters().all().await?;
    for r in &mut rows {
        if let Some(c) = pending.get(&r.id) {
            r.character = c.clone();
        }
    }
    Ok(rows)
}

/// The character whose first name is `name` (case-insensitive), as a flush and
/// `by_first_name` would read it (T-113): a queued character write holding the name
/// counts, and a stored row whose queued write changes its name does not.
pub async fn by_first_name(
    db: &Db,
    persist: &Persist,
    name: &str,
) -> silvermere_db::Result<Option<CharacterRow>> {
    let pending = persist.pending_characters();
    let named = |c: &Character| c.firstname.eq_ignore_ascii_case(name);
    if let Some((&id, c)) = pending.iter().find(|(_, c)| named(c))
        // the row exists: a character's first save inserts it directly
        && let Some(mut row) = db.characters().get(id).await?
    {
        row.character = c.clone();
        return Ok(Some(row));
    }
    let found = db.characters().by_first_name(name).await?;
    Ok(found.and_then(|mut r| match pending.get(&r.id) {
        // first names are unique, so no other row holds it
        Some(c) if !named(c) => None,
        Some(c) => {
            r.character = c.clone();
            Some(r)
        }
        None => Some(r),
    }))
}

/// The character of `account`, as a flush and `by_account` would read it (T-113).
async fn by_account(
    db: &Db,
    persist: &Persist,
    account: AccountId,
) -> silvermere_db::Result<Option<CharacterRow>> {
    let pending = persist.pending_characters();
    let mut found = db.characters().by_account(account).await?;
    if let Some(r) = &mut found
        && let Some(c) = pending.get(&r.id)
    {
        r.character = c.clone();
    }
    Ok(found)
}

async fn row_of(
    db: &Db,
    rows: &mut HashMap<AccountId, CharacterId>,
    account: AccountId,
) -> silvermere_db::Result<Option<CharacterId>> {
    if let Some(&id) = rows.get(&account) {
        return Ok(Some(id));
    }
    let found = db.characters().by_account(account).await?.map(|r| r.id);
    if let Some(id) = found {
        rows.insert(account, id);
    }
    Ok(found)
}

async fn handle(
    db: &Db,
    persist: &Persist,
    rows: &mut HashMap<AccountId, CharacterId>,
    to_engine: &mpsc::Sender<EngineInput>,
    job: Job,
) -> silvermere_db::Result<()> {
    match job {
        Job::Persist(PersistRequest::Character { account, character }) => {
            match row_of(db, rows, account).await? {
                Some(id) => persist.send(Write::Character { id, character }).await?,
                None => {
                    let id = db.characters().insert(account, &character).await?;
                    rows.insert(account, id);
                }
            }
        }
        Job::Persist(PersistRequest::DeleteCharacter { account }) => {
            persist.flush().await?;
            if let Some(id) = row_of(db, rows, account).await? {
                db.characters().delete(id).await?;
            }
            rows.remove(&account);
        }
        Job::Persist(PersistRequest::Profile { account, record }) => {
            persist
                .send(Write::Profile {
                    account,
                    record: *record,
                })
                .await?;
        }
        Job::Persist(PersistRequest::EvilPoints { account, record }) => {
            persist.send(Write::EvilPoints { account, record }).await?;
        }
        Job::Persist(PersistRequest::BankBook { account, book }) => {
            persist.send(Write::BankBook { account, book }).await?;
        }
        Job::Persist(PersistRequest::Flush) => persist.flush().await?,
        Job::Persist(PersistRequest::DeleteEvilPoints(a)) => {
            persist.send(Write::EvilPointsDelete(a)).await?;
        }
        Job::Persist(PersistRequest::DeleteProfile(a)) => {
            persist.flush().await?;
            db.profiles().delete(a).await?;
        }
        Job::Persist(PersistRequest::RoomState(state)) => {
            persist.send(Write::RoomState(room_state(*state))).await?;
        }
        Job::Persist(PersistRequest::ShopStock { shop, qty }) => {
            persist
                .send(Write::ShopStock(silvermere_db::repo::ShopStock {
                    number: shop,
                    qty,
                }))
                .await?;
        }
        Job::Persist(PersistRequest::ItemOwnershipInsert(own)) => {
            persist.send(Write::ItemOwnershipInsert(own)).await?;
        }
        Job::Persist(PersistRequest::ItemOwnershipDelete(id)) => {
            persist.send(Write::ItemOwnershipDelete(id)).await?;
        }
        Job::Persist(PersistRequest::ItemActive { item, active }) => {
            persist.send(Write::ItemActive { item, active }).await?;
        }
        // T-035: active monsters and the known monsters' live part
        Job::Persist(PersistRequest::ActiveMonster(m)) => {
            persist.send(Write::ActiveMonsterUpdate(*m)).await?;
        }
        Job::Persist(PersistRequest::ActiveMonsterDelete(n)) => {
            persist.send(Write::ActiveMonsterDelete(n)).await?;
        }
        Job::Persist(PersistRequest::MonsterState {
            number,
            curalive,
            killed_at,
        }) => {
            let state = silvermere_db::repo::MonsterState {
                number,
                curalive,
                killed_at,
            };
            persist.send(Write::MonsterState(state)).await?;
        }
        Job::Roster { id } => {
            let chars: Vec<_> = all_characters(db, persist)
                .await?
                .into_iter()
                .map(|r| (r.account, r.character))
                .collect();
            let _ = to_engine.send(EngineInput::Custom {
                session: Some(id),
                run: Box::new(move |w| {
                    silvermere_engine::state::sysop_menu::on_roster(w, id, chars);
                }),
            });
        }
        Job::Reload { id, game } => {
            let _ = to_engine.send(EngineInput::Custom {
                session: Some(id),
                run: Box::new(move |w| match game {
                    Some(g) => w.config = *g,
                    None => tracing::warn!("SYSOP RELOAD: the configuration did not load"),
                }),
            });
        }
        Job::Persist(PersistRequest::Gang(gang)) => {
            persist.send(Write::Gang(gang)).await?;
        }
        Job::Persist(PersistRequest::DeleteGang(key)) => {
            persist.send(Write::GangDelete(key)).await?;
        }
        Job::Persist(PersistRequest::GangShop(shop)) => {
            persist.flush().await?;
            db.shops()
                .put_many(&[*shop], silvermere_db::repo::StateWrite::Reset)
                .await?;
        }
        Job::GangQuery { id, query } => {
            let answer = match &query {
                GangQuery::Members { gang } => {
                    let mut rows: Vec<_> = all_characters(db, persist)
                        .await?
                        .into_iter()
                        .filter(|r| r.character.gang.eq_ignore_ascii_case(gang))
                        .collect();
                    rows.sort_by_key(|r| (r.character.exp, r.id));
                    GangAnswer::Members(rows.into_iter().map(|r| r.character).collect())
                }
                GangQuery::Offline { name, .. } => GangAnswer::Offline(
                    by_first_name(db, persist, name)
                        .await?
                        .map(|r| (r.account, Box::new(r.character))),
                ),
            };
            let _ = to_engine.send(EngineInput::Custom {
                session: Some(id),
                run: Box::new(move |w| gangs::on_query_answer(w, id, query, answer)),
            });
        }
        Job::CleanupScan => {
            let scan = async {
                let rows: Vec<_> = all_characters(db, persist)
                    .await?
                    .into_iter()
                    .map(|r| (r.account, r.character))
                    .collect();
                Ok::<_, silvermere_db::DbError>(rows)
            };
            match scan.await {
                Ok(rows) => {
                    let _ = to_engine.send(EngineInput::Custom {
                        session: None,
                        run: Box::new(move |w| {
                            silvermere_engine::cleanup::on_scan(w, rows);
                        }),
                    });
                }
                Err(e) => {
                    // the run must not wait for an answer that never comes
                    let _ = to_engine.send(EngineInput::Custom {
                        session: None,
                        run: Box::new(silvermere_engine::cleanup::on_scan_failed),
                    });
                    return Err(e);
                }
            }
        }
        Job::AccountDeleteScan { account } => {
            let scan = async {
                // accounts are written directly, never queued
                if db.accounts().get(account).await?.is_none() {
                    return Ok(AccountScan::NoAccount);
                }
                let stored = by_account(db, persist, account).await?;
                Ok::<_, silvermere_db::DbError>(AccountScan::Found(
                    stored.map(|r| Box::new(r.character)),
                ))
            };
            let (answer, failed) = match scan.await {
                Ok(a) => (a, None),
                Err(e) => (AccountScan::Failed(e.to_string()), Some(e)),
            };
            // the request must not wait for an answer that never comes
            let _ = to_engine.send(EngineInput::Custom {
                session: None,
                run: Box::new(move |w| session::on_account_scan(w, account, answer)),
            });
            if let Some(e) = failed {
                return Err(e);
            }
        }
        Job::Persist(PersistRequest::DeleteAccount(account)) => {
            persist.flush().await?;
            rows.remove(&account);
            if let Err(e) = silvermere_auth::cli::delete_account(db, account).await {
                tracing::error!(account = account.0, %e, "account row not deleted");
            }
        }
        Job::NameCheck { id, name } => {
            let taken = by_first_name(db, persist, &name).await?.is_some();
            let _ = to_engine.send(EngineInput::Custom {
                session: Some(id),
                run: Box::new(move |w| validate_name::on_checked(w, id, taken)),
            });
        }
    }
    Ok(())
}

/// The engine's live room state as the repository's row (the same fields).
fn room_state(s: silvermere_engine::RoomState) -> silvermere_db::repo::RoomState {
    silvermere_db::repo::RoomState {
        key: s.key,
        coins: s.coins,
        hidcoins: s.hidcoins,
        owner: s.owner,
        specalive: s.specalive,
        foundmask: s.foundmask,
        lastkill: s.lastkill,
        areacount: s.areacount,
        searchers: s.searchers,
        nummonsters: s.nummonsters,
        items: s.items,
        hiditems: s.hiditems,
        monsters: s.monsters,
    }
}
