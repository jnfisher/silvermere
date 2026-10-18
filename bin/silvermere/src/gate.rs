//! The login gate: `silvermere-auth`'s login and signup dialogue in front of the engine
//! (T-021).
//!
//! Every connection is opened in the engine at once (it holds a session id from the
//! start, so the session cap and the admin `who` see it), but its lines go to a login
//! task of its own until the dialogue ends in a login. The task then loads the account's
//! character and the gate sends the engine `EngineInput::LoggedIn`; from then on the
//! lines go to the engine. `R` at the BBS menu shim brings the dialogue back
//! ([`GateControl::ReturnToLogin`]). The account is released from the one-session rule when
//! the connection drops or returns to the login.

use std::collections::HashMap;
use std::sync::Arc;

use silvermere_auth::login::{EchoMode as AuthEcho, LoginConfig, Output};
use silvermere_auth::{
    Account, LoginDialogue, LoginService, Perm, Role, State, SystemClock, has_permission, has_role,
};
use silvermere_db::Db;
use silvermere_engine::state::login::Login;
use silvermere_engine::{AccountRef, ConnId};
use silvermere_net::{EchoMode, SessionCommand, SessionEvent, SessionId as NetId, cp437};
use silvermere_types::ids::AccountId;
use tokio::sync::mpsc;

/// The login service the gate uses: the database's accounts, the system clock.
pub type Service = LoginService<Arc<Db>, SystemClock>;

/// What the gate passes on to the engine bridge.
#[derive(Debug)]
pub enum GateEvent {
    /// A transport event for a connection the engine reads.
    Net(SessionEvent),
    /// The dialogue logged the connection's user in.
    LoggedIn(Box<Login>),
}

/// What the engine side asks of the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateControl {
    /// `R` at the BBS menu shim: run the dialogue on this connection again.
    ReturnToLogin(ConnId),
}

/// The net crate's connection number, as the engine's [`ConnId`].
pub fn conn_id(id: NetId) -> ConnId {
    ConnId(u64::from(id.0))
}

struct Conn {
    net: NetId,
    tx: Option<mpsc::Sender<SessionCommand>>,
    /// Lines for the running login task, while there is one.
    login: Option<mpsc::UnboundedSender<String>>,
    /// The account once logged in, to release at the end.
    account: Option<AccountId>,
}

/// Everything the gate needs besides its channels.
pub struct GateDeps {
    pub service: Arc<Service>,
    pub db: Arc<Db>,
    pub config: LoginConfig,
    /// How long the dialogue waits for a line before closing the connection; `None` for
    /// no limit (`net.login_timeout_secs = 0`).
    pub login_timeout: Option<std::time::Duration>,
}

