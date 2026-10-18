//! The event loop: one thread, one world, one input channel.
//!
//! Session events, ticks and admin requests all arrive as [`EngineInput`] on one channel
//! (the `select!` happens where the channel is fed: `bin` merges
//! the session tasks, a tokio interval for the fast tick and the admin API into it).
//! The engine thread blocks on `recv`, handles one input at a time inside
//! `catch_unwind`, flushes every touched output batch as one write per session, and
//! emits [`EngineOutput`]s. Nothing else mutates game state.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use silvermere_text::ansi::ANSI_CLRLINE;
use silvermere_text::{Msg, cp437, finish};
use silvermere_types::ids::SessionId;

use crate::output::{PromptText, render_items};
use crate::player_session::Caps;
use crate::scheduler::{self, Clock, Rng, TickReport};
use crate::session;
use crate::state::Outcome;
use crate::tell::{palette_of, prompt_bytes};
use crate::world::{World, WorldSnapshot};

/// The transport's handle for a connection, before the engine gives it a [`SessionId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnId(pub u64);

/// How typed bytes are echoed (`echon`, `echsec`); mirrors `silvermere-net`'s `EchoMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EchoMode {
    /// Echo each byte as typed.
    #[default]
    On,
    /// Echo nothing.
    Off,
    /// Echo `ch` per byte up to `width` bytes (a password field echoes `*` for 8 bytes).
    Secure { ch: u8, width: u8 },
}

/// A request from the admin API (T-018).
#[derive(Debug)]
pub enum AdminRequest {
    /// `/v1/who`: the players in the realm, in `listslot` order.
    Who { reply: Sender<Vec<WhoEntry>> },
    /// `/v1/broadcast`: a line to everyone in the game (the sysop `SPEAK`); the reply is
    /// the number of players told.
    Broadcast { text: String, reply: Sender<usize> },
    /// `/v1/reload-world`: replace the content tables.
    ReloadContent {
        snapshot: Box<WorldSnapshot>,
        reply: Sender<Result<(), crate::world::WorldError>>,
    },
    /// `DELETE /v1/accounts/{id}` (T-069): the account and its character go through
    /// [`session::on_account_deleted`]; a character in the game is logged off first. The
    /// reply comes once the row deletion is queued ([`session::delete_account`]).
    DeleteAccount {
        account: silvermere_types::ids::AccountId,
        reply: Sender<session::AccountDeletion>,
    },
}

/// One row of `/v1/who`: what `display_scan_of_users` shows for a player,
/// as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoEntry {
    pub id: SessionId,
    /// The player's place in the user list.
    pub listslot: usize,
    pub first_name: String,
    pub last_name: String,
    pub title: String,
    /// The class name (`class.name`).
    pub class: String,
    /// The race name (`race.name`).
    pub race: String,
    pub level: u16,
    /// `player.evil`, the evil points the reputation column is derived from.
    pub alignment: i16,
    /// `player.gang`, empty for none.
    pub gang: String,
    /// Seconds since the session's last input.
    pub idle_secs: u64,
    /// The sysop made himself invisible (`INVISB`): the in-game list skips him, the sysop's
    /// own view does not.
    pub hidden: bool,
}

