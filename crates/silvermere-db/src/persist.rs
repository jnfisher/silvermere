//! The write-behind persistence worker.
//!
//! The engine holds the world in memory and sends typed [`Write`]s here; the worker
//! batches up to `batch_max` writes or `batch_window` of them into one transaction. The
//! moments the original saved (a character on level, death, logoff and in the 60 s sweep
//! of everyone online; rooms, shops, gangs and bank books as they change) become queued
//! writes; account and
//! character-creation writes stay direct `async fn`s on the repositories.
//!
//! A batch that fails is retried with backoff and [`PersistStatus`] flips to `Unhealthy`
//! until it commits (the engine stops logins
//! meanwhile); no write is dropped for a connectivity error. A write the database refuses
//! outright (a constraint violation, a row that does not exist) is logged and skipped so
//! one bad write cannot wedge the queue; [`Persist::dropped`] counts them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silvermere_types::ids::{AccountId, ActiveMonsterId, CharacterId, ItemNo, ShopNo};
use silvermere_types::records::{
    ActiveMonster, BankBook, Character, EvilRecord, Gang, ItemOwnership, PermanentRecord,
};
use sqlx::Connection as _;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::events::Event;
use crate::pool::Db;
use crate::repo::{self, Conn, MonsterState, RoomState, ShopStock};
use crate::{DbError, Result};
use silvermere_engine::metrics::{DB_LATENCY_SECONDS, PERSIST_ERRORS_TOTAL, PERSIST_QUEUE_DEPTH};

/// One queued write: a typed row, never SQL.
#[derive(Debug, Clone)]
pub enum Write {
    /// The live part of a room.
    RoomState(RoomState),
    /// The live stock of a shop.
    ShopStock(ShopStock),
    /// The kill time and alive count of a known monster.
    MonsterState(MonsterState),
    /// The copies-in-play count of a limited item.
    ItemActive {
        /// The item.
        item: ItemNo,
        /// Copies in play.
        active: u16,
    },
    /// A generated active monster (dinsbtv).
    ActiveMonsterInsert(ActiveMonster),
    /// An active monster's save (dupdbtv; an upsert).
    ActiveMonsterUpdate(ActiveMonster),
    /// A removed active monster (delbtv).
    ActiveMonsterDelete(ActiveMonsterId),
    /// A character save (`save_player`).
    Character {
        /// The character.
        id: CharacterId,
        /// Its record.
        character: Box<Character>,
    },
    /// A profile save.
    Profile {
        /// The account.
        account: AccountId,
        /// The record.
        record: PermanentRecord,
    },
    /// An evil-points save (`save_evil_points`).
    EvilPoints {
        /// The account.
        account: AccountId,
        /// The record.
        record: EvilRecord,
    },
    /// Removed evil points.
    EvilPointsDelete(AccountId),
    /// A bank book save.
    BankBook {
        /// The account.
        account: AccountId,
        /// The book.
        book: BankBook,
    },
    /// A closed bank book.
    BankBookDelete {
        /// The account.
        account: AccountId,
        /// The bank.
        bank: ShopNo,
    },
    /// A gang save (insert when new).
    Gang(Gang),
    /// A disbanded gang, by key.
    GangDelete(String),
    /// A new limited-item ownership record.
    ItemOwnershipInsert(ItemOwnership),
    /// A removed limited-item ownership record, by id.
    ItemOwnershipDelete(i32),
    /// A game event.
    Event(Event),
    /// Raw SQL, for the tests of refusals no typed write can provoke.
    #[cfg(test)]
    Raw(&'static str),
}

/// How the same row is identified across writes, for coalescing within a batch and for
/// keeping the order of the writes to one row when a batch is grouped by kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Ownership(i32),
    Room(silvermere_types::ids::RoomKey),
    Shop(ShopNo),
    Monster(silvermere_types::ids::MonsterNo),
    Item(ItemNo),
    ActiveMonster(ActiveMonsterId),
    Character(CharacterId),
    Profile(AccountId),
    Evil(AccountId),
    Bank(AccountId, ShopNo),
    Gang(String),
}

impl Write {
    /// The key of a full-row overwrite: a later write with the same key makes an earlier
    /// one in the same batch redundant. Inserts, deletes and events have none.
    fn overwrite_key(&self) -> Option<Key> {
        match self {
            Write::RoomState(s) => Some(Key::Room(s.key)),
            Write::ShopStock(s) => Some(Key::Shop(s.number)),
            Write::MonsterState(s) => Some(Key::Monster(s.number)),
            Write::ItemActive { item, .. } => Some(Key::Item(*item)),
            Write::ActiveMonsterUpdate(m) => Some(Key::ActiveMonster(m.number)),
            Write::Character { id, .. } => Some(Key::Character(*id)),
            Write::Profile { account, .. } => Some(Key::Profile(*account)),
            Write::EvilPoints { account, .. } => Some(Key::Evil(*account)),
            Write::BankBook { account, book } => Some(Key::Bank(*account, book.bank)),
            Write::Gang(g) => Some(Key::Gang(g.key.to_lowercase())),
            _ => None,
        }
    }

    /// The row this write touches. Two writes to the same row keep their order when a
    /// batch is grouped by kind ([`apply_batch`]); writes to different rows commute,
    /// since no table one write touches references a row another kind writes. `None` for
    /// an event (a new row each).
    fn row_key(&self) -> Option<Key> {
        match self {
            Write::ActiveMonsterInsert(m) => Some(Key::ActiveMonster(m.number)),
            Write::ActiveMonsterDelete(n) => Some(Key::ActiveMonster(*n)),
            Write::EvilPointsDelete(a) => Some(Key::Evil(*a)),
            Write::BankBookDelete { account, bank } => Some(Key::Bank(*account, *bank)),
            Write::GangDelete(k) => Some(Key::Gang(k.to_lowercase())),
            Write::ItemOwnershipInsert(o) => Some(Key::Ownership(o.id)),
            Write::ItemOwnershipDelete(id) => Some(Key::Ownership(*id)),
            _ => self.overwrite_key(),
        }
    }

