//! `serve`: load config, take the store's single-writer guard (T-072), open the pool, migrate, check the world, start everything, stop
//! on SIGTERM or Ctrl-C, or when the guard is lost for good (T-073).

use std::process::ExitCode;

use anyhow::Context;
use silvermere_db::{Db, GuardLost, TryAcquire, Watchdog, WorldImport, WriterGuard};
use silvermere_engine::{SeededRng, SystemClock, World};
use silvermere_types::ids::RoomKey;

use crate::cli::ServeArgs;
use crate::config::Config;

const NO_WORLD: &str = "No world data found. Run: silvermere import (downloads the stock 1.11p distribution once and checks it; silvermere import FILE.zip or --from DIR imports files you have, offline)";

pub fn run(cfg: &Config, args: &ServeArgs) -> anyhow::Result<ExitCode> {
    run_with(cfg, args, false)
}

/// [`run`]; `first_start` (T-109) adds the plain "Silvermere is running" line for a
/// double-clicked console.
pub fn run_with(cfg: &Config, args: &ServeArgs, first_start: bool) -> anyhow::Result<ExitCode> {
    if args.print_config {
        print!("{}", cfg.to_toml()?);
        return Ok(ExitCode::SUCCESS);
    }
    silvermere_api::check_bind(&cfg.server.admin_bind, cfg.admin.tokens.len())?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        super::ensure_sqlite_dir(cfg)?;
        // T-072: one server per store, taken before the migrations and held until exit
        let guard = match WriterGuard::try_acquire(&cfg.database.url)
            .await
            .with_context(|| format!("taking the writer guard of {}", cfg.database_shown()))?
        {
            TryAcquire::Taken(guard) => guard,
            TryAcquire::Held(by) => anyhow::bail!(
                "another silvermere is already serving {}: {by}. Stop it first; two \
                 servers on one store overwrite each other's saves",
                cfg.database_shown()
            ),
        };
        // T-073: the task keeps the guard until the process ends and answers only when
        // the guard is lost for good.
        let mut guard_lost = tokio::spawn(watch_guard(guard));
        let db = Db::connect_and_migrate(&cfg.database.url)
            .await
            .with_context(|| format!("opening {}", cfg.database_shown()))?;
        let rooms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rooms")
            .fetch_one(&db.pool())
            .await
            .context("counting rooms")?;
        if rooms == 0 {
            eprintln!("{NO_WORLD}");
            return Ok(ExitCode::from(super::EXIT_NO_WORLD));
        }
        // T-109: rooms are written first, so a cut-off import leaves rooms and no items
        if silvermere_db::world_import::state(&db).await? == WorldImport::Unfinished {
            eprintln!("{}", super::UNFINISHED_IMPORT);
            return Ok(ExitCode::from(super::EXIT_NO_WORLD));
        }
        // As at the original's start-up, the special rooms (all on map 1) must exist.
        // `World::load` makes the same check; this one answers before the whole world is read.
        for (name, room) in World::special_rooms(&cfg.game) {
            if db.rooms().get(RoomKey::new(1, room)).await?.is_none() {
                anyhow::bail!("game.{name} = {room}: room does not exist on map 1");
            }
        }
        db.close().await;
        // The world seed is not configurable yet (T-059 adds it to the config); the
        // production source draws from a seed taken from the clock.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        // Ordering requirement: the SIGTERM and SIGINT handlers must be installed before the
        // "telnet listening" line is printed (and before binding). Callers, such as the
        // Layer-3 tests, treat that line as "a SIGTERM is now handled gracefully"; with the
        // handler installed afterwards, an early SIGTERM hits the default action and kills
        // the process.
        let mut signals = Signals::new(first_start || std::io::IsTerminal::is_terminal(&std::io::stdin()));
        let running =
            crate::serve_in_process(cfg.clone(), SystemClock, SeededRng::new(seed)).await?;
        tracing::info!(telnet = %running.telnet_addr, "listening");
        // T-108: which store this server writes, in the journal (the password masked)
        eprintln!("silvermere: database {}", cfg.database_shown());
        // The line the Layer-3 tests wait for.
        eprintln!("silvermere: telnet listening on {}", running.telnet_addr);
        if let Some(a) = running.admin_addr {
            eprintln!("silvermere: admin listening on {a}");
        }
        if first_start {
            println!(
                "\n{}\n",
                super::first_start::running_line(running.telnet_addr.port())
            );
        }
        let lost = tokio::select! {
            stop = signals.next() => {
                tracing::info!(signal = stop.name(), "stop requested");
                None
            }
            lost = &mut guard_lost => Some(lost.map_or_else(
                |e| format!("the writer guard's watchdog stopped: {e}"),
                |lost| lost.to_string(),
            )),
        };
        if let Some(why) = &lost {
            // Another process may be writing the store now: save what is queued and stop.
            tracing::error!(%why, "writer guard lost; flushing and stopping");
            eprintln!("silvermere: {why}; flushing and stopping");
        }
        // every line from here on is on disk when it is logged: Windows may end the process
        // at any moment of the shutdown (T-111)
        crate::flush_log();
        tracing::info!("shutting down");
        eprintln!("silvermere: shutting down");
        let grace = std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS);
        // After a lost guard another process may hold the store: save what the database
        // takes within the barrier's quiet time, no longer (T-113).
        let barrier_max = if lost.is_some() {
            crate::BARRIER_TIMEOUT
        } else {
            crate::BARRIER_MAX
        };
        let shutdown = running.shutdown_gracefully(grace, crate::BARRIER_TIMEOUT, barrier_max);
        tokio::pin!(shutdown);
        let result = loop {
            tokio::select! {
                r = &mut shutdown => break r,
                stop = signals.next() => {
                    if !stop.leaves_now() {
                        // Windows closing the console, or the session ending, while the save
                        // runs: finishing it is all that helps (T-111)
                        tracing::info!(signal = stop.name(), "ignored during shutdown; still saving");
                        continue;
                    }
                    // A second signal from a person: leave now.
                    tracing::error!("second signal during shutdown: exiting now; data from the last sweep may be stale");
                    eprintln!("silvermere: second signal during shutdown: exiting now; data from the last sweep may be stale");
                    std::process::exit(EXIT_INTERRUPTED);
                }
            }
        };
        match result {
            Ok(done) => {
                tracing::info!(
                    saved = done.report.saved,
                    "shutdown complete: {} characters saved in {} ms",
                    done.report.saved,
                    done.elapsed.as_millis()
                );
                eprintln!(
                    "silvermere: shutdown complete: {} characters saved in {} ms",
                    done.report.saved,
                    done.elapsed.as_millis()
                );
                Ok(if lost.is_some() {
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                })
            }
            Err(e) => {
                tracing::error!(%e, "shutdown failed; characters may not be saved");
                eprintln!("silvermere: shutdown failed: {e:#}");
                Ok(ExitCode::from(1))
            }
        }
    })
}