/// What comes into the engine.
pub enum EngineInput {
    /// A connection is up.
    Connected { conn: ConnId, caps: Caps },
    /// The login dialogue (`silvermere-auth`, run by `bin`) authenticated the session's
    /// user: the engine binds the account and its character and shows the BBS menu shim
    /// or the `[MAJORMUD]:` menu (T-021, `state::login::logged_in`).
    LoggedIn {
        id: SessionId,
        login: Box<crate::state::login::Login>,
    },
    /// One edited input line (UTF-8, decoded from CP437 by the transport).
    Line { id: SessionId, text: String },
    /// One keystroke in character mode.
    Key { id: SessionId, byte: u8 },
    /// The client reported its window size.
    Naws {
        id: SessionId,
        columns: u16,
        rows: u16,
    },
    /// The connection is gone.
    Disconnect { id: SessionId },
    /// The fast tick (one per second in production).
    Tick,
    /// An admin API request.
    Admin(AdminRequest),
    /// The daily cleanup is due (T-056).
    DailyCleanup,
    /// Run a closure on the world thread, on behalf of `session` if any. Tooling and
    /// tests use it; a panic inside it is handled like any other handler panic.
    Custom {
        session: Option<SessionId>,
        run: Box<dyn FnOnce(&mut World) + Send>,
    },
    /// Graceful shutdown (T-059): see [`crate::shutdown`].
    Control(crate::shutdown::EngineControl),
    /// Stop: `finrou`, then the loop returns the world.
    Shutdown,
}

impl fmt::Debug for EngineInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected { conn, caps } => f
                .debug_struct("Connected")
                .field("conn", conn)
                .field("caps", caps)
                .finish(),
            Self::LoggedIn { id, login } => f
                .debug_struct("LoggedIn")
                .field("id", id)
                .field("account", &login.account)
                .finish_non_exhaustive(),
            Self::Line { id, text } => f
                .debug_struct("Line")
                .field("id", id)
                .field("text", text)
                .finish(),
            Self::Key { id, byte } => f
                .debug_struct("Key")
                .field("id", id)
                .field("byte", byte)
                .finish(),
            Self::Naws { id, columns, rows } => f
                .debug_struct("Naws")
                .field("id", id)
                .field("columns", columns)
                .field("rows", rows)
                .finish(),
            Self::Disconnect { id } => f.debug_struct("Disconnect").field("id", id).finish(),
            Self::Tick => f.write_str("Tick"),
            Self::Admin(req) => f.debug_tuple("Admin").field(req).finish(),
            Self::DailyCleanup => f.write_str("DailyCleanup"),
            Self::Custom { session, .. } => f
                .debug_struct("Custom")
                .field("session", session)
                .finish_non_exhaustive(),
            Self::Control(c) => f.debug_tuple("Control").field(c).finish(),
            Self::Shutdown => f.write_str("Shutdown"),
        }
    }
}

impl EngineInput {
    /// The variant's name, for log lines.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "Connected",
            Self::LoggedIn { .. } => "LoggedIn",
            Self::Line { .. } => "Line",
            Self::Key { .. } => "Key",
            Self::Naws { .. } => "Naws",
            Self::Disconnect { .. } => "Disconnect",
            Self::Tick => "Tick",
            Self::Admin(_) => "Admin",
            Self::DailyCleanup => "DailyCleanup",
            Self::Custom { .. } => "Custom",
            Self::Control(_) => "Control",
            Self::Shutdown => "Shutdown",
        }
    }

    /// The session an input concerns, for the `Command interrupted.` path.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Line { id, .. }
            | Self::LoggedIn { id, .. }
            | Self::Key { id, .. }
            | Self::Naws { id, .. }
            | Self::Disconnect { id } => Some(*id),
            Self::Custom { session, .. } => *session,
            _ => None,
        }
    }
}

/// A game event for the events table and the SSE stream (T-018 fills the variants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    /// A session was opened.
    Connected { id: SessionId },
    /// A session was closed.
    Closed { id: SessionId },
    /// `SUICIDE` cost a life (logged as `User %s has committed suicide.`).
    Suicided { id: SessionId },
    /// `SUICIDE NOW`, `REROLL` or a last-life `SUICIDE` deleted the character
    /// (logged as `User %s has rerolled.`).
    Rerolled { id: SessionId },
    /// The session's character trained to `level` (`train_level`, T-039).
    Level { id: SessionId, level: u16 },
    /// A sysop action (the operator-visible ones of the sysop commands and the sysop
    /// menu, T-055): `kind` is `sysop.<keyword>`, `user` the operator's user-id.
    Sysop {
        id: SessionId,
        kind: &'static str,
        user: String,
        detail: String,
    },
    /// A world audit line: the original's audit and log lines of the daily cleanup
    /// (`cleanup.start`, `ganghouse.tax`, ...; T-056). `fields` are the figures.
    Audit {
        kind: &'static str,
        fields: Vec<(&'static str, AuditValue)>,
    },
}