    /// Which [`KINDS`] entry this write counts under.
    fn kind(&self) -> usize {
        match self {
            Write::Character { .. } => 0,
            Write::Profile { .. } => 1,
            Write::EvilPoints { .. } | Write::EvilPointsDelete(_) => 2,
            Write::BankBook { .. } | Write::BankBookDelete { .. } => 3,
            Write::ItemOwnershipInsert(_) | Write::ItemOwnershipDelete(_) => 4,
            Write::RoomState(_) => 5,
            Write::ShopStock(_) => 6,
            Write::MonsterState(_) => 7,
            Write::ItemActive { .. } => 8,
            Write::ActiveMonsterInsert(_)
            | Write::ActiveMonsterUpdate(_)
            | Write::ActiveMonsterDelete(_) => 9,
            Write::Gang(_) | Write::GangDelete(_) => 10,
            Write::Event(_) => 11,
            #[cfg(test)]
            Write::Raw(_) => 11,
        }
    }

    /// The `op` label of `silvermere_db_latency_seconds`.
    fn op(&self) -> &'static str {
        match self {
            Write::Character { .. } => "save_character",
            Write::Event(_) => "event_insert",
            _ => "save_state",
        }
    }

    fn describe(&self) -> String {
        match self {
            Write::RoomState(s) => format!("room_state {}/{}", s.key.map, s.key.room),
            Write::ShopStock(s) => format!("shop_stock {}", s.number),
            Write::MonsterState(s) => format!("monster_state {}", s.number),
            Write::ItemActive { item, .. } => format!("item_active {item}"),
            Write::ActiveMonsterInsert(m) => format!("active_monster insert {}", m.number),
            Write::ActiveMonsterUpdate(m) => format!("active_monster update {}", m.number),
            Write::ActiveMonsterDelete(n) => format!("active_monster delete {n}"),
            Write::Character { id, .. } => format!("character {id}"),
            Write::Profile { account, .. } => format!("profile {account}"),
            Write::EvilPoints { account, .. } => format!("evil_points {account}"),
            Write::EvilPointsDelete(a) => format!("evil_points delete {a}"),
            Write::BankBook { account, book } => format!("bank_book {account}/{}", book.bank),
            Write::BankBookDelete { account, bank } => format!("bank_book delete {account}/{bank}"),
            Write::Gang(g) => format!("gang {}", g.key),
            Write::GangDelete(k) => format!("gang delete {k}"),
            Write::ItemOwnershipInsert(o) => format!("item_ownership insert {}", o.id),
            Write::ItemOwnershipDelete(id) => format!("item_ownership delete {id}"),
            Write::Event(e) => format!("event {}", e.kind),
            #[cfg(test)]
            Write::Raw(sql) => format!("raw {sql}"),
        }
    }
}

/// Applies one write on `conn`.
async fn apply(conn: &mut Conn, db: &Db, write: &Write) -> Result<()> {
    match write {
        Write::RoomState(s) => repo::rooms::put_state(conn, s).await,
        Write::ShopStock(s) => repo::shops::put_stock(conn, s).await,
        Write::MonsterState(s) => repo::monsters::put_state(conn, s).await,
        Write::ItemActive { item, active } => repo::items::set_active(conn, *item, *active).await,
        Write::ActiveMonsterInsert(m) => repo::active_monsters::insert(conn, m).await,
        Write::ActiveMonsterUpdate(m) => repo::active_monsters::update(conn, m).await,
        Write::ActiveMonsterDelete(n) => repo::active_monsters::delete(conn, *n).await.map(|_| ()),
        Write::Character { id, character } => {
            if repo::characters::update(conn, *id, character).await? {
                Ok(())
            } else {
                Err(DbError::Sqlx(sqlx::Error::RowNotFound))
            }
        }
        Write::Profile { account, record } => repo::profiles::put(conn, *account, record).await,
        Write::EvilPoints { account, record } => {
            repo::evil_points::put(conn, *account, record).await
        }
        Write::EvilPointsDelete(a) => repo::evil_points::delete(conn, *a).await.map(|_| ()),
        Write::BankBook { account, book } => repo::bank_books::put(conn, *account, book).await,
        Write::BankBookDelete { account, bank } => repo::bank_books::delete(conn, *account, *bank)
            .await
            .map(|_| ()),
        Write::Gang(g) => repo::gangs::update(conn, db, g).await,
        Write::GangDelete(k) => repo::gangs::delete(conn, db, k).await.map(|_| ()),
        Write::ItemOwnershipInsert(o) => repo::item_ownership::insert(conn, o).await,
        Write::ItemOwnershipDelete(id) => repo::item_ownership::delete(conn, *id).await.map(|_| ()),
        Write::Event(e) => crate::events::append(conn, e).await.map(|_| ()),
        #[cfg(test)]
        Write::Raw(sql) => {
            use sqlx::Executor as _;
            conn.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(*sql)))
                .await
                .map(|_| ())
                .map_err(Into::into)
        }
    }
}

/// Applies `writes` on `conn` in a few statements per table rather than a few per write
/// (T-113): over a network every statement costs a round trip.
///
/// The batch is cut into stages: the n-th write to a row goes in stage n, so the writes
/// to one row keep their order (a delete before its re-insert) while the rest are free to
/// group. Each stage then writes kind by kind, each kind in multi-row statements; events
/// are all in the first stage, in queue order. What is stored after the batch is what
/// applying the writes one by one would store (`tests/persist_batch.rs`).
async fn apply_batch(conn: &mut Conn, db: &Db, writes: &[Write]) -> Result<()> {
    #[cfg(test)]
    if writes.iter().any(|w| matches!(w, Write::Raw(_))) {
        for w in writes {
            apply(conn, db, w).await?;
        }
        return Ok(());
    }
    let mut seen: HashMap<Key, usize> = HashMap::new();
    let mut stages: Vec<Vec<&Write>> = Vec::new();
    for w in writes {
        let stage = match w.row_key() {
            Some(k) => {
                let n = seen.entry(k).or_insert(0);
                *n += 1;
                *n - 1
            }
            None => 0,
        };
        if stages.len() <= stage {
            stages.resize_with(stage + 1, Vec::new);
        }
        stages[stage].push(w);
    }
    for stage in &stages {
        apply_stage(conn, db, stage).await?;
    }
    Ok(())
}