/// Holds `guard` and returns when it is lost for good ([`WriterGuard::watch`]).
async fn watch_guard(mut guard: WriterGuard) -> GuardLost {
    guard.watch(watchdog()).await
}

/// The watchdog's timing: [`Watchdog::default`], or `SILVERMERE_GUARD_WATCHDOG_MS` =
/// `<ping every>,<give up after>` in milliseconds (for tests).
fn watchdog() -> Watchdog {
    let parsed = std::env::var("SILVERMERE_GUARD_WATCHDOG_MS")
        .ok()
        .and_then(|v| {
            let (every, give_up) = v.split_once(',')?;
            Some(Watchdog {
                every: std::time::Duration::from_millis(every.trim().parse().ok()?),
                give_up: std::time::Duration::from_millis(give_up.trim().parse().ok()?),
            })
        });
    parsed.unwrap_or_default()
}

/// How long the sessions get to drain on the first signal (`EngineControl::Shutdown`).
const SHUTDOWN_GRACE_SECS: u64 = 10;
/// Exit code of a second signal during shutdown (128 + SIGINT).
const EXIT_INTERRUPTED: i32 = 130;

/// SIGTERM (Unix) and Ctrl-C, as a stream of "stop" requests. On Windows (T-109): Ctrl-C,
/// Ctrl-Break, the console window closing, system shutdown, and, for a server on a console
/// (a double-click), the user signing out. A service also receives the logoff event when any
/// user signs out, so a server with no console (NSSM) ignores it. Windows ends the process a
/// few seconds after a close, logoff or shutdown event (tokio holds its handler meanwhile),
/// which the save of every character fits in.
///
/// Windows sends the logoff and shutdown events to a console process only while it has
/// loaded neither user32.dll nor gdi32.dll (SetConsoleCtrlHandler); a dependency that loads
/// either (shell32 loads user32) silences them. ci.yml checks the exe's imports
/// (.github/scripts/windows-imports.sh); patches/etcetera keeps sqlx-postgres off shell32 (T-111).
struct Signals {
    #[cfg(unix)]
    term: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    int: Option<tokio::signal::unix::Signal>,
    #[cfg(windows)]
    win: windows::Events,
}