/// One figure of a [`GameEvent::Audit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditValue {
    Int(i64),
    Text(String),
}

impl GameEvent {
    /// The `events.kind` and SSE event name.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "connected",
            Self::Closed { .. } => "closed",
            Self::Suicided { .. } => "suicided",
            Self::Rerolled { .. } => "rerolled",
            Self::Level { .. } => "level",
            Self::Audit { kind, .. } => kind,
            Self::Sysop { kind, .. } => kind,
        }
    }

    /// The session the event concerns.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Connected { id }
            | Self::Closed { id }
            | Self::Suicided { id }
            | Self::Rerolled { id }
            | Self::Level { id, .. }
            | Self::Sysop { id, .. } => Some(*id),
            Self::Audit { .. } => None,
        }
    }
}

/// A write for the persistence worker (`silvermere-db`'s `Write` is built from this in
/// `bin`). T-023 and later cards add the variants they save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistRequest {
    /// Save a character record (`save_player`), keyed by account: `characters_per_account`
    /// is 1. The worker upserts it.
    Character {
        account: silvermere_types::ids::AccountId,
        character: Box<silvermere_types::records::Character>,
    },
    /// Delete the account's character (`delete_player`).
    DeleteCharacter {
        account: silvermere_types::ids::AccountId,
    },
    /// Save the account's profile.
    Profile {
        account: silvermere_types::ids::AccountId,
        record: Box<silvermere_types::records::PermanentRecord>,
    },
    /// Save the account's kept evil points (`save_evil_points`).
    EvilPoints {
        account: silvermere_types::ids::AccountId,
        record: silvermere_types::records::EvilRecord,
    },
    /// Save a bank book (marked dirty by `deposit_gold`/`withdraw_gold`, written back when
    /// its buffer is saved).
    BankBook {
        account: silvermere_types::ids::AccountId,
        book: silvermere_types::records::BankBook,
    },
    /// Save a room's live state.
    RoomState(Box<crate::world::RoomState>),
    /// Save a shop's stock levels (marked dirty, written back when its buffer is saved).
    ShopStock {
        shop: silvermere_types::ids::ShopNo,
        qty: [u16; silvermere_types::sizes::NSHOPITEMS],
    },
    /// A copy of a limited item comes into play.
    ItemOwnershipInsert(silvermere_types::records::ItemOwnership),
    /// A copy of a limited item leaves play, by record id.
    ItemOwnershipDelete(i32),
    /// A limited item's count of copies in play (`item.active`).
    ItemActive {
        item: silvermere_types::ids::ItemNo,
        active: u16,
    },
    /// Delete the account's saved profile (the sysop menu's CP, T-055).
    DeleteProfile(silvermere_types::ids::AccountId),
    /// Delete the account's kept evil points (the sysop menu's C, T-055).
    DeleteEvilPoints(silvermere_types::ids::AccountId),
    /// Save a gang record (marked modified, written back when its buffer is saved; inserted
    /// when new, as CREATE does). T-044.
    Gang(silvermere_types::records::Gang),
    /// Save a gang shop's whole record: STOCK and UNSTOCK change the items, prices,
    /// currencies and owner kept in its slots, MARKUP its markup. T-044.
    GangShop(Box<silvermere_types::records::Shop>),
    // --- T-035: monsters ---
    /// Write an active monster (upsert): generated, or changed since its last write.
    ActiveMonster(Box<silvermere_types::records::ActiveMonster>),
    /// Delete an active monster's record (`remove_active_monster`).
    ActiveMonsterDelete(silvermere_types::ids::ActiveMonsterId),
    /// A known monster's live part (`curalive`, `killed_at`; `monster_state`).
    MonsterState {
        number: silvermere_types::ids::MonsterNo,
        curalive: u16,
        killed_at: i64,
    },
    /// Delete a gang record by key (pass 4 of the daily cleanup: a gang with no members or
    /// `GF_DELETE`). T-056.
    DeleteGang(String),
    /// Delete the account row with what cascades from it (`account_roles`, bank books,
    /// evil points, profile), after the worker has written everything queued before it
    /// (`DELETE /v1/accounts/{id}`, T-069).
    DeleteAccount(silvermere_types::ids::AccountId),
    /// Barrier (shutdown): the worker commits everything queued before this, then the
    /// shutdown can go on. `bin` answers it after the persistence worker's flush.
    Flush,
}