/// Runs the gate until the transport side closes.
pub async fn run(
    mut net: mpsc::Receiver<(NetId, SessionEvent)>,
    mut registry: mpsc::UnboundedReceiver<(NetId, mpsc::Sender<SessionCommand>)>,
    registry_out: mpsc::UnboundedSender<(NetId, mpsc::Sender<SessionCommand>)>,
    events_out: mpsc::Sender<(NetId, GateEvent)>,
    mut control: mpsc::UnboundedReceiver<GateControl>,
    deps: GateDeps,
) {
    let deps = Arc::new(deps);
    let mut conns: HashMap<ConnId, Conn> = HashMap::new();
    let (done_tx, mut done) = mpsc::unbounded_channel::<(ConnId, Box<Login>)>();
    loop {
        tokio::select! {
            Some(entry) = registry.recv() => {
                if !register(&mut conns, &registry_out, entry) {
                    return;
                }
            }
            Some((cid, login)) = done.recv() => {
                let Some(conn) = conns.get_mut(&cid) else {
                    // the connection went away while the character was loading: nobody will
                    // ever log this account off
                    deps.service.online().release(login.account.id);
                    continue;
                };
                conn.login = None;
                conn.account = Some(login.account.id);
                if events_out.send((conn.net, GateEvent::LoggedIn(login))).await.is_err() {
                    return;
                }
            }
            Some(GateControl::ReturnToLogin(cid)) = control.recv() => {
                let Some(conn) = conns.get_mut(&cid) else { continue };
                if let Some(id) = conn.account.take() {
                    deps.service.online().release(id);
                }
                if let Some(tx) = conn.tx.clone() {
                    conn.login = Some(spawn_login(cid, tx, deps.clone(), done_tx.clone()));
                }
            }
            ev = net.recv() => {
                let Some((net_id, ev)) = ev else { return };
                // The transport registers a connection before its first event; take
                // what is queued so `Connected` finds the session's sender.
                while let Ok(entry) = registry.try_recv() {
                    if !register(&mut conns, &registry_out, entry) {
                        return;
                    }
                }
                let cid = conn_id(net_id);
                let conn = conns.entry(cid).or_insert_with(|| Conn {
                    net: net_id, tx: None, login: None, account: None,
                });
                match ev {
                    SessionEvent::Line(bytes) => {
                        if let Some(lines) = &conn.login {
                            // the dialogue runs the line now: its CR echo goes ahead of the reply
                            if let Some(tx) = &conn.tx {
                                let _ = tx.try_send(SessionCommand::LineStarted);
                            }
                            if lines.send(cp437::decode(&bytes)).is_ok() {
                                continue;
                            }
                            conn.login = None;
                        }
                        if events_out.send((net_id, GateEvent::Net(SessionEvent::Line(bytes)))).await.is_err() {
                            return;
                        }
                    }
                    SessionEvent::Connected(caps) => {
                        if let Some(tx) = conn.tx.clone() {
                            conn.login = Some(spawn_login(cid, tx, deps.clone(), done_tx.clone()));
                        }
                        if events_out.send((net_id, GateEvent::Net(SessionEvent::Connected(caps)))).await.is_err() {
                            return;
                        }
                    }
                    SessionEvent::Disconnect(reason) => {
                        if let Some(conn) = conns.remove(&cid)
                            && let Some(id) = conn.account
                        {
                            deps.service.online().release(id);
                        }
                        if events_out.send((net_id, GateEvent::Net(SessionEvent::Disconnect(reason)))).await.is_err() {
                            return;
                        }
                    }
                    other => {
                        if events_out.send((net_id, GateEvent::Net(other))).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Keeps a new connection's command sender and passes it on to the output bridge.
fn register(
    conns: &mut HashMap<ConnId, Conn>,
    registry_out: &mpsc::UnboundedSender<(NetId, mpsc::Sender<SessionCommand>)>,
    (net_id, tx): (NetId, mpsc::Sender<SessionCommand>),
) -> bool {
    let conn = conns.entry(conn_id(net_id)).or_insert_with(|| Conn {
        net: net_id,
        tx: None,
        login: None,
        account: None,
    });
    conn.tx = Some(tx.clone());
    registry_out.send((net_id, tx)).is_ok()
}

fn spawn_login(
    cid: ConnId,
    tx: mpsc::Sender<SessionCommand>,
    deps: Arc<GateDeps>,
    done: mpsc::UnboundedSender<(ConnId, Box<Login>)>,
) -> mpsc::UnboundedSender<String> {
    let (lines_tx, lines) = mpsc::unbounded_channel();
    tokio::spawn(login_task(cid, tx, lines, deps, done));
    lines_tx
}

/// One connection's dialogue: the prompts, each line answered through the login
/// service, and on success the account's character read and handed to the gate.
async fn login_task(
    cid: ConnId,
    tx: mpsc::Sender<SessionCommand>,
    mut lines: mpsc::UnboundedReceiver<String>,
    deps: Arc<GateDeps>,
    done: mpsc::UnboundedSender<(ConnId, Box<Login>)>,
) {
    let mut dialogue = LoginDialogue::new(deps.config.clone());
    if !apply(&tx, deps.service.start(&mut dialogue)).await {
        return;
    }
    loop {
        let line = match deps.login_timeout {
            Some(limit) => match tokio::time::timeout(limit, lines.recv()).await {
                Ok(line) => line,
                Err(_) => {
                    tracing::info!(?cid, "login dialogue idle too long; closing");
                    let _ = tx.send(SessionCommand::Close).await;
                    return;
                }
            },
            None => lines.recv().await,
        };
        let Some(line) = line else { return };
        match deps.service.drive(&mut dialogue, &line).await {
            Ok(out) => {
                // A dialogue that finished has claimed its account (`Service::drive`). From
                // here on, whoever leaves without handing the login to the gate releases it,
                // or the account stays "already logged on" for good.
                let claimed = match dialogue.state() {
                    State::Done(id) => Some(*id),
                    _ => None,
                };
                if !apply(&tx, out).await {
                    if let Some(id) = claimed {
                        deps.service.online().release(id);
                    }
                    return;
                }
                // the line is answered: the input typed behind it may be taken
                let _ = tx.send(SessionCommand::LineDone).await;
            }
            Err(e) => {
                tracing::error!(%e, ?cid, "login failed");
                let _ = tx.send(SessionCommand::Close).await;
                return;
            }
        }
        if matches!(dialogue.state(), State::Done(_)) {
            let Some(account) = dialogue.into_account() else {
                return;
            };
            let id = account.id;
            match login_for(&deps.db, account).await {
                Ok(login) => {
                    if done.send((cid, Box::new(login))).is_err() {
                        deps.service.online().release(id);
                    }
                }
                Err(e) => {
                    tracing::error!(%e, ?cid, "loading the character failed");
                    deps.service.online().release(id);
                    let _ = tx.send(SessionCommand::Close).await;
                }
            }
            return;
        }
    }
}

/// The engine's view of a logged-in account, with its character.
async fn login_for(db: &Db, account: Account) -> anyhow::Result<Login> {
    let character = db
        .characters()
        .by_account(account.id)
        .await?
        .map(|row| Box::new(row.character));
    Ok(Login {
        account: AccountRef {
            id: account.id,
            username: account.username.clone(),
            sysop: has_role(&account, Role::Sysop),
            gameop: has_role(&account, Role::GameOp),
            sex: account.sex.as_byte(),
        },
        character,
        may_gossip: has_permission(&account, Perm::Gossip),
        may_auction: has_permission(&account, Perm::Auction),
        sysop_keys: sysop_keys(&account),
    })
}

/// The `SYS_*` INI keys and the developer gate the account holds, as the engine names
/// them (T-055).
fn sysop_keys(account: &Account) -> silvermere_engine::commands::sysop::Keys {
    use silvermere_engine::commands::sysop::Key;
    Perm::ALL
        .iter()
        .copied()
        .filter(|p| has_permission(account, *p))
        .filter_map(|p| Key::from_ini_name(p.ini_name()))
        .collect()
}

/// Sends the dialogue's outputs to the session; `false` once it asked to close.
async fn apply(tx: &mpsc::Sender<SessionCommand>, out: Vec<Output>) -> bool {
    for o in out {
        let cmd = match o {
            Output::Raw(bytes) => SessionCommand::Write(bytes),
            Output::Text(m) => SessionCommand::Write(silvermere_text::cp437::encode(&m.text())),
            // MBBSEmu's `wid = 0xFF`: one `*` per byte with no practical limit
            // (`docs/auth.md`).
            Output::SetEcho(AuthEcho::Secure { ch }) => {
                SessionCommand::SetEcho(EchoMode::Secure { ch, width: 0xFF })
            }
            Output::SetEcho(AuthEcho::On) => SessionCommand::SetEcho(EchoMode::On),
            Output::Close => {
                let _ = tx.send(SessionCommand::Close).await;
                return false;
            }
            // `LoginService::drive` answers every request itself.
            Output::Request(_) => continue,
        };
        if tx.send(cmd).await.is_err() {
            return false;
        }
    }
    true
}

/// No gate: every event goes to the engine as it comes (a server started without a
/// database, [`crate::serve_world`]).
pub async fn pass_through(
    mut net: mpsc::Receiver<(NetId, SessionEvent)>,
    mut registry: mpsc::UnboundedReceiver<(NetId, mpsc::Sender<SessionCommand>)>,
    registry_out: mpsc::UnboundedSender<(NetId, mpsc::Sender<SessionCommand>)>,
    events_out: mpsc::Sender<(NetId, GateEvent)>,
) {
    loop {
        tokio::select! {
            Some(entry) = registry.recv() => {
                if registry_out.send(entry).is_err() {
                    return;
                }
            }
            ev = net.recv() => {
                let Some((id, ev)) = ev else { return };
                if events_out.send((id, GateEvent::Net(ev))).await.is_err() {
                    return;
                }
            }
        }
    }
}
