//! The server as a library: `main.rs` is a thin shell over [`run`], and tests start a
//! server in-process with [`serve_in_process`].

#![forbid(unsafe_code)]

pub mod admin;
pub mod bridge;
pub mod cli;
pub mod cmd_import;
pub mod commands;
pub mod config;
pub mod gate;
pub mod stock;
pub mod store;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use config::{Config, LogFormat};
use silvermere_db::Db;
use silvermere_engine::{Clock, EngineControl, EngineInput, Rng, World};
use tokio::sync::{mpsc as tmpsc, watch};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;

/// A server started by [`serve_in_process`].
pub struct Running {
    /// The bound telnet address (the configured port may have been 0).
    pub telnet_addr: SocketAddr,
    /// The bound admin address (`server.admin_bind`; the configured port may have been 0).
    pub admin_addr: Option<SocketAddr>,
    /// Send `true` (or drop) to stop accepting and ticking.
    pub shutdown: watch::Sender<bool>,
    /// The engine's input channel (the admin API and tests feed it).
    pub to_engine: mpsc::Sender<EngineInput>,
    engine: Option<std::thread::JoinHandle<World>>,
    /// The admin listener and the event writer; aborted on stop (an open SSE connection
    /// would otherwise keep the runtime alive).
    admin_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Writes engine events to the table; ends when the bridge drops its sender, so
    /// `stop` lets it drain the events queued at shutdown (the sessions' `closed`).
    events_task: Option<tokio::task::JoinHandle<()>>,
    events_db: Option<std::sync::Arc<Db>>,
    /// Writes the engine's saves and answers its name checks (`store.rs`); ends when the
    /// bridge drops its sender after `Stopped`, after a final flush.
    store_task: Option<tokio::task::JoinHandle<()>>,
    /// Bumped each time the store task has seen a `Flush` barrier through the worker.
    flushed: watch::Receiver<u64>,
    /// The worker's progress, for a shutdown deciding whether to keep waiting (T-113).
    progress: silvermere_db::persist::Progress,
}

/// How long shutdown waits for the persistence barrier with no write saved before
/// giving up (the database is unreachable). While
/// writes are being saved it waits on, up to [`BARRIER_MAX`] (T-113): a deep queue over a
/// slow link is not an outage.
pub const BARRIER_TIMEOUT: Duration = Duration::from_secs(15);

/// The longest shutdown waits for the barrier while the queue still drains (T-113). With
/// the engine's report (up to [`BARRIER_TIMEOUT`]) and the stop's own waits (about 12 s)
/// a shutdown takes at most about 57 s, inside the 60 s `TimeoutStopSec` and
/// `stop_grace_period` of docs/install.md and docker/compose.yaml.
pub const BARRIER_MAX: Duration = Duration::from_secs(30);

/// Waits for the shutdown's persistence barrier (T-113): fails when `quiet` passes with
/// no write saved, or at `max`. A queue still draining over a slow database is waited for;
/// one the database stopped taking is not. On failure the journal gets the writes not
/// saved, per kind, players' records first.
pub async fn barrier(
    flushed: &mut watch::Receiver<u64>,
    progress: &silvermere_db::persist::Progress,
    quiet: Duration,
    max: Duration,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + max;
    let mut saved = progress.saved();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(quiet.min(left), flushed.changed()).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(_)) => break,
            Err(_) => {
                let now = progress.saved();
                if now == saved || left <= quiet {
                    break;
                }
                let queued: u64 = progress.unsaved().iter().map(|(_, n)| n).sum();
                tracing::info!(queued, "shutdown: still saving");
                saved = now;
            }
        }
    }
    let unsaved = progress.unsaved();
    let total: u64 = unsaved.iter().map(|(_, n)| n).sum();
    let kinds = unsaved
        .iter()
        .map(|(k, n)| format!("{k} {n}"))
        .collect::<Vec<_>>()
        .join(", ");
    tracing::error!(
        unsaved = total,
        kinds,
        "persistence barrier not reached; {total} queued writes not saved ({kinds}) (is the database reachable?)"
    );
    anyhow::bail!("persistence barrier timed out: {total} writes not saved ({kinds})")
}