/// What comes out of the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineOutput {
    /// The connection got a session.
    Opened {
        conn: ConnId,
        id: SessionId,
    },
    /// Every slot is taken: send the full message and close.
    Refused {
        conn: ConnId,
    },
    /// Bytes for one session, already rendered (CP437, ANSI, `\r\n`).
    Write {
        id: SessionId,
        bytes: Vec<u8>,
    },
    SetEcho {
        id: SessionId,
        mode: EchoMode,
    },
    SetCharMode {
        id: SessionId,
        on: bool,
    },
    /// The session's `Line` is being run now: its output follows. The transport holds back the
    /// line's CR echo until then, so a tick that runs before the line cannot land between
    /// the echo and the output (MBBSEmu ran the input routine in the pass that echoed the CR,
    /// `MbbsHost.cs`:426, :929, and the timers only after it, :458-459; T-094). Only an
    /// engine built with [`Engine::acking_lines`] sends it.
    LineStarted {
        id: SessionId,
    },
    /// The session's last `Line` has been run and its output sent: the transport may take
    /// the input typed behind it. Only an engine built with [`Engine::acking_lines`] sends it.
    LineDone {
        id: SessionId,
    },
    /// Flush queued output, then close.
    Close {
        id: SessionId,
    },
    /// `R` at the BBS menu shim's logoff question: the session is logged out of its
    /// account but stays connected; the transport side starts the login dialogue again
    /// on it (T-021).
    ReturnToLogin {
        id: SessionId,
    },
    Event(GameEvent),
    Persist(PersistRequest),
    /// A new character's first name must be checked against the stored characters
    /// (`characters.by_first_name`, T-003), one indexed query. The answer comes back as
    /// `EngineInput::Custom` running
    /// `state::newchar::validate_name::on_checked(world, id, taken)`.
    NameCheck {
        id: SessionId,
        name: String,
    },
    /// SYSOP RELOAD (T-055): read the game options of the configuration
    /// file again; `bin` answers with `EngineInput::Custom` replacing `world.config`.
    ReloadConfig {
        id: SessionId,
    },
    /// The sysop menu's D and H and REPORT USERS walk every stored character (T-055):
    /// `bin` flushes the writes, reads them all and answers with `EngineInput::Custom`
    /// running `state::sysop_menu::on_roster(world, id, characters)`.
    Roster {
        id: SessionId,
    },
    /// A gang command needs characters that are not in the game: the walks of WCCUSERS
    /// for the member list and the first-name lookups of removing an offline member,
    /// PROMOTE and DEMOTE. After a flush the answer comes back as
    /// `EngineInput::Custom` running [`crate::gangs::on_query_answer`]. T-044.
    GangQuery {
        id: SessionId,
        query: crate::gangs::GangQuery,
    },
    /// The daily cleanup (T-056) needs every stored character with its account. After a
    /// flush the store answers with `EngineInput::Custom` running
    /// [`crate::cleanup::on_scan`]; the cleanup itself then runs in that one input.
    CleanupScan,
    /// An account is being deleted (T-069): after a flush the store reads the account
    /// and its character and answers with `EngineInput::Custom` running
    /// [`session::on_account_scan`].
    AccountDeleteScan {
        account: silvermere_types::ids::AccountId,
    },
    /// The engine stopped; nothing follows.
    Stopped,
}

