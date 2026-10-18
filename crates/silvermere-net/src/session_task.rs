//! The per-connection session task and the accept loop.
//!
//! A session task owns its socket, the telnet codec, the line editor and the output queue.
//! It turns bytes into [`SessionEvent`]s for the engine and drains [`SessionCommand`]s into
//! the socket. The engine never sees a socket and never waits on one: a client that stops
//! reading is disconnected once its queue passes the bound.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::{Instant, sleep_until};

use crate::events::{Reason, SessionCommand, SessionEvent, SessionId};
use crate::line_editor::{EditorEvent, LineEditor};
use crate::telnet::parser::{IAC, NOP};
use crate::telnet::{TelnetCodec, escape_into};

/// Commands the engine may have in flight to one session before `try_send` reports full.
const COMMAND_CAPACITY: usize = 256;
/// How long a `Close` waits for queued output to drain.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
const READ_CHUNK: usize = 4096;
/// How long the bytes after a completed line wait for the engine to say it has run the line
/// (`SessionCommand::LineDone`). The host the original ran on handled one line before it
/// echoed the next, so a line that switches the echo off (`add_delay`) is not followed by the
/// echo of the lines typed ahead of it. A line the engine never acknowledges (the login
/// dialogue) only delays the bytes behind it by this long.
const LINE_HOLD: Duration = Duration::from_millis(250);

/// Per-session settings.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Output bytes queued (after escaping) before the session is dropped as backpressure.
    pub output_limit: usize,
    /// Send `IAC NOP` after this long without output. Off by default, as in our MBBSEmu
    /// configuration (`Telnet.Heartbeat` false).
    pub heartbeat: Option<Duration>,
    /// Hold a completed line's CR echo until the engine says it is running the line
    /// (`SessionCommand::LineStarted`, or `LineDone`, `Close` or the [`LINE_HOLD`] timeout),
    /// so no output the engine sends before then lands between the echo and the line's own
    /// output. MBBSEmu echoed the CR in the pass that ran the module's input routine, and ran
    /// the timers only after it (`MbbsHost.cs`:426, :458-459, :929; T-094). Off by default:
    /// a peer that never sends `LineStarted` would see each echo late.
    pub hold_line_echo: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            output_limit: 64 * 1024,
            heartbeat: None,
            hold_line_echo: false,
        }
    }
}

/// Accept-loop settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Connections beyond this get `full_message` and are closed.
    pub max_sessions: usize,
    /// `compat.full_message`.
    pub full_message: Vec<u8>,
    /// `SO_SNDBUF` for accepted sockets (set on the listener by [`bind`]; Linux sockets
    /// inherit it). Without a cap the kernel buffers megabytes for a client that stopped
    /// reading, and the output-queue bound would take that long to notice.
    pub socket_send_buffer: u32,
    pub session: SessionConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_sessions: 64,
            full_message: b"The server is full. Please try again later.\r\n".to_vec(),
            socket_send_buffer: 32 * 1024,
            session: SessionConfig::default(),
        }
    }
}

/// Starts a session task with the default [`SessionConfig`].
pub fn spawn_session(
    id: SessionId,
    stream: TcpStream,
    to_engine: mpsc::Sender<(SessionId, SessionEvent)>,
) -> mpsc::Sender<SessionCommand> {
    spawn_session_with(id, stream, to_engine, SessionConfig::default())
}

/// Starts a session task. The returned sender carries engine commands; the engine should
/// use `try_send` so a stuck session can never block it.
pub fn spawn_session_with(
    id: SessionId,
    stream: TcpStream,
    to_engine: mpsc::Sender<(SessionId, SessionEvent)>,
    config: SessionConfig,
) -> mpsc::Sender<SessionCommand> {
    let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
    tokio::spawn(run(id, stream, to_engine, rx, config, None));
    tx
}

/// Binds the listening socket with the send-buffer cap from `config`.
pub fn bind(addr: SocketAddr, config: &ServerConfig) -> io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.set_send_buffer_size(config.socket_send_buffer)?;
    socket.bind(addr)?;
    socket.listen(128)
}