/// Writes of different rows ([`apply_batch`]), kind by kind.
async fn apply_stage(conn: &mut Conn, db: &Db, writes: &[&Write]) -> Result<()> {
    macro_rules! pick {
        ($pat:pat => $out:expr) => {
            writes
                .iter()
                .filter_map(|w| match w {
                    $pat => Some($out),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
    }
    async fn timed<F: std::future::Future<Output = Result<()>>>(
        op: &'static str,
        n: usize,
        f: F,
    ) -> Result<()> {
        if n == 0 {
            return Ok(());
        }
        let started = std::time::Instant::now();
        f.await?;
        metrics::histogram!(DB_LATENCY_SECONDS, "op" => op).record(started.elapsed());
        Ok(())
    }
    let rooms = pick!(Write::RoomState(s) => s);
    timed(
        "save_state",
        rooms.len(),
        repo::rooms::put_states(conn, &rooms),
    )
    .await?;
    let shops = pick!(Write::ShopStock(s) => s);
    timed(
        "save_state",
        shops.len(),
        repo::shops::put_stocks(conn, &shops),
    )
    .await?;
    let monsters = pick!(Write::MonsterState(s) => s);
    let n = monsters.len();
    timed("save_state", n, repo::monsters::put_states(conn, &monsters)).await?;
    let items = pick!(Write::ItemActive { item, active } => (*item, *active));
    let n = items.len();
    timed("save_state", n, repo::items::set_active_many(conn, &items)).await?;
    let deleted = pick!(Write::ActiveMonsterDelete(n) => *n);
    let n = deleted.len();
    timed(
        "save_state",
        n,
        repo::active_monsters::delete_many(conn, &deleted),
    )
    .await?;
    let inserted = pick!(Write::ActiveMonsterInsert(m) => m);
    let n = inserted.len();
    timed(
        "save_state",
        n,
        repo::active_monsters::insert_many_of(conn, &inserted),
    )
    .await?;
    let updated = pick!(Write::ActiveMonsterUpdate(m) => m);
    let n = updated.len();
    timed(
        "save_state",
        n,
        repo::active_monsters::update_many(conn, &updated),
    )
    .await?;
    let chars = pick!(Write::Character { id, character } => (*id, &**character));
    let n = chars.len();
    timed(
        "save_character",
        n,
        repo::characters::update_many(conn, &chars),
    )
    .await?;
    let profiles = pick!(Write::Profile { account, record } => (*account, record));
    let n = profiles.len();
    timed(
        "save_state",
        n,
        repo::profiles::put_many_of(conn, &profiles),
    )
    .await?;
    let evil = pick!(Write::EvilPoints { account, record } => (*account, record));
    let n = evil.len();
    timed("save_state", n, repo::evil_points::put_many_of(conn, &evil)).await?;
    let evil_gone = pick!(Write::EvilPointsDelete(a) => *a);
    let n = evil_gone.len();
    timed(
        "save_state",
        n,
        repo::evil_points::delete_many(conn, &evil_gone),
    )
    .await?;
    let books = pick!(Write::BankBook { account, book } => (*account, book));
    let n = books.len();
    timed("save_state", n, repo::bank_books::put_many_of(conn, &books)).await?;
    let owned = pick!(Write::ItemOwnershipInsert(o) => o);
    let n = owned.len();
    timed(
        "save_state",
        n,
        repo::item_ownership::insert_many_of(conn, &owned),
    )
    .await?;
    let disowned = pick!(Write::ItemOwnershipDelete(id) => *id);
    let n = disowned.len();
    timed(
        "save_state",
        n,
        repo::item_ownership::delete_many(conn, &disowned),
    )
    .await?;
    let events = pick!(Write::Event(e) => e);
    let n = events.len();
    timed("event_insert", n, crate::events::append_many(conn, &events)).await?;
    // rare (a closed book, a gang save or disband): one at a time
    for w in writes {
        if matches!(
            w,
            Write::BankBookDelete { .. } | Write::Gang(_) | Write::GangDelete(_)
        ) {
            let started = std::time::Instant::now();
            apply(conn, db, w).await?;
            metrics::histogram!(DB_LATENCY_SECONDS, "op" => w.op()).record(started.elapsed());
        }
    }
    Ok(())
}

/// Whether the worker is keeping up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistStatus {
    /// The last batch committed.
    Healthy,
    /// Batches are failing and being retried.
    Unhealthy {
        /// Consecutive failed attempts.
        failures: u32,
        /// The last error.
        error: String,
    },
}

/// Worker tuning.
#[derive(Debug, Clone)]
pub struct PersistConfig {
    /// Most writes in one transaction.
    pub batch_max: usize,
    /// Longest wait for more writes before committing a batch.
    pub batch_window: Duration,
    /// First retry delay after a failed batch; doubled each failure.
    pub retry_min: Duration,
    /// Longest retry delay.
    pub retry_max: Duration,
    /// Queue capacity; `send` waits when it is full.
    pub queue: usize,
    /// A queue deeper than this for [`Self::backlog_after`] is logged at WARN with its
    /// depth and drain rate (T-113): a database too slow or too far away for the load.
    pub backlog_warn: usize,
    /// How long the queue must stay above [`Self::backlog_warn`] before the WARN, and the
    /// least time between two of them.
    pub backlog_after: Duration,
    /// Tests only: apply each write by itself with the per-row writers (the path before
    /// T-113), the reference the batched path is compared against (`tests/persist_batch.rs`).
    #[doc(hidden)]
    pub per_row_reference: bool,
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self {
            batch_max: 2048,
            batch_window: Duration::from_millis(50),
            retry_min: Duration::from_millis(100),
            retry_max: Duration::from_secs(5),
            queue: 65_536,
            backlog_warn: 1_000,
            backlog_after: Duration::from_secs(30),
            per_row_reference: false,
        }
    }
}

enum Msg {
    Write(Box<Write>),
    Flush(oneshot::Sender<()>),
}

/// The kinds of write, as the unsaved count names them, players' records first (a
/// character holds items whose ownership records are kept beside it).
pub const KINDS: [&str; 12] = [
    "character",
    "profile",
    "evil_points",
    "bank_book",
    "item_ownership",
    "room_state",
    "shop_stock",
    "monster_state",
    "item_active",
    "active_monster",
    "gang",
    "event",
];

/// Writes queued and not yet saved, per kind, and writes saved so far: what a shutdown
/// that cannot wait any longer reports (T-113). Shared by the handles and the worker.
#[derive(Debug, Default)]
struct Counts {
    unsaved: [AtomicU64; KINDS.len()],
    saved: AtomicU64,
}

impl Counts {
    /// `write` is done with: committed, refused, or overwritten by a later write.
    fn done(&self, write: &Write) {
        self.unsaved[write.kind()].fetch_sub(1, Ordering::Relaxed);
    }

    /// `n` writes reached the database: committed or refused. Only this is progress; a
    /// write overwritten in the queue is not.
    fn saved(&self, n: usize) {
        self.saved.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// The worker's progress, readable without keeping the worker alive (a [`Persist`]
/// handle does): for a shutdown deciding whether to keep waiting (T-113).
#[derive(Debug, Clone)]
pub struct Progress(Arc<Counts>);

impl Progress {
    /// Writes the database has committed or refused so far: what a shutdown counts as
    /// progress.
    pub fn saved(&self) -> u64 {
        self.0.saved.load(Ordering::Relaxed)
    }

    /// Writes queued and not yet done with, per kind with any ([`KINDS`] order).
    pub fn unsaved(&self) -> Vec<(&'static str, u64)> {
        KINDS
            .iter()
            .zip(&self.0.unsaved)
            .map(|(k, n)| (*k, n.load(Ordering::Relaxed)))
            .filter(|(_, n)| *n > 0)
            .collect()
    }
}

/// The character writes queued and not yet committed (or refused), latest per character:
/// what a read of the characters must see without waiting for the whole queue (T-113).
type Pending = Arc<Mutex<HashMap<CharacterId, Character>>>;

/// A handle to the worker: queue writes, await a flush, watch the status. Cloneable;
/// the worker exits once every handle is dropped and the queue is drained.
#[derive(Debug, Clone)]
pub struct Persist {
    tx: mpsc::Sender<Msg>,
    status: watch::Receiver<PersistStatus>,
    dropped: Arc<AtomicU64>,
    pending: Pending,
    counts: Arc<Counts>,
}

impl Persist {
    /// Starts the worker on `db`.
    pub fn spawn(db: Arc<Db>, config: PersistConfig) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(config.queue.max(1));
        let (status_tx, status_rx) = watch::channel(PersistStatus::Healthy);
        let dropped = Arc::new(AtomicU64::new(0));
        let pending = Pending::default();
        let counts = Arc::new(Counts::default());
        let worker = Worker {
            db,
            config,
            rx,
            status: status_tx,
            dropped: Arc::clone(&dropped),
            pending: Arc::clone(&pending),
            counts: Arc::clone(&counts),
        };
        let handle = tokio::spawn(worker.run());
        (
            Self {
                tx,
                status: status_rx,
                dropped,
                pending,
                counts,
            },
            handle,
        )
    }

    /// Queues `write`; waits while the queue is full.
    pub async fn send(&self, write: Write) -> Result<()> {
        let character = match &write {
            Write::Character { id, character } => {
                lock(&self.pending).insert(*id, (**character).clone());
                Some(*id)
            }
            _ => None,
        };
        let kind = write.kind();
        self.counts.unsaved[kind].fetch_add(1, Ordering::Relaxed);
        if self.tx.send(Msg::Write(Box::new(write))).await.is_err() {
            self.counts.unsaved[kind].fetch_sub(1, Ordering::Relaxed);
            // the worker is gone: no read may count on this write
            if let Some(id) = character {
                lock(&self.pending).remove(&id);
            }
            return Err(DbError::PersistClosed);
        }
        self.record_depth();
        Ok(())
    }

    fn record_depth(&self) {
        metrics::gauge!(PERSIST_QUEUE_DEPTH).set(self.queued() as f64);
    }

    /// The worker's progress, without a handle that keeps it running.
    pub fn progress(&self) -> Progress {
        Progress(Arc::clone(&self.counts))
    }

    /// Returns once every write queued before the call has been committed.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Msg::Flush(tx))
            .await
            .map_err(|_| DbError::PersistClosed)?;
        rx.await.map_err(|_| DbError::PersistClosed)
    }

    /// The character writes queued and not yet committed, latest per character (T-113).
    ///
    /// A read of the characters that must see every queued character write takes this,
    /// then reads, and lays this over what it read (a character here replaces its row):
    /// that is what a [`Self::flush`] and the same read would give, without waiting behind
    /// the rest of the queue. Taken before the read, a write committed meanwhile is in
    /// both, the same record. Only character writes are covered; a read of any other
    /// table still needs a flush.
    pub fn pending_characters(&self) -> HashMap<CharacterId, Character> {
        lock(&self.pending).clone()
    }

    /// Writes queued and not yet taken by the worker.
    pub fn queued(&self) -> usize {
        self.tx.max_capacity() - self.tx.capacity()
    }

    /// The current status.
    pub fn status(&self) -> PersistStatus {
        self.status.borrow().clone()
    }

    /// A receiver that wakes on every status change.
    pub fn watch(&self) -> watch::Receiver<PersistStatus> {
        self.status.clone()
    }

    /// How many writes the database refused and the worker skipped.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The lock of [`Pending`]; a panic while it was held leaves a map that is still whole.
fn lock(p: &Pending) -> std::sync::MutexGuard<'_, HashMap<CharacterId, Character>> {
    p.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Worker {
    db: Arc<Db>,
    config: PersistConfig,
    rx: mpsc::Receiver<Msg>,
    status: watch::Sender<PersistStatus>,
    dropped: Arc<AtomicU64>,
    pending: Pending,
    counts: Arc<Counts>,
}

/// The WARN for a queue that stays deep (T-113).
struct Backlog {
    /// When the queue went above the threshold, and the writes saved by then.
    since: Option<(Instant, u64)>,
    /// The last WARN.
    warned: Option<Instant>,
}

impl Backlog {
    fn new() -> Self {
        Self {
            since: None,
            warned: None,
        }
    }

    /// Notes `depth` writes not yet saved and `saved` saved so far ([`Counts`]); the drain
    /// rate (writes per second) when it logged the WARN.
    fn check(&mut self, config: &PersistConfig, saved: u64, depth: u64) -> Option<f64> {
        let now = Instant::now();
        if depth <= config.backlog_warn as u64 {
            self.since = None;
            return None;
        }
        let (since, at_start) = *self.since.get_or_insert((now, saved));
        let held = now - since;
        let quiet = self.warned.is_none_or(|w| now - w >= config.backlog_after);
        if held >= config.backlog_after && quiet {
            let rate = saved.saturating_sub(at_start) as f64 / held.as_secs_f64();
            tracing::warn!(
                depth,
                drain_per_s = format!("{rate:.0}"),
                secs = held.as_secs(),
                "persistence queue backlog: the database is not keeping up \
                 (silvermere_persist_queue_depth; see docs/install.md, hosted PostgreSQL)"
            );
            self.warned = Some(now);
            return Some(rate);
        }
        None
    }

    /// Checks every `backlog_after / 6` while the worker runs, so a worker held in one
    /// long batch (an outage, a slow commit) is warned about too, not only between
    /// batches.
    async fn watch(counts: Arc<Counts>, config: PersistConfig) {
        let mut backlog = Self::new();
        let mut tick =
            tokio::time::interval((config.backlog_after / 6).max(Duration::from_millis(100)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let depth = counts
                .unsaved
                .iter()
                .map(|n| n.load(Ordering::Relaxed))
                .sum();
            let _ = backlog.check(&config, counts.saved.load(Ordering::Relaxed), depth);
        }
    }
}

/// A refusal by the database itself (constraint, missing row, bad value) or an error that
/// no retry can change (a value that does not encode or decode, bad configuration), as
/// opposed to a connectivity failure or a transient condition: retrying would fail the
/// same way, and one such write must not block the ones behind it.
fn is_permanent(err: &DbError) -> bool {
    match err {
        DbError::Sqlx(sqlx::Error::Database(e)) => !is_transient(e.as_ref()),
        DbError::Duplicate(_)
        | DbError::Decode { .. }
        | DbError::Sqlx(
            sqlx::Error::RowNotFound
            | sqlx::Error::Encode(_)
            | sqlx::Error::Decode(_)
            | sqlx::Error::ColumnDecode { .. }
            | sqlx::Error::Configuration(_),
        ) => true,
        _ => false,
    }
}

/// The kind and code of `err` for the drop log: what the database called a write it
/// refused, so a good write classed as permanent is named by its code.
fn error_class(err: &DbError) -> (String, String) {
    match err {
        DbError::Sqlx(sqlx::Error::Database(e)) => (
            format!("{:?}", e.kind()),
            e.code().map(|c| c.into_owned()).unwrap_or_default(),
        ),
        DbError::Sqlx(e) => (format!("{e:?}").chars().take(40).collect(), String::new()),
        other => (
            format!("{other:?}").chars().take(40).collect(),
            String::new(),
        ),
    }
}

/// Why a commit failed.
enum CommitError {
    /// No connection to run on, or it would not begin a transaction. That is never about
    /// the write (a rotated password, a vanished database directory, an exhausted pool),
    /// so however the error is classed, the batch waits for the database and is retried.
    Outage(DbError),
    /// A write or the commit itself failed: [`is_permanent`] tells a refusal from an outage.
    Write(DbError),
}

impl CommitError {
    fn err(&self) -> &DbError {
        match self {
            CommitError::Outage(e) | CommitError::Write(e) => e,
        }
    }

    /// Whether the database refused the write, so retrying would fail the same way.
    fn is_refusal(&self) -> bool {
        matches!(self, CommitError::Write(e) if is_permanent(e))
    }

    /// Whether the connection can go back to the pool: a constraint refusal (SQLite
    /// class 19, PostgreSQL class 23) or a refusal raised outside the database (a missing
    /// row, a value that does not encode). Those end one statement and leave the
    /// transaction to sqlx's rollback. Any other database error may have rolled the whole
    /// transaction back already (SQLite CORRUPT, INTERRUPT, ABORT_ROLLBACK), leaving
    /// sqlx's transaction depth out of step with the connection.
    fn keeps_connection(&self) -> bool {
        match self {
            CommitError::Write(DbError::Sqlx(sqlx::Error::Database(e))) => {
                is_constraint(e.as_ref())
            }
            other => other.is_refusal(),
        }
    }
}

/// A constraint violation: SQLite primary code 19 (extended codes carry it in the low
/// byte), PostgreSQL SQLSTATE class 23.
fn is_constraint(e: &dyn sqlx::error::DatabaseError) -> bool {
    let Some(code) = e.code() else {
        return false;
    };
    if code.len() <= 4 {
        code.parse::<u32>().is_ok_and(|n| n & 0xff == 19)
    } else {
        code.starts_with("23")
    }
}

/// A database error that says "try again" or "try again later", not "no": contention
/// (SQLite BUSY/LOCKED, PostgreSQL serialization failure, deadlock, `lock_not_available`,
/// `query_canceled`) and an outage of the database or its host (connection exceptions,
/// operator shutdown, disk full, out of memory, SQLite I/O errors). These turn the status
/// Unhealthy and are retried with backoff; none drops a write.
///
/// `query_canceled` is also what a `statement_timeout` raises, so a write that always runs
/// past a role-level timeout is retried forever rather than dropped: the status stays
/// Unhealthy, which is the right trade against losing a write that only met a cancel.
fn is_transient(e: &dyn sqlx::error::DatabaseError) -> bool {
    let Some(code) = e.code() else {
        return false;
    };
    // SQLite reports its result code as a number of at most four digits (extended codes
    // carry the primary code in the low byte); PostgreSQL's SQLSTATE is five characters.
    if code.len() <= 4 {
        return code.parse::<u32>().is_ok_and(|n| {
            matches!(
                n & 0xff,
                5 | 6   // SQLITE_BUSY, SQLITE_LOCKED and their extended codes
                | 7     // SQLITE_NOMEM
                | 10    // SQLITE_IOERR*
                | 13    // SQLITE_FULL
                | 15 // SQLITE_PROTOCOL
            )
        });
    }
    matches!(
        code.as_ref(),
        // serialization_failure, deadlock_detected, lock_not_available, query_canceled
        "40001" | "40P01" | "55P03" | "57014"
            // admin_shutdown, crash_shutdown, cannot_connect_now
            | "57P01" | "57P02" | "57P03"
            // disk_full, out_of_memory
            | "53100" | "53200"
    ) || code.starts_with("08")
}

impl Worker {
    fn depth(&self) -> usize {
        self.rx.len()
    }

    async fn run(mut self) {
        let watch = tokio::spawn(Backlog::watch(
            Arc::clone(&self.counts),
            self.config.clone(),
        ));
        loop {
            let Some(first) = self.rx.recv().await else {
                break;
            };
            let mut writes: Vec<Write> = Vec::new();
            let mut flushes: Vec<oneshot::Sender<()>> = Vec::new();
            self.absorb(first, &mut writes, &mut flushes);
            let deadline = Instant::now() + self.config.batch_window;
            while writes.len() < self.config.batch_max {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    msg = self.rx.recv() => match msg {
                        Some(msg) => self.absorb(msg, &mut writes, &mut flushes),
                        None => break,
                    },
                }
            }
            self.commit_with_retry(&writes).await;
            self.settle_pending(&writes);
            for w in &writes {
                self.counts.done(w);
            }
            let depth = self.depth();
            metrics::gauge!(PERSIST_QUEUE_DEPTH).set(depth as f64);
            for f in flushes {
                let _ = f.send(());
            }
        }
        watch.abort();
    }

    /// Drops the committed (or refused) character writes from [`Pending`]: each entry
    /// still holding the record this batch wrote. One a later write replaced stays.
    fn settle_pending(&self, writes: &[Write]) {
        let mut pending = lock(&self.pending);
        for w in writes {
            if let Write::Character { id, character } = w
                && pending.get(id) == Some(&**character)
            {
                pending.remove(id);
            }
        }
    }

    fn absorb(&self, msg: Msg, writes: &mut Vec<Write>, flushes: &mut Vec<oneshot::Sender<()>>) {
        match msg {
            Msg::Write(w) => {
                if let Some(key) = w.overwrite_key() {
                    writes.retain(|earlier| {
                        let keep = earlier.overwrite_key() != Some(key.clone());
                        if !keep {
                            self.counts.done(earlier);
                        }
                        keep
                    });
                }
                writes.push(*w);
            }
            Msg::Flush(f) => flushes.push(f),
        }
    }

    /// Commits `writes` in one transaction, retrying connectivity failures with backoff
    /// and skipping writes the database refuses.
    async fn commit_with_retry(&self, writes: &[Write]) {
        if writes.is_empty() {
            return;
        }
        let mut failures: u32 = 0;
        let mut delay = self.config.retry_min;
        loop {
            match self.commit(writes).await {
                Ok(()) => {
                    self.counts.saved(writes.len());
                    if failures > 0 {
                        tracing::info!(failures, "persistence recovered");
                    }
                    self.status.send_replace(PersistStatus::Healthy);
                    return;
                }
                Err(err) if err.is_refusal() => {
                    // Find the offending writes by halves, skip them, keep the rest.
                    self.commit_split(writes).await;
                    self.status.send_replace(PersistStatus::Healthy);
                    return;
                }
                Err(err) => {
                    let err = err.err();
                    failures += 1;
                    metrics::counter!(PERSIST_ERRORS_TOTAL).increment(1);
                    tracing::warn!(%err, failures, writes = writes.len(), "persistence batch failed; retrying");
                    self.status.send_replace(PersistStatus::Unhealthy {
                        failures,
                        error: err.to_string(),
                    });
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(self.config.retry_max);
                }
            }
        }
    }

    /// Commits `writes` in one transaction on a connection of its own.
    ///
    /// A connection that failed for any reason but a constraint refusal or a refusal
    /// raised outside the database ([`CommitError::keeps_connection`]) is closed, not
    /// returned to the pool. SQLite may roll a transaction back by itself (`SQLITE_FULL`,
    /// `SQLITE_IOERR`, `SQLITE_ABORT_ROLLBACK`) without sqlx noticing: its transaction
    /// depth then disagrees with SQLite's, and the next batch on that connection would
    /// "commit" a savepoint inside a transaction nobody ever ends, reporting success for a
    /// write that is lost and holding the write lock. Keeping the connection on a
    /// constraint refusal lets a one-connection in-memory SQLite database survive one.
    async fn commit(&self, writes: &[Write]) -> std::result::Result<(), CommitError> {
        let mut conn = self
            .db
            .pool()
            .acquire()
            .await
            .map_err(|e| CommitError::Outage(e.into()))?;
        let result = self.run_batch(&mut conn, writes).await;
        if result.as_ref().is_err_and(|e| !e.keeps_connection()) {
            conn.close_on_drop();
        }
        result
    }

    async fn run_batch(
        &self,
        conn: &mut Conn,
        writes: &[Write],
    ) -> std::result::Result<(), CommitError> {
        let mut tx = conn
            .begin()
            .await
            .map_err(|e| CommitError::Outage(e.into()))?;
        if self.config.per_row_reference {
            for w in writes {
                apply(&mut tx, &self.db, w)
                    .await
                    .map_err(CommitError::Write)?;
            }
        } else {
            apply_batch(&mut tx, &self.db, writes)
                .await
                .map_err(CommitError::Write)?;
        }
        tx.commit().await.map_err(|e| CommitError::Write(e.into()))
    }

    /// Commits a refused batch in halves, in order, down to the single writes the
    /// database refuses, which are logged and skipped (T-113: one write at a time cost a
    /// network round trip or more per write; halving finds a refusal in about
    /// 2·log2(n) commits). The order of the writes is kept: each part commits after the
    /// parts before it.
    async fn commit_split(&self, writes: &[Write]) {
        let mut delay = self.config.retry_min;
        let mut parts: Vec<&[Write]> = Vec::new();
        if writes.len() > 1 {
            let (a, b) = writes.split_at(writes.len() / 2);
            parts.extend([b, a]);
        } else {
            parts.push(writes);
        }
        while let Some(part) = parts.pop() {
            loop {
                match self.commit(part).await {
                    Ok(()) => {
                        self.counts.saved(part.len());
                        break;
                    }
                    Err(err) if err.is_refusal() && part.len() > 1 => {
                        let (a, b) = part.split_at(part.len() / 2);
                        parts.extend([b, a]);
                        break;
                    }
                    Err(err) if err.is_refusal() => {
                        self.counts.saved(1);
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                        let err = err.err();
                        let (kind, code) = error_class(err);
                        let w = &part[0];
                        tracing::error!(%err, kind, code, write = %w.describe(), "write refused by the database; skipped");
                        break;
                    }
                    Err(err) => {
                        let err = err.err();
                        metrics::counter!(PERSIST_ERRORS_TOTAL).increment(1);
                        tracing::warn!(%err, "persistence write failed; retrying");
                        self.status.send_replace(PersistStatus::Unhealthy {
                            failures: 1,
                            error: err.to_string(),
                        });
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(self.config.retry_max);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use sqlx::error::{DatabaseError, ErrorKind};

    use super::*;

    /// A database error carrying only a code and a kind.
    #[derive(Debug)]
    struct Coded(&'static str);

    impl std::fmt::Display for Coded {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "code {}", self.0)
        }
    }

    impl std::error::Error for Coded {}

    impl DatabaseError for Coded {
        fn message(&self) -> &str {
            "coded"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn db_err(code: &'static str) -> DbError {
        DbError::Sqlx(sqlx::Error::Database(Box::new(Coded(code))))
    }

    #[test]
    fn busy_locked_and_serialization_errors_are_retried() {
        for code in [
            "5", "261", "517", "773", "6", "262", "518", "40001", "40P01", "55P03", "57014",
            "08006", "08001", "57P01", "57P02", "57P03",
        ] {
            assert!(!is_permanent(&db_err(code)), "{code}");
        }
    }

    #[test]
    fn out_of_space_memory_and_io_errors_are_an_outage_not_a_refusal() {
        // SQLite: SQLITE_FULL, SQLITE_PROTOCOL, SQLITE_IOERR and extended IOERR_WRITE;
        // SQLITE_NOMEM; PostgreSQL: disk_full, out_of_memory.
        for code in ["13", "15", "10", "778", "7", "53100", "53200"] {
            assert!(!is_permanent(&db_err(code)), "{code}");
        }
    }

    #[test]
    fn deterministic_errors_are_permanent_so_they_cannot_wedge_the_queue() {
        let boxed = || -> Box<dyn std::error::Error + Send + Sync> { "bad".into() };
        for err in [
            sqlx::Error::Encode(boxed()),
            sqlx::Error::Decode(boxed()),
            sqlx::Error::ColumnDecode {
                index: "c".into(),
                source: boxed(),
            },
            sqlx::Error::Configuration(boxed()),
        ] {
            assert!(is_permanent(&DbError::Sqlx(err)));
        }
    }

    #[test]
    fn constraint_refusals_stay_permanent() {
        // SQLite: constraint (19), unique (2067), foreign key (787); PostgreSQL: unique
        // violation (23505), foreign key (23503), undefined table (42P01).
        for code in ["19", "2067", "787", "23505", "23503", "42P01"] {
            assert!(is_permanent(&db_err(code)), "{code}");
        }
        assert!(is_permanent(&DbError::Sqlx(sqlx::Error::RowNotFound)));
        assert!(!is_permanent(&DbError::Sqlx(sqlx::Error::PoolTimedOut)));
    }

    #[test]
    fn an_error_opening_a_connection_is_an_outage_whatever_its_class() {
        // A rotated password (PostgreSQL 28P01) and a bad option both classify as
        // permanent per write; raised while connecting they say nothing about the write.
        let cases = [
            db_err("28P01"),
            DbError::Sqlx(sqlx::Error::Configuration("bad".into())),
        ];
        for err in cases {
            assert!(is_permanent(&err));
            assert!(CommitError::Write(err).is_refusal());
        }
        for err in [
            db_err("28P01"),
            DbError::Sqlx(sqlx::Error::Configuration("bad".into())),
        ] {
            assert!(!CommitError::Outage(err).is_refusal());
        }
        assert!(!CommitError::Write(db_err("13")).is_refusal());
    }

    #[test]
    fn only_constraint_refusals_keep_their_connection() {
        // SQLite: constraint (19), unique (2067), primary key (1555), foreign key (787);
        // PostgreSQL: unique (23505), foreign key (23503), check (23514).
        for code in ["19", "2067", "1555", "787", "23505", "23503", "23514"] {
            assert!(
                CommitError::Write(db_err(code)).keeps_connection(),
                "{code}"
            );
        }
        // SQLite: generic error (1), ABORT (4), ABORT_ROLLBACK (516), INTERRUPT (9),
        // CORRUPT (11); PostgreSQL: undefined table (42P01), division by zero (22012).
        for code in ["1", "4", "516", "9", "11", "42P01", "22012"] {
            let err = CommitError::Write(db_err(code));
            assert!(err.is_refusal(), "{code}");
            assert!(!err.keeps_connection(), "{code}");
        }
        assert!(CommitError::Write(DbError::Sqlx(sqlx::Error::RowNotFound)).keeps_connection());
        assert!(CommitError::Write(DbError::Duplicate("k".into())).keeps_connection());
        assert!(!CommitError::Write(db_err("13")).keeps_connection());
        assert!(!CommitError::Outage(db_err("19")).keeps_connection());
    }

    /// T-113: a queue that stays deep is logged once per `backlog_after`, with the drain
    /// rate over the time it has been deep; a queue that comes back under the threshold
    /// starts the clock again.
    #[tokio::test(start_paused = true)]
    async fn a_queue_that_stays_deep_is_warned_about_with_its_drain_rate() {
        let config = PersistConfig::default();
        let mut b = Backlog::new();
        let step = Duration::from_secs(1);
        // deep for 29 s, draining at 30 writes a second: nothing yet
        let mut saved = 0;
        for _ in 0..30 {
            assert_eq!(b.check(&config, saved, 15_000), None);
            saved += 30;
            tokio::time::advance(step).await;
        }
        // 30 s deep at 30 writes a second
        let rate = b.check(&config, saved, 14_000).expect("warned");
        assert!((rate - 30.0).abs() < 1.5, "{rate}");
        // not again within 30 s
        tokio::time::advance(step).await;
        assert_eq!(b.check(&config, saved, 14_000), None);
        // drained below the threshold: the clock starts again
        assert_eq!(b.check(&config, saved, 10), None);
        tokio::time::advance(Duration::from_secs(40)).await;
        assert_eq!(b.check(&config, saved, 5_000), None);
        // a worker that saves nothing (one batch waiting out an outage) is warned about
        // too, at 0 writes a second
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(b.check(&config, saved, 5_000), Some(0.0));
    }

    async fn flushed(persist: &Persist, write: Write) {
        persist.send(write).await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), persist.flush())
            .await
            .unwrap()
            .unwrap();
    }

    /// T-073: a refusal that has rolled the whole transaction back (as SQLite
    /// ABORT_ROLLBACK does) closes its connection, so no pooled connection is left with
    /// sqlx's transaction depth out of step, and the writes after it really commit.
    #[tokio::test]
    async fn a_refusal_that_rolled_back_leaves_no_open_transaction() {
        use sqlx::Executor as _;

        let test = crate::TestDb::from_env().await.unwrap();
        let db = Arc::clone(test.db());
        let (persist, worker) = Persist::spawn(Arc::clone(&db), PersistConfig::default());
        flushed(
            &persist,
            Write::Raw("CREATE TABLE t073 (id INTEGER PRIMARY KEY)"),
        )
        .await;
        flushed(&persist, Write::Raw("INSERT INTO t073 VALUES (1)")).await;
        // the transaction ends behind sqlx's back, then the statement is refused
        flushed(
            &persist,
            Write::Raw("ROLLBACK; SELECT * FROM no_such_table_t073"),
        )
        .await;
        assert_eq!(persist.dropped(), 1);
        // a constraint refusal keeps its connection; then a good write
        flushed(&persist, Write::Raw("INSERT INTO t073 VALUES (1)")).await;
        assert_eq!(persist.dropped(), 2);
        flushed(&persist, Write::Raw("INSERT INTO t073 VALUES (2)")).await;
        assert_eq!(persist.status(), PersistStatus::Healthy);

        // No pooled connection is inside a transaction.
        let mut conns = Vec::new();
        while db.pool().num_idle() > 0 {
            conns.push(db.pool().acquire().await.unwrap());
        }
        assert!(!conns.is_empty());
        let mut pids = Vec::new();
        for c in &mut conns {
            match db.dialect() {
                crate::Dialect::Sqlite => {
                    c.execute("BEGIN IMMEDIATE")
                        .await
                        .expect("no connection is left in an open transaction");
                    c.execute("ROLLBACK").await.unwrap();
                }
                crate::Dialect::Postgres => {
                    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                        .fetch_one(&mut **c)
                        .await
                        .unwrap();
                    pids.push(pid);
                }
            }
        }
        drop(conns);
        let other = Db::connect(db.url()).await.unwrap();
        for pid in pids {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM pg_stat_activity WHERE pid = $1")
                    .bind(pid)
                    .fetch_optional(&other.pool())
                    .await
                    .unwrap()
                    .flatten();
            assert_eq!(state.as_deref(), Some("idle"), "backend {pid}");
        }
        // what the worker reported committed is visible to a new pool
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t073")
            .fetch_one(&other.pool())
            .await
            .unwrap();
        assert_eq!(n, 2);
        other.close().await;
        drop(persist);
        worker.await.unwrap();
        test.close().await;
    }
}