/// The engine: the world, its clock and random source, and where outputs go.
pub struct Engine<C: Clock, R: Rng> {
    pub world: World,
    pub clock: C,
    /// The engine's `Rng` only seeds `world.rng`, the one random source (T-035).
    rng: std::marker::PhantomData<fn() -> R>,
    outputs: Sender<EngineOutput>,
    /// The last tick's report, for tests and the status line.
    pub last_tick: Option<TickReport>,
    /// Send `LineDone` after each line (the transport holds typed-ahead input until then).
    ack_lines: bool,
    /// A shutdown in progress: where to reply once the logoff saves are out and the barrier
    /// follows them.
    shutdown: Option<(
        Sender<crate::shutdown::ShutdownReport>,
        crate::shutdown::ShutdownReport,
    )>,
}

impl<C: Clock, R: Rng> Engine<C, R> {
    /// An engine over `world`, emitting on `outputs`. `rng` seeds `world.rng`, which
    /// every handler and cycle then draws from.
    pub fn new(mut world: World, clock: C, mut rng: R, outputs: Sender<EngineOutput>) -> Self {
        world.rng = crate::scheduler::SeededRng::new(rng.long(0, i64::MAX) as u64);
        world.boot_minute = minute_of_day(clock.wall().as_second());
        world.set_wall(clock.wall().as_second());
        // The sysop statistics count from start-up until the first CLEAR
        world.sysop.stats_start = world.wall;
        // T-056: the daily cleanup's first run, at the next `game.cleanup.at`
        crate::cleanup::arm(&mut world);
        Self {
            world,
            clock,
            rng: std::marker::PhantomData,
            outputs,
            last_tick: None,
            ack_lines: false,
            shutdown: None,
        }
    }

    /// Acknowledge each handled line with [`EngineOutput::LineDone`].
    pub fn acking_lines(mut self) -> Self {
        self.ack_lines = true;
        self
    }

    /// Runs until [`EngineInput::Shutdown`] arrives or every sender is dropped, then
    /// returns the world. Meant for a dedicated thread (`spawn`).
    ///
    /// A `Tick` waits for the typed input (`Line`, `Key`) queued directly behind it, as
    /// MBBSEmu's host loop ran every channel's input before the timers of its pass
    /// (`MbbsHost.cs`:339-459): a line completed at the same moment as a tick is dispatched
    /// first (T-094). Any other input keeps its place, so a `Custom` sent after ticks still
    /// runs after them; each tick waits at most once.
    pub fn run(mut self, inputs: Receiver<EngineInput>) -> World {
        let mut backlog: std::collections::VecDeque<(EngineInput, bool)> =
            std::collections::VecDeque::new();
        loop {
            let (input, deferred) = match backlog.pop_front() {
                Some(next) => next,
                None => match inputs.recv() {
                    Ok(input) => (input, false),
                    Err(_) => break,
                },
            };
            if matches!(input, EngineInput::Shutdown) {
                break;
            }
            if matches!(input, EngineInput::Tick) && !deferred {
                backlog.extend(inputs.try_iter().map(|i| (i, false)));
                let typed = backlog
                    .iter()
                    .take_while(|(i, _)| {
                        matches!(i, EngineInput::Line { .. } | EngineInput::Key { .. })
                    })
                    .count();
                if typed > 0 {
                    backlog.insert(typed, (input, true));
                    continue;
                }
            }
            self.handle(input);
        }
        session::on_shutdown(&mut self.world);
        let _ = self.outputs.send(EngineOutput::Stopped);
        self.world
    }