/// What a graceful shutdown did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownSummary {
    pub report: silvermere_engine::ShutdownReport,
    /// From the start of the shutdown to the characters' commit (T-111: the save comes
    /// first; the drain and the teardown follow it).
    pub committed: Duration,
    pub elapsed: Duration,
}

impl Running {
    /// The graceful shutdown of `silvermere serve`: stops accepting, has the engine tell
    /// every player, save every character and close the sessions, waits for the
    /// persistence barrier (at most `barrier_timeout` with no write saved, `barrier_max` in
    /// all, [`barrier`]), then stops as [`Running::stop`]
    /// does and checkpoints the database. An error means characters may not have reached
    /// the database; the process still stops.
    pub async fn shutdown_gracefully(
        self,
        grace: Duration,
        barrier_timeout: Duration,
        barrier_max: Duration,
    ) -> anyhow::Result<ShutdownSummary> {
        let started = std::time::Instant::now();
        // T-111: a line as each phase starts, with the time so far, so a log cut short by
        // Windows shows how far the shutdown got
        tracing::info!("shutdown phase 1/4: saving characters");
        // No new connections, no more ticks.
        let _ = self.shutdown.send(true);
        let mut flushed = self.flushed.clone();
        flushed.mark_unchanged();
        let (reply, report_rx) = mpsc::channel();
        let sent = self
            .to_engine
            .send(EngineInput::Control(EngineControl::Shutdown {
                grace,
                reply,
            }))
            .is_ok();
        let report = if sent {
            tokio::task::spawn_blocking(move || report_rx.recv_timeout(grace.max(barrier_timeout)))
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            None
        };
        let Some(report) = report else {
            tracing::error!("the engine did not answer the shutdown order");
            let _ = self.stop().await;
            anyhow::bail!("the engine did not answer the shutdown order");
        };
        tracing::info!(
            "shutdown phase 2/4: {} saves queued in {} ms; waiting for the commit",
            report.saved,
            started.elapsed().as_millis()
        );
        let failure =
            match barrier(&mut flushed, &self.progress, barrier_timeout, barrier_max).await {
                // the saves are committed: logged before the teardown, which Windows may cut
                // short at a sign-out (T-111)
                Ok(()) => {
                    tracing::info!(
                        saved = report.saved,
                        "characters saved: {} committed in {} ms",
                        report.saved,
                        started.elapsed().as_millis()
                    );
                    None
                }
                // `barrier` has logged what is left unsaved, by kind
                Err(e) => Some(e),
            };
        let committed = started.elapsed();
        tracing::info!(
            "shutdown phase 3/4: draining sessions at {} ms",
            committed.as_millis()
        );
        // Let the sessions' last writes reach their sockets before the runtime goes.
        tokio::time::sleep(Duration::from_millis(250)).await;
        tracing::info!(
            "shutdown phase 4/4: stopping the engine and the database at {} ms",
            started.elapsed().as_millis()
        );
        let stopped = self.stop().await;
        if let Some(e) = failure {
            return Err(e);
        }
        stopped?;
        Ok(ShutdownSummary {
            report,
            committed,
            elapsed: started.elapsed(),
        })
    }

    /// The worker's progress (writes saved, writes queued per kind).
    pub fn progress(&self) -> &silvermere_db::persist::Progress {
        &self.progress
    }

    /// Waits (at most `timeout`) until every write the engine has queued so far is
    /// committed: the start-up pass alone queues thousands (T-076), and a login's name
    /// check runs only after a flush. `false` when the barrier did not come back in time.
    pub async fn settle(&self, timeout: Duration) -> bool {
        let mut flushed = self.flushed.clone();
        flushed.mark_unchanged();
        let queued = self
            .to_engine
            .send(EngineInput::Custom {
                session: None,
                run: Box::new(|w: &mut World| w.queue_flush()),
            })
            .is_ok();
        queued
            && matches!(
                tokio::time::timeout(timeout, flushed.changed()).await,
                Ok(Ok(()))
            )
    }