impl Signals {
    /// `console`: the server runs on a console a person sees (Windows: stop on logoff too).
    fn new(console: bool) -> Self {
        #[cfg(unix)]
        {
            // Unix has no logoff event
            let _ = console;
            use tokio::signal::unix::{SignalKind, signal};
            let term = signal(SignalKind::terminate())
                .inspect_err(|e| {
                    tracing::warn!(%e, "no SIGTERM handler; Ctrl-C only");
                    eprintln!("silvermere: no SIGTERM handler ({e}); Ctrl-C only");
                })
                .ok();
            let int = signal(SignalKind::interrupt())
                .inspect_err(|e| {
                    tracing::warn!(%e, "no SIGINT handler");
                    eprintln!("silvermere: no SIGINT handler ({e})");
                })
                .ok();
            Self { term, int }
        }
        #[cfg(windows)]
        {
            Self {
                win: windows::Events::new(console),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self {}
        }
    }

    /// Resolves on the next signal.
    async fn next(&mut self) -> Stop {
        #[cfg(unix)]
        if self.term.is_some() || self.int.is_some() {
            return tokio::select! {
                _ = recv_or_pending(self.term.as_mut()) => Stop::Terminate,
                _ = recv_or_pending(self.int.as_mut()) => Stop::Interrupt,
            };
        }
        #[cfg(windows)]
        if self.win.installed() > 0 {
            return self.win.next().await;
        }
        let _ = tokio::signal::ctrl_c().await;
        Stop::Interrupt
    }
}

/// What asked the server to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // each platform builds only its own signals
enum Stop {
    /// Ctrl-C, Ctrl-Break or SIGINT.
    Interrupt,
    /// SIGTERM.
    Terminate,
    /// Windows: the console window closing.
    Close,
    /// Windows: the user signing out.
    Logoff,
    /// Windows: the system shutting down.
    SystemShutdown,
}

impl Stop {
    fn name(self) -> &'static str {
        match self {
            Stop::Interrupt => "interrupt",
            Stop::Terminate => "terminate",
            Stop::Close => "console close",
            Stop::Logoff => "sign-out",
            Stop::SystemShutdown => "system shutdown",
        }
    }

    /// Whether this signal, arriving during the shutdown, ends the process at once. Only a
    /// person's second Ctrl-C (or a second SIGTERM) does. Windows' close, sign-out and
    /// shutdown events arrive together at a sign-out (the terminal closes as the session
    /// ends), and Windows ends the process on its own deadline anyway: the save carries on.
    fn leaves_now(self) -> bool {
        matches!(self, Stop::Interrupt | Stop::Terminate)
    }
}

#[cfg(windows)]
mod windows {
    use tokio::signal::windows as w;

    /// The console control events that stop the server.
    pub(super) struct Events {
        c: Option<w::CtrlC>,
        brk: Option<w::CtrlBreak>,
        close: Option<w::CtrlClose>,
        shutdown: Option<w::CtrlShutdown>,
        logoff: Option<w::CtrlLogoff>,
    }

    fn ok<T>(what: &str, r: std::io::Result<T>) -> Option<T> {
        r.inspect_err(|e| tracing::warn!(%e, "no {what} handler"))
            .ok()
    }

    impl Events {
        /// `logoff`: also stop when the user signs out (a server on a console).
        pub(super) fn new(logoff: bool) -> Self {
            Self {
                c: ok("Ctrl-C", w::ctrl_c()),
                brk: ok("Ctrl-Break", w::ctrl_break()),
                close: ok("console close", w::ctrl_close()),
                shutdown: ok("shutdown", w::ctrl_shutdown()),
                logoff: logoff.then(|| ok("logoff", w::ctrl_logoff())).flatten(),
            }
        }

        /// How many handlers are installed.
        pub(super) fn installed(&self) -> usize {
            [
                self.c.is_some(),
                self.brk.is_some(),
                self.close.is_some(),
                self.shutdown.is_some(),
                self.logoff.is_some(),
            ]
            .into_iter()
            .filter(|&b| b)
            .count()
        }

        pub(super) async fn next(&mut self) -> super::Stop {
            use super::Stop;
            async fn on<T>(s: Option<&mut T>, recv: impl AsyncFnOnce(&mut T)) {
                match s {
                    Some(s) => recv(s).await,
                    None => std::future::pending().await,
                }
            }
            tokio::select! {
                _ = on(self.c.as_mut(), async |s| { s.recv().await; }) => Stop::Interrupt,
                _ = on(self.brk.as_mut(), async |s| { s.recv().await; }) => Stop::Interrupt,
                _ = on(self.close.as_mut(), async |s| { s.recv().await; }) => Stop::Close,
                _ = on(self.shutdown.as_mut(), async |s| { s.recv().await; }) => Stop::SystemShutdown,
                _ = on(self.logoff.as_mut(), async |s| { s.recv().await; }) => Stop::Logoff,
            }
        }
    }

    /// Every console event the server stops on gets a handler.
    #[test]
    fn every_stop_event_has_a_handler() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // a console server stops on logoff too; a service ignores it
        let console = rt.block_on(async { Events::new(true) });
        assert_eq!(console.installed(), 5);
        drop(console);
        let service = rt.block_on(async { Events::new(false) });
        assert_eq!(service.installed(), 4);
    }
}

/// The next delivery of `sig`, or never when its handler could not be installed.
#[cfg(unix)]
async fn recv_or_pending(sig: Option<&mut tokio::signal::unix::Signal>) {
    match sig {
        Some(sig) => {
            sig.recv().await;
        }
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod stop_tests {
    use super::Stop;

    /// A second Ctrl-C or SIGTERM leaves at once; Windows' own events never cut the save
    /// short (T-111: at a sign-out the console closes as well).
    #[test]
    fn only_a_person_leaves_during_shutdown() {
        assert!(Stop::Interrupt.leaves_now());
        assert!(Stop::Terminate.leaves_now());
        for s in [Stop::Close, Stop::Logoff, Stop::SystemShutdown] {
            assert!(!s.leaves_now(), "{}", s.name());
        }
    }
}