    /// Handles one input: the handler inside `catch_unwind`, then the flush. A panic
    /// leaves the world as the handler left it, logs the input, and sends `Command
    /// interrupted.` (the original's wording) to the session concerned, nothing else.
    pub fn handle(&mut self, input: EngineInput) {
        let session = input.session();
        let kind = input.kind();
        let line_id = match &input {
            EngineInput::Line { id, .. } => Some(*id),
            _ => None,
        };
        // Only a line's text is worth keeping for the log, and only when it was a game
        // command: outside `Playing` it may be a password.
        let line = match &input {
            EngineInput::Line { text, .. } => Some(text.clone()),
            _ => None,
        };
        if self.ack_lines
            && let Some(id) = line_id
        {
            let _ = self.outputs.send(EngineOutput::LineStarted { id });
        }
        let result = catch_unwind(AssertUnwindSafe(|| self.dispatch(input)));
        if result.is_err() {
            let playing = session
                .and_then(|id| self.world.session(id))
                .is_some_and(|s| s.state == silvermere_types::enums::SessionState::Playing);
            let line = line
                .filter(|_| playing)
                .unwrap_or_else(|| "<redacted>".into());
            tracing::error!(session = ?session, input = kind, %line, "handler panicked");
            metrics::counter!(crate::metrics::HANDLER_PANICS_TOTAL).increment(1);
            if let Some(id) = session
                && let Some(s) = self.world.session_mut(id)
            {
                s.output.clear();
                s.output.push(Msg::CommandInterrupted);
            }
        }
        self.flush();
        if self.ack_lines
            && let Some(id) = line_id
        {
            let _ = self.outputs.send(EngineOutput::LineDone { id });
        }
        metrics::gauge!(crate::metrics::SESSIONS).set(self.world.sessions.len() as f64);
    }

    fn dispatch(&mut self, input: EngineInput) {
        let now = self.clock.now();
        self.world.today = day_start(self.clock.wall().as_second());
        self.world.set_wall(self.clock.wall().as_second());
        let world = &mut self.world;
        match input {
            EngineInput::Connected { conn, caps } => match session::on_connect(world, caps, now) {
                Ok(id) => world.opened(conn, id),
                Err(e) => {
                    tracing::info!(%e, "connection refused");
                    world.refused(conn);
                }
            },
            EngineInput::LoggedIn { id, login } => {
                if let Some(s) = world.sessions.get_mut(&id) {
                    s.idle_since = now;
                }
                crate::state::login::logged_in(world, id, *login);
            }
            EngineInput::Line { id, text } => {
                if let Some(s) = world.sessions.get_mut(&id) {
                    s.idle_since = now;
                    s.ust.lastinput.clone_from(&text);
                }
                let today = day_start(self.clock.wall().as_second());
                let outcome = session::on_line(world, id, &text, today);
                self.finish_outcome(id, outcome);
            }
            EngineInput::Key { id, byte } => {
                if let Some(s) = world.sessions.get_mut(&id) {
                    s.idle_since = now;
                }
                let outcome = session::on_key(world, id, byte);
                self.finish_outcome(id, outcome);
            }
            EngineInput::Naws { id, columns, rows } => {
                if let Some(s) = world.sessions.get_mut(&id) {
                    s.caps.columns = columns;
                    s.caps.rows = rows;
                }
            }
            EngineInput::Disconnect { id } => {
                session::on_disconnect(world, id);
                world
                    .outbox
                    .push(EngineOutput::Event(GameEvent::Closed { id }));
            }
            EngineInput::Tick => {
                // Wall time on purpose: the tick duration metric measures the host, and
                // nothing in the game reads it.
                let started = Instant::now();
                let report = scheduler::on_fast_tick(world);
                // the paged lists the per-poll routine advanced (T-027)
                crate::state::listing::on_tick(world);
                // the sysop menu's jobs and the sysop's paged reports (T-055)
                crate::state::sysop_menu::on_tick(world);
                // the store's acknowledgements waiting on logins in progress (T-088)
                crate::player::settle_characters_written(world);
                metrics::histogram!(crate::metrics::TICK_SECONDS).record(started.elapsed());
                self.last_tick = Some(report);
            }
            EngineInput::Admin(req) => self.admin(req),
            EngineInput::DailyCleanup => session::on_daily_cleanup(world),
            EngineInput::Control(crate::shutdown::EngineControl::Shutdown { reply, .. }) => {
                self.shutdown = Some((reply, crate::shutdown::begin(world)));
            }
            EngineInput::Custom { run, .. } => run(world),
            EngineInput::Shutdown => {}
        }
    }