/// Accepts connections until the listener fails. Each new session's command sender is
/// sent on `registry` before its task starts, so by the time the engine sees that
/// session's `Connected`, the registration is already waiting in `registry`.
pub async fn serve(
    listener: TcpListener,
    config: ServerConfig,
    to_engine: mpsc::Sender<(SessionId, SessionEvent)>,
    registry: mpsc::UnboundedSender<(SessionId, mpsc::Sender<SessionCommand>)>,
) -> io::Result<()> {
    let slots = Arc::new(Semaphore::new(config.max_sessions));
    let mut next_id = 0u32;
    loop {
        let (mut stream, peer) = listener.accept().await?;
        // Small writes (echo, prompts) must not wait on Nagle.
        let _ = stream.set_nodelay(true);
        let Ok(permit) = Arc::clone(&slots).try_acquire_owned() else {
            tracing::info!(%peer, "server full; connection refused");
            let message = config.full_message.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(CLOSE_GRACE, async {
                    let _ = stream.write_all(&message).await;
                    let _ = stream.shutdown().await;
                })
                .await;
            });
            continue;
        };
        next_id = next_id.wrapping_add(1);
        let id = SessionId(next_id);
        let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
        if registry.send((id, tx)).is_err() {
            // The engine is gone; nothing will ever read this session's events.
            return Ok(());
        }
        tracing::info!(%peer, ?id, "connection accepted");
        tokio::spawn(run(
            id,
            stream,
            to_engine.clone(),
            rx,
            config.session.clone(),
            Some(permit),
        ));
    }
}

async fn run(
    id: SessionId,
    stream: TcpStream,
    to_engine: mpsc::Sender<(SessionId, SessionEvent)>,
    commands: mpsc::Receiver<SessionCommand>,
    config: SessionConfig,
    permit: Option<OwnedSemaphorePermit>,
) {
    let reason = Session::new(stream, config)
        .drive(id, &to_engine, commands)
        .await;
    match &reason {
        Reason::Backpressure => {
            tracing::warn!(
                ?id,
                "client not reading; output queue over limit, disconnected"
            );
        }
        other => tracing::debug!(?id, reason = ?other, "session ended"),
    }
    // Free the slot before the engine hears about it, so a reconnect is not refused.
    drop(permit);
    let _ = to_engine.send((id, SessionEvent::Disconnect(reason))).await;
}

struct Session {
    /// Taken (and split) when the session starts driving.
    stream: Option<TcpStream>,
    config: SessionConfig,
    codec: TelnetCodec,
    editor: LineEditor,
    /// Decoded input not yet fed to the editor (held while the editor's queue is full).
    inbox: VecDeque<u8>,
    /// Engine-bound events that are not editor output (NAWS).
    outbox: VecDeque<SessionEvent>,
    /// Escaped bytes waiting for the socket.
    out: Vec<u8>,
    last_output: Instant,
    /// Input behind the line just sent waits until this instant or a `LineDone`.
    hold_until: Option<Instant>,
    /// The CR echo of the line the engine has not started yet (`hold_line_echo`).
    held_echo: Vec<u8>,
}

impl Session {
    fn new(stream: TcpStream, config: SessionConfig) -> Self {
        Self {
            stream: Some(stream),
            config,
            codec: TelnetCodec::new(),
            editor: LineEditor::new(),
            inbox: VecDeque::new(),
            outbox: VecDeque::new(),
            out: TelnetCodec::offers().to_vec(),
            last_output: Instant::now(),
            hold_until: None,
            held_echo: Vec::new(),
        }
    }

    /// One command from the engine.
    fn apply(&mut self, command: Option<SessionCommand>, close_by: &mut Option<Instant>) {
        match command {
            Some(SessionCommand::Write(bytes)) => escape_into(&bytes, &mut self.out),
            Some(SessionCommand::SetEcho(mode)) => self.editor.set_echo(mode),
            Some(SessionCommand::SetCharMode(on)) => self.editor.set_char_mode(on),
            Some(SessionCommand::LineStarted) => self.release_echo(),
            Some(SessionCommand::LineDone) => {
                self.release_echo();
                self.hold_until = None;
            }
            Some(SessionCommand::Close) | None => {
                self.release_echo();
                *close_by = Some(Instant::now() + CLOSE_GRACE);
            }
        }
    }

    /// Sends the held CR echo, if any.
    fn release_echo(&mut self) {
        escape_into(&std::mem::take(&mut self.held_echo), &mut self.out);
    }