    /// Stops accepting, sends the engine its shutdown and waits for the world thread to
    /// finish. Returns the world as it was at shutdown.
    pub async fn stop(mut self) -> anyhow::Result<World> {
        let _ = self.shutdown.send(true);
        let _ = self.to_engine.send(EngineInput::Shutdown);
        let handle = self.engine.take().context("engine already joined")?;
        let world = tokio::task::spawn_blocking(move || handle.join())
            .await?
            .map_err(|_| anyhow::anyhow!("engine thread panicked"));
        if let Some(t) = self.events_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), t).await;
        }
        if let Some(t) = self.store_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(10), t).await;
        }
        for t in &self.admin_tasks {
            t.abort();
        }
        if let Some(db) = self.events_db.take() {
            if let Err(e) = silvermere_db::backup::checkpoint(&db).await {
                tracing::warn!(%e, "WAL checkpoint failed");
            }
            db.close().await;
        }
        world
    }
}

/// Opens the database, migrates, loads the world (refusing when a special room is
/// missing), starts the engine thread with `clock` and `rng`, the fast ticker, the
/// telnet acceptor on `cfg.server.telnet_bind` and the bridges between them.
pub async fn serve_in_process(
    cfg: Config,
    clock: impl Clock + Send + 'static,
    rng: impl Rng + Send + 'static,
) -> anyhow::Result<Running> {
    commands::ensure_sqlite_dir(&cfg)?;
    let db = Db::connect_and_migrate(&cfg.database.url)
        .await
        .with_context(|| format!("opening {}", cfg.database_shown()))?;
    let snapshot = bridge::load_snapshot(&db).await?;
    let max_sessions = u16::try_from(cfg.server.max_sessions).unwrap_or(u16::MAX);
    let mut world = World::load(cfg.game.clone(), max_sessions, snapshot)?;
    // The `FILE DESCRIPTION` room files (T-025); without them those rooms show no title.
    let assets = cfg.data_dir.join("assets");
    match silvermere_engine::display::Assets::load_dir(&assets) {
        Ok(a) => world.display.assets = a,
        Err(e) => tracing::warn!(dir = %assets.display(), "room description files not loaded: {e}"),
    }
    // The help, the creation forms and the long screens (T-138).
    stock::install_text(&cfg.data_dir);
    serve_world_with_db(cfg, world, Some(db), clock, rng).await
}

/// [`serve_in_process`] over a world that is already loaded, with no database for the
/// login dialogue or the saves: no login dialogue runs, the engine sees every line (tests
/// of the transport and the bridge). The `events` table still needs a reachable
/// `database.url`.
pub async fn serve_world(
    cfg: Config,
    world: World,
    clock: impl Clock + Send + 'static,
    rng: impl Rng + Send + 'static,
) -> anyhow::Result<Running> {
    serve_world_with_db(cfg, world, None, clock, rng).await
}

/// The `[compat]` settings the engine reads.
/// SYSOP RELOAD (T-055): the configuration assembled again from the same data directory
/// (the command line's overrides do not touch the game options).
fn config_reloader(cfg: &Config) -> bridge::ConfigReloader {
    let data_dir = cfg.data_dir.clone();
    bridge::ConfigReloader(std::sync::Arc::new(move || {
        match Config::load(Some(&data_dir), &config::Overrides::default()) {
            Ok(c) => Some(c.game),
            Err(e) => {
                tracing::warn!(%e, "SYSOP RELOAD: configuration not reloaded");
                None
            }
        }
    }))
}

fn engine_compat(cfg: &Config) -> silvermere_engine::Compat {
    let version = cfg.compat.version_string.clone();
    // The short version is the full version string without its build stamp.
    let short = version
        .split_once(" (")
        .map_or(version.as_str(), |(v, _)| v)
        .to_string();
    silvermere_engine::Compat {
        bbs_menu: cfg.compat.bbs_menu,
        mud_version: version,
        short_version: short,
        utf8: cfg.compat.charset == config::Charset::Utf8,
    }
}