    fn finish_outcome(&mut self, id: SessionId, outcome: Outcome) {
        match outcome {
            Outcome::Continue => {}
            Outcome::Logoff => self.world.logoff(id),
            Outcome::Unimplemented(state) => {
                tracing::warn!(?id, ?state, "input in a state not handled yet");
            }
        }
    }

    fn admin(&mut self, req: AdminRequest) {
        let now = self.clock.now();
        let world = &mut self.world;
        match req {
            AdminRequest::Who { reply } => {
                let _ = reply.send(who_rows(world, now));
            }
            AdminRequest::Broadcast { text, reply } => {
                let _ = reply.send(sysop_speak(world, &text));
            }
            AdminRequest::ReloadContent { snapshot, reply } => {
                let _ = reply.send(world.reload_content(&snapshot));
            }
            AdminRequest::DeleteAccount { account, reply } => {
                session::delete_account(world, account, reply);
            }
        }
    }

    /// Sends, in this order: `Opened`/`Refused` for the event's new connections; every
    /// touched batch as one write per session, in first-touch order; the queued control
    /// outputs (echo, char mode, close, events, persistence). Sessions that logged off
    /// during the event leave last, after their final write and `Close`.
    pub fn flush(&mut self) {
        for out in std::mem::take(&mut self.world.outbox_first) {
            let _ = self.outputs.send(out);
        }
        // The transport echoes at receive time, so an echo or char-mode switch must be in force
        // before the prompt it belongs to is on the wire: a client that answers at once would
        // otherwise be fed its first byte with the old mode.
        let (modes, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.world.outbox)
            .into_iter()
            .partition(|o| {
                matches!(
                    o,
                    EngineOutput::SetEcho { .. } | EngineOutput::SetCharMode { .. }
                )
            });
        self.world.outbox = rest;
        for out in modes {
            let _ = self.outputs.send(out);
        }
        let touched = std::mem::take(&mut self.world.touched);
        for id in touched {
            let Some(s) = self.world.sessions.get_mut(&id) else {
                continue;
            };
            if s.output.is_empty() {
                continue;
            }
            let (items, reprompt) = s.output.take();
            let palette = palette_of(&self.world, id);
            // Each queued prompt carries the values of the moment it was queued
            // (the prompt, then the send); only the re-prompt reads the state now.
            let mut bytes = Vec::new();
            render_items(
                &mut bytes,
                &items,
                palette,
                PromptText::Captured(&self.world),
            );
            if reprompt {
                bytes.extend_from_slice(&prompt_bytes(&self.world, id));
            }
            // `SET UTF8 ON` (T-028): the CP437 bytes as UTF-8 text.
            if self.world.sessions.get(&id).is_some_and(|s| s.caps.utf8) {
                bytes = silvermere_text::cp437::decode(&bytes).into_bytes();
            }
            if !bytes.is_empty() {
                let _ = self.outputs.send(EngineOutput::Write { id, bytes });
            }
        }
        for out in std::mem::take(&mut self.world.outbox) {
            let _ = self.outputs.send(out);
        }
        for id in std::mem::take(&mut self.world.logoffs) {
            session::on_logoff(&mut self.world, id);
        }
        // The logoff saves queued just above.
        let mut saved = 0;
        for out in std::mem::take(&mut self.world.outbox) {
            saved += usize::from(matches!(
                out,
                EngineOutput::Persist(PersistRequest::Character { .. })
            ));
            let _ = self.outputs.send(out);
        }
        // Shutdown: the barrier follows the last save, then the order is answered.
        if let Some((reply, mut report)) = self.shutdown.take() {
            report.saved = saved;
            let _ = self
                .outputs
                .send(EngineOutput::Persist(PersistRequest::Flush));
            let _ = reply.send(report);
        }
    }
}