    /// Runs until the session ends; returns why.
    async fn drive(
        mut self,
        id: SessionId,
        to_engine: &mpsc::Sender<(SessionId, SessionEvent)>,
        mut commands: mpsc::Receiver<SessionCommand>,
    ) -> Reason {
        // Negotiation runs alongside the login prompt, never before it.
        let connected = SessionEvent::Connected(self.codec.capabilities());
        if to_engine.send((id, connected)).await.is_err() {
            return Reason::Closed;
        }
        let mut raw = vec![0u8; READ_CHUNK];
        let mut eof = false;
        let mut close_by: Option<Instant> = None;
        let Some(stream) = self.stream.take() else {
            return Reason::Closed;
        };
        let (mut rd, mut wr) = stream.into_split();

        loop {
            // Commands that are already waiting go first, so that the engine's echo switch
            // that follows a prompt is in force before the reply to that prompt is read, and
            // so that a `LineDone` among them releases the hold before the held input is fed:
            // clearing the hold after the feed step left the input in the inbox with nothing
            // armed to wake the task (reading is off while the inbox is non-empty, and the hold
            // timer is off once the hold is cleared).
            while close_by.is_none() {
                match commands.try_recv() {
                    Ok(command) => self.apply(Some(command), &mut close_by),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.apply(None, &mut close_by);
                    }
                }
            }

            // Feed held input while the editor has room; echo goes out at receive time (a CR's
            // echo, with `hold_line_echo`, when the engine starts the line).
            let mut echo = Vec::new();
            if self.hold_until.is_some_and(|t| t <= Instant::now()) {
                self.release_echo();
                self.hold_until = None;
            }
            // one completed line at a time, and none while the last is being run
            while self.hold_until.is_none() && !self.editor.is_full() && !self.editor.has_event() {
                let Some(b) = self.inbox.pop_front() else {
                    break;
                };
                let mark = echo.len();
                self.editor.feed(b, &mut echo);
                if self.config.hold_line_echo && self.editor.line_waiting() {
                    // the CR echo waits for the engine to start the line
                    self.held_echo.extend(echo.drain(mark..));
                }
            }
            escape_into(&echo, &mut self.out);
            if self.out.len() > self.config.output_limit {
                return Reason::Backpressure;
            }

            let has_event = !self.outbox.is_empty() || self.editor.has_event();
            if eof && !has_event && self.inbox.is_empty() {
                return Reason::Eof;
            }
            if close_by.is_some() && self.out.is_empty() {
                let _ = wr.shutdown().await;
                return Reason::Closed;
            }
            let can_read = !eof && close_by.is_none() && self.inbox.is_empty();
            let heartbeat_at = self
                .config
                .heartbeat
                .filter(|_| self.out.is_empty() && close_by.is_none())
                .map(|d| self.last_output + d);

            tokio::select! {
                read = rd.read(&mut raw), if can_read => match read {
                    Ok(0) => eof = true,
                    Ok(n) => {
                        let mut reply = Vec::new();
                        if let Some(caps) = self.codec.decode(&raw[..n], &mut self.inbox, &mut reply) {
                            self.outbox.push_back(SessionEvent::Naws(caps.columns, caps.rows));
                        }
                        self.out.extend_from_slice(&reply);
                    }
                    Err(e) if is_drop(&e) => eof = true,
                    Err(e) => return Reason::Io(e.kind()),
                },
                written = wr.write(&self.out), if !self.out.is_empty() => match written {
                    Ok(0) => return Reason::Eof,
                    Ok(n) => {
                        self.out.drain(..n);
                        self.last_output = Instant::now();
                    }
                    Err(e) if is_drop(&e) => return Reason::Eof,
                    Err(e) => return Reason::Io(e.kind()),
                },
                permit = to_engine.reserve(), if has_event => {
                    let Ok(permit) = permit else { return Reason::Closed };
                    let event = self
                        .outbox
                        .pop_front()
                        .or_else(|| self.editor.next_event().map(to_session_event));
                    if let Some(event) = event {
                        if matches!(event, SessionEvent::Line(_)) {
                            self.hold_until = Some(Instant::now() + LINE_HOLD);
                        }
                        permit.send((id, event));
                    }
                }
                command = commands.recv(), if close_by.is_none() => self.apply(command, &mut close_by),
                () = sleep_until(self.hold_until.unwrap_or_else(Instant::now)), if self.hold_until.is_some() => {
                    self.release_echo();
                    self.hold_until = None;
                }
                () = sleep_until(close_by.unwrap_or_else(Instant::now)), if close_by.is_some() => {
                    return Reason::Closed;
                }
                () = sleep_until(heartbeat_at.unwrap_or_else(Instant::now)), if heartbeat_at.is_some() => {
                    self.out.extend_from_slice(&[IAC, NOP]);
                }
            }
        }
    }
}

fn to_session_event(event: EditorEvent) -> SessionEvent {
    match event {
        EditorEvent::Line(line) => SessionEvent::Line(line),
        EditorEvent::LineTooLong => SessionEvent::LineTooLong,
        EditorEvent::Key(k) => SessionEvent::Key(k),
    }
}

/// Errors that just mean the client went away.
fn is_drop(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
    )
}