/// [`serve_world`] with the database the login dialogue reads accounts and characters
/// from (T-021's gate, [`gate::run`]).
pub async fn serve_world_with_db(
    cfg: Config,
    mut world: World,
    db: Option<Db>,
    clock: impl Clock + Send + 'static,
    rng: impl Rng + Send + 'static,
) -> anyhow::Result<Running> {
    world.compat = engine_compat(&cfg);
    let addr: SocketAddr = cfg.server.telnet_bind.parse()?;
    let net_cfg = silvermere_net::ServerConfig {
        max_sessions: cfg.server.max_sessions as usize,
        // the engine and the login gate send `LineStarted` for every line they take (T-094)
        session: silvermere_net::SessionConfig {
            hold_line_echo: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let listener = silvermere_net::bind(addr, &net_cfg)?;
    let telnet_addr = listener.local_addr()?;
    let admin_addr: SocketAddr = cfg.server.admin_bind.parse()?;
    let admin_listener = tokio::net::TcpListener::bind(admin_addr)
        .await
        .with_context(|| format!("binding server.admin_bind {admin_addr}"))?;
    let admin_addr = admin_listener.local_addr()?;
    // One pool for the gate, the saves and the events table when the caller has a
    // database; otherwise the events table opens its own.
    let db = db.map(std::sync::Arc::new);
    let events_db = match &db {
        Some(db) => db.clone(),
        None => std::sync::Arc::new(
            Db::connect_and_migrate(&cfg.database.url)
                .await
                .with_context(|| format!("opening {}", cfg.database_shown()))?,
        ),
    };
    let (shutdown, stop) = watch::channel(false);
    let (to_engine, from_bin) = mpsc::channel();
    let (outputs, from_engine) = mpsc::channel();
    let tick = Duration::from_secs(u64::from(cfg.game.sec_fast.max(1)));
    let engine = silvermere_engine::spawn(world, clock, rng, from_bin, outputs)?;

    let (net_tx, net_rx) = tmpsc::channel(256);
    let (registry, registry_rx) = tmpsc::unbounded_channel();
    let (opened_tx, opened_rx) = tmpsc::unbounded_channel();
    let (game_events, game_events_rx) = tmpsc::unbounded_channel();
    let (persist, persist_worker) =
        silvermere_db::Persist::spawn(events_db.clone(), silvermere_db::PersistConfig::default());
    let progress = persist.progress();
    let vitals = admin::Vitals::with_persist(persist.watch());
    let state = admin::api_state(&cfg, events_db.clone(), to_engine.clone(), vitals.clone());
    silvermere_engine::metrics::describe_all();
    metrics::gauge!(
        silvermere_engine::metrics::BUILD_INFO,
        "version" => env!("CARGO_PKG_VERSION"),
        "backend" => events_db.dialect().name(),
    )
    .set(1.0);
    let events_task = tokio::spawn(admin::forward_game_events(
        game_events_rx,
        state.log.clone(),
        state.events.clone(),
    ));
    let admin_tasks = vec![tokio::spawn(async move {
        let never = std::future::pending();
        if let Err(e) = silvermere_api::serve(admin_listener, state, never).await {
            tracing::error!(%e, "admin listener stopped");
        }
    })];
    let (gate_tx, gate_rx) = tmpsc::channel(256);
    let (gate_registry_tx, gate_registry_rx) = tmpsc::unbounded_channel();
    let (control_tx, control_rx) = tmpsc::unbounded_channel();
    let (flushed_tx, flushed) = watch::channel(0u64);
    match &db {
        Some(db) => {
            let db = db.clone();
            let service = silvermere_auth::LoginService::new(
                db.clone(),
                silvermere_auth::SystemClock,
                silvermere_auth::LockoutConfig::default(),
            );
            let deps = gate::GateDeps {
                service: std::sync::Arc::new(service),
                db,
                config: silvermere_auth::LoginConfig {
                    login_art: silvermere_auth::art::load(&cfg.data_dir.join("assets")).bytes,
                    open_signup: cfg.game.open_signup,
                    ..Default::default()
                },
                login_timeout: (cfg.net.login_timeout_secs > 0)
                    .then(|| Duration::from_secs(u64::from(cfg.net.login_timeout_secs))),
            };
            tokio::spawn(gate::run(
                net_rx,
                registry_rx,
                gate_registry_tx,
                gate_tx,
                control_rx,
                deps,
            ));
        }
        None => {
            tokio::spawn(gate::pass_through(
                net_rx,
                registry_rx,
                gate_registry_tx,
                gate_tx,
            ));
        }
    }
    tokio::spawn(bridge::forward_events(
        gate_rx,
        opened_rx,
        to_engine.clone(),
    ));
    let (store_tx, store_rx) = tmpsc::unbounded_channel();
    let store_task = tokio::spawn(store::run(
        events_db.clone(),
        persist,
        persist_worker,
        store_rx,
        to_engine.clone(),
        flushed_tx,
    ));
    tokio::spawn(bridge::forward_outputs(
        from_engine,
        gate_registry_rx,
        opened_tx,
        net_cfg.full_message.clone(),
        control_tx,
        bridge::Sinks {
            events: game_events,
            store: store_tx,
            reload: Some(config_reloader(&cfg)),
        },
        vitals.engine_alive_flag(),
    ));
    tokio::spawn(bridge::ticker(
        tick,
        to_engine.clone(),
        stop.clone(),
        vitals.ticks_counter(),
    ));
    let mut stop_accept = stop;
    tokio::spawn(async move {
        tokio::select! {
            r = silvermere_net::serve(listener, net_cfg, net_tx, registry) => {
                if let Err(e) = r { tracing::error!(%e, "telnet acceptor stopped"); }
            }
            _ = stop_accept.changed() => {}
        }
    });
    Ok(Running {
        telnet_addr,
        admin_addr: Some(admin_addr),
        shutdown,
        to_engine,
        engine: Some(engine),
        admin_tasks,
        events_task: Some(events_task),
        events_db: Some(events_db),
        store_task: Some(store_task),
        flushed,
        progress,
    })
}

/// The file log's writer thread, until [`flush_log`] drains it.
static LOG_GUARD: std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> =
    std::sync::Mutex::new(None);
/// Set by [`flush_log`]: the file log is written synchronously from then on.
static LOG_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Writes out the file log's queued lines, and from then on writes each line to the file
/// before the call that logs it returns. Called when the server starts to shut down,
/// since Windows may end the process at any moment after that (T-111), and before
/// `std::process::exit`.
pub fn flush_log() {
    // direct writes first: a line queued after the writer thread stopped would be lost (one
    // written directly while the queue drains may only land ahead of older lines)
    LOG_SYNC.store(true, std::sync::atomic::Ordering::SeqCst);
    let guard = LOG_GUARD.lock().map(|mut g| g.take()).unwrap_or(None);
    // dropping the guard waits for the writer thread to empty its queue
    drop(guard);
}

/// Flushes the file log when dropped ([`flush_log`]).
#[must_use = "the file log is flushed when this is dropped"]
struct LogFlush;

impl Drop for LogFlush {
    fn drop(&mut self) {
        flush_log();
    }
}

/// The file log: through the writer thread, then, after [`flush_log`], straight to the file.
struct FileLog {
    queued: tracing_appender::non_blocking::NonBlocking,
    direct: tracing_appender::rolling::RollingFileAppender,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileLog {
    type Writer = tracing_subscriber::fmt::writer::EitherWriter<
        tracing_appender::non_blocking::NonBlocking,
        tracing_appender::rolling::RollingWriter<'a>,
    >;

    fn make_writer(&'a self) -> Self::Writer {
        use tracing_subscriber::fmt::writer::EitherWriter;
        if LOG_SYNC.load(std::sync::atomic::Ordering::SeqCst) {
            EitherWriter::B(self.direct.make_writer())
        } else {
            EitherWriter::A(self.queued.clone())
        }
    }
}

/// What `serve` also writes to stderr (the journal under systemd) besides the log file:
/// warnings and errors, and the writer guard's and the persistence worker's own lines, so
/// a recovered guard or a retried write shows where the operator looks (T-108).
/// sqlx's slow-statement warning (the room load at start-up) stays in the file, and so do
/// `serve`'s own events, which it prints to stderr itself.
const JOURNAL_FILTER: &str = "warn,sqlx::query=error,silvermere::commands::serve=off,\
     silvermere_db::guard=info,silvermere_db::persist=info";

fn init_logging(cfg: &Config, to_file: bool) -> LogFlush {
    // PostgreSQL's notices (`relation "_sqlx_migrations" already exists, skipping` on every
    // one-shot command) are noise at info; its warnings still show.
    let filter = || {
        EnvFilter::try_new(format!(
            "sqlx::postgres::notice=warn,{},audit=info",
            cfg.logging.level
        ))
        .unwrap_or_else(|_| EnvFilter::new("info"))
    };
    if !to_file {
        let b = tracing_subscriber::fmt().with_env_filter(filter());
        let _ = match cfg.logging.format {
            LogFormat::Json => b.json().with_writer(std::io::stderr).try_init(),
            LogFormat::Pretty => b.with_writer(std::io::stderr).try_init(),
        };
        return LogFlush;
    }
    let logs = cfg.data_dir.join("logs");
    let (queued, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&logs, "silvermere.log"));
    // the same daily file, appended to once the queue is drained
    let direct = tracing_appender::rolling::daily(&logs, "silvermere.log");
    let file = FileLog { queued, direct };
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let journal = || EnvFilter::new(JOURNAL_FILTER);
    let layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> =
        match cfg.logging.format {
            LogFormat::Json => vec![
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(file)
                    .with_filter(filter())
                    .boxed(),
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::io::stderr)
                    .with_filter(journal())
                    .boxed(),
            ],
            LogFormat::Pretty => vec![
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(file)
                    .with_filter(filter())
                    .boxed(),
                tracing_subscriber::fmt::layer()
                    .with_ansi(ansi)
                    .with_writer(std::io::stderr)
                    .with_filter(journal())
                    .boxed(),
            ],
        };
    let _ = tracing_subscriber::registry().with(layers).try_init();
    if let Ok(mut g) = LOG_GUARD.lock() {
        *g = Some(guard);
    }
    LogFlush
}

/// Parses `args`, runs the subcommand and returns its exit code.
pub fn run(args: impl IntoIterator<Item = std::ffi::OsString>) -> anyhow::Result<ExitCode> {
    use cli::Command::*;
    let cli = match cli::Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            e.print()?;
            return Ok(ExitCode::from(code as u8));
        }
    };
    let Some(command) = &cli.command else {
        // T-109: a double-click. Logs go to the data directory; the console keeps the plain
        // lines, and a failure stays on screen until Enter.
        use std::io::IsTerminal as _;
        let result = Config::load(cli.data_dir.as_deref(), &cli.overrides()).and_then(|cfg| {
            let _guard = init_logging(
                &cfg,
                std::fs::create_dir_all(cfg.data_dir.join("logs")).is_ok(),
            );
            commands::first_start::run(&cfg)
        });
        return Ok(commands::first_start::finish(
            result,
            std::io::stdin().is_terminal(),
            &mut std::io::stdin().lock(),
            &mut std::io::stderr(),
        ));
    };
    if let Healthcheck(a) = command {
        return Ok(commands::healthcheck::run(a));
    }
    if let Service { action } = command {
        return commands::service::run(action, cli.data_dir.as_deref(), &cli.overrides());
    }
    // Before the load: `config path` and `config edit` work on a file startup refuses.
    if let Settings { action } = command {
        return commands::config::run(action, cli.data_dir.as_deref(), &cli.overrides());
    }
    let cfg = Config::load(cli.data_dir.as_deref(), &cli.overrides())?;
    let serving = matches!(command, Serve(a) if !a.print_config);
    let _guard = init_logging(
        &cfg,
        serving && std::fs::create_dir_all(cfg.data_dir.join("logs")).is_ok(),
    );
    match command {
        Serve(a) => commands::serve::run(&cfg, a),
        Import(a) => commands::import::run(&cfg, a),
        Migrate => commands::migrate::run(&cfg),
        Backup(a) => commands::backup::run(&cfg, a),
        Restore(a) => commands::restore::run(&cfg, a),
        Account { action } => commands::account::run(&cfg, action),
        Admin { action } => commands::admin::run(&cfg, action),
        Sysop { action } => commands::sysop::run(&cfg, action),
        Setup => commands::first_start::setup(&cfg),
        Healthcheck(a) => Ok(commands::healthcheck::run(a)),
        Service { .. } | Settings { .. } => unreachable!("dispatched above"),
    }
}