/// The players `display_scan_of_users` would list (channels in the game), ranked as the
/// original ranks its user list (first names upper-cased and sorted, each player's
/// `listslot` is his index). Names are unique in the
/// game, so the tie-break (channel order) never decides anything.
fn who_rows(world: &World, now: Instant) -> Vec<WhoEntry> {
    let mut rows: Vec<WhoEntry> = world
        .session_ids()
        .into_iter()
        .filter(|&id| world.is_in_game(id))
        .filter_map(|id| {
            let s = world.sessions.get(&id)?;
            let c = s.player()?;
            Some(WhoEntry {
                id,
                listslot: 0,
                first_name: c.firstname.clone(),
                last_name: c.lastname.clone(),
                title: c.title.clone(),
                class: world
                    .classes
                    .get(&c.class)
                    .map_or_else(|| c.class.0.to_string(), |k| k.name.clone()),
                race: world
                    .races
                    .get(&c.race)
                    .map_or_else(|| c.race.0.to_string(), |k| k.name.clone()),
                level: c.level,
                alignment: c.evil,
                gang: c.gang.clone(),
                idle_secs: now.saturating_duration_since(s.idle_since).as_secs(),
                // No card sets `INVISB` yet (T-055 ports the sysop console).
                hidden: false,
            })
        })
        .collect();
    rows.sort_by_cached_key(|r| r.first_name.to_uppercase());
    for (slot, row) in rows.iter_mut().enumerate() {
        row.listslot = slot;
    }
    rows
}

/// The sysop `SPEAK` broadcast: `ANSI_CLRLINE`, the text and `\r` to everyone in the game
/// (no flag mask, the listener's prompt after, no colour prefix). The
/// in-game command leaves the speaker out and tells him `You have spoken.`; the API has
/// no speaker, so everyone is told. Returns the number told.
fn sysop_speak(world: &mut World, text: &str) -> usize {
    let mut prf = ANSI_CLRLINE.to_vec();
    prf.extend_from_slice(&cp437::encode(text));
    prf.push(b'\r');
    let bytes = finish(&prf, None);
    let mut told = 0;
    for id in world.session_ids() {
        if !world.is_in_game(id) {
            continue;
        }
        if let Some(s) = world.session_mut(id) {
            s.output.push_raw(bytes.clone());
            s.push_prompt();
            told += 1;
        }
    }
    told
}

/// Starts the engine on its own thread. The handle yields the world after
/// [`EngineInput::Shutdown`].
pub fn spawn<C: Clock + Send + 'static, R: Rng + Send + 'static>(
    world: World,
    clock: C,
    rng: R,
    inputs: Receiver<EngineInput>,
    outputs: Sender<EngineOutput>,
) -> std::io::Result<std::thread::JoinHandle<World>> {
    std::thread::Builder::new()
        .name("silvermere-engine".into())
        .spawn(move || {
            Engine::new(world, clock, rng, outputs)
                .acking_lines()
                .run(inputs)
        })
}

/// The UTC midnight of the day containing `unix_seconds`: a day as the database stores
/// the day fields (`lastdate`, `deathdate`), and as `load_player` and the
/// profiles compare them.
pub fn day_start(unix_seconds: i64) -> i64 {
    unix_seconds - unix_seconds.rem_euclid(86_400)
}

/// The minute of the (UTC) day, 0..1440: the current time as minutes since midnight. The
/// game's clock is the host's local time; this assumes a UTC host, as the shipped container is.
pub fn minute_of_day(unix_seconds: i64) -> u16 {
    (unix_seconds.rem_euclid(86_400) / 60) as u16
}
