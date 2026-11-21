//! The sysop's moderation, shared by the admin API (and so `silvermere sysop`) and, later,
//! the web console: [`Operator`] stores each sanction, then tells the running game and the
//! login. [`IpBans`] is the list the telnet listener checks at accept; [`restore`] reads the
//! closed realm and the IP bans back at start.

use std::net::IpAddr;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use jiff::Timestamp;
use serde_json::{Value, json};
use silvermere_api::events::BoxFuture;
use silvermere_api::moderation::{Action, Kind, ModError};
use silvermere_auth::RealmGate;
use silvermere_db::Db;
use silvermere_db::repo::{RealmClosed, Sanction, SanctionKind};
use silvermere_engine::moderation::{Kind as EngineKind, Sanction as EngineSanction};
use silvermere_engine::{AdminRequest, EngineInput};
use silvermere_net::Network;
use silvermere_types::ids::AccountId;

/// How long an action waits for the engine thread.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(5);

/// The IP bans in force, as the telnet listener checks them.
#[derive(Debug, Default)]
pub struct IpBans(RwLock<Vec<(Network, Option<Timestamp>)>>);

impl IpBans {
    fn set(&self, bans: Vec<(Network, Option<Timestamp>)>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = bans;
    }

    /// Reads the bans from the store.
    pub async fn load(&self, db: &Db) -> silvermere_db::Result<()> {
        let mut bans = Vec::new();
        for b in db.moderation().ip_bans().await? {
            match b.network.parse::<Network>() {
                Ok(net) => bans.push((net, b.until)),
                Err(e) => tracing::warn!(%e, "ip_bans row ignored"),
            }
        }
        self.set(bans);
        Ok(())
    }
}

impl silvermere_net::IpFilter for IpBans {
    fn refuses(&self, ip: IpAddr) -> bool {
        let now = Timestamp::now();
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(net, until)| until.is_none_or(|u| u > now) && net.contains(ip))
    }
}

/// At start: a realm closed before the restart stays closed, and the IP bans are loaded.
pub async fn restore(db: &Db, realm: &RealmGate, ip_bans: &IpBans) -> silvermere_db::Result<()> {
    if let Some(closed) = db.moderation().realm_closed().await? {
        tracing::info!(reason = %closed.reason, by = %closed.by_whom, "the realm is closed: only sysops may log in");
        realm.close(&closed.reason);
    }
    ip_bans.load(db).await
}

/// Does the moderation actions: the store first, then the game and the login.
pub struct Operator {
    db: Arc<Db>,
    engine: Sender<EngineInput>,
    realm: Arc<RealmGate>,
    ip_bans: Arc<IpBans>,
    /// The players' screens and their connections, for watching a player and typing for
    /// him; `None` for a server without them.
    watching: Option<(Arc<silvermere_net::Screens>, Arc<crate::spectate::Links>)>,
}

/// A player as a sysop names him.
struct Player {
    account: AccountId,
    /// The character's first name, else the account's username.
    name: String,
    door: bool,
}

fn store(e: silvermere_db::DbError) -> ModError {
    ModError::Store(e.to_string())
}

fn stored_kind(kind: Kind) -> SanctionKind {
    match kind {
        Kind::Ban => SanctionKind::Ban,
        Kind::Jail => SanctionKind::Jail,
        Kind::Mute => SanctionKind::Mute,
    }
}

fn rfc(t: Option<Timestamp>) -> Value {
    t.map_or(Value::Null, |t| Value::String(t.to_string()))
}

fn until_of(now: Timestamp, for_secs: Option<u64>) -> Option<Timestamp> {
    for_secs.and_then(|s| {
        now.checked_add(jiff::SignedDuration::from_secs(s as i64))
            .ok()
    })
}

impl Operator {
    pub fn new(
        db: Arc<Db>,
        engine: Sender<EngineInput>,
        realm: Arc<RealmGate>,
        ip_bans: Arc<IpBans>,
    ) -> Self {
        Self {
            db,
            engine,
            realm,
            ip_bans,
            watching: None,
        }
    }

    /// The operator with the players' screens and connections to watch and type through.
    pub fn watching(
        mut self,
        screens: Arc<silvermere_net::Screens>,
        links: Arc<crate::spectate::Links>,
    ) -> Self {
        self.watching = Some((screens, links));
        self
    }

    /// The player in the realm `name` means, and his connection.
    async fn connection(
        &self,
        name: &str,
    ) -> Result<
        (
            String,
            silvermere_net::SessionId,
            tokio::sync::mpsc::Sender<silvermere_net::SessionCommand>,
        ),
        ModError,
    > {
        let Some((_, links)) = &self.watching else {
            return Err(ModError::Unavailable);
        };
        let p = self.player(name).await?;
        let (reply, rx) = std::sync::mpsc::channel();
        let who = self.ask(AdminRequest::Who { reply }, rx).await?;
        let row = who
            .into_iter()
            .find(|r| r.account == Some(p.account))
            .ok_or(ModError::NotConnected)?;
        let (net, tx) = links.of(row.id).ok_or(ModError::NotConnected)?;
        Ok((row.first_name, net, tx))
    }

    /// The player's screen as it stands and what follows.
    pub async fn watch(&self, name: &str) -> Result<silvermere_api::Watch, ModError> {
        let (name, net, _) = self.connection(name).await?;
        let Some((screens, _)) = &self.watching else {
            return Err(ModError::Unavailable);
        };
        let (recent, mut bytes) = screens.watch(net).ok_or(ModError::NotConnected)?;
        let (tx, stream) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            while let Some(b) = bytes.recv().await {
                if tx.send(crate::spectate::screen_text(&b)).await.is_err() {
                    return;
                }
            }
        });
        Ok(silvermere_api::Watch {
            name,
            recent: crate::spectate::screen_text(&recent),
            stream,
        })
    }

    /// Types `line` for the player, as if he had.
    async fn input(&self, name: &str, line: &str) -> Result<Value, ModError> {
        let (name, _, tx) = self.connection(name).await?;
        tx.try_send(silvermere_net::SessionCommand::Inject(
            silvermere_text::cp437::encode(line),
        ))
        .map_err(|_| ModError::NotConnected)?;
        Ok(json!({ "player": name, "line": line }))
    }

    /// One in-game `SYSOP` line for `by`: a console sysop (`web:<name>`) with his account's
    /// keys, a token with a sysop's default keys.
    async fn sysop_line(&self, line: &str, by: &str) -> Result<Value, ModError> {
        use silvermere_engine::commands::sysop::{ConsoleRefusal, Keys};
        let (name, keys) = match by.strip_prefix("web:") {
            Some(user) => {
                let account = silvermere_auth::AccountStore::by_username(self.db.as_ref(), user)
                    .await
                    .map_err(|e| ModError::Store(e.to_string()))?;
                let keys = match account {
                    Some(a) => crate::gate::sysop_keys(&a),
                    None => Keys::for_roles(true, false),
                };
                (user.to_owned(), keys)
            }
            None => (by.to_owned(), Keys::for_roles(true, false)),
        };
        let (reply, rx) = std::sync::mpsc::channel();
        let req = AdminRequest::SysopLine {
            name,
            keys,
            line: line.to_owned(),
            reply,
        };
        match self.ask(req, rx).await? {
            Ok(bytes) => Ok(json!({ "output": crate::spectate::screen_text(&bytes) })),
            Err(ConsoleRefusal::NotSysop) => Err(ModError::NotSysopLine),
            Err(ConsoleRefusal::NeedsCharacter) => Err(ModError::NeedsCharacter),
            Err(ConsoleRefusal::ServerFull) => Err(ModError::ServerFull),
        }
    }

    /// Sends `req` to the engine and waits for its answer.
    async fn ask<T: Send + 'static>(
        &self,
        req: AdminRequest,
        rx: Receiver<T>,
    ) -> Result<T, ModError> {
        self.engine
            .send(EngineInput::Admin(req))
            .map_err(|_| ModError::Engine("engine_unavailable"))?;
        tokio::task::spawn_blocking(move || rx.recv_timeout(ENGINE_TIMEOUT))
            .await
            .map_err(|_| ModError::Engine("engine_unavailable"))?
            .map_err(|e| match e {
                RecvTimeoutError::Timeout => ModError::Engine("engine_timeout"),
                RecvTimeoutError::Disconnected => ModError::Engine("engine_unavailable"),
            })
    }

    /// The account a name means: an account's username first, else a character's first
    /// name, ignoring case.
    async fn player(&self, name: &str) -> Result<Player, ModError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ModError::NoSuchPlayer);
        }
        let accounts = self.db.accounts();
        let row = match accounts.by_username(name).await.map_err(store)? {
            Some(row) => row,
            None => {
                let Some(c) = self
                    .db
                    .characters()
                    .by_first_name(name)
                    .await
                    .map_err(store)?
                else {
                    return Err(ModError::NoSuchPlayer);
                };
                accounts
                    .get(c.account)
                    .await
                    .map_err(store)?
                    .ok_or(ModError::NoSuchPlayer)?
            }
        };
        let character = self
            .db
            .characters()
            .by_account(row.id)
            .await
            .map_err(store)?
            .map(|c| c.character.firstname)
            .filter(|n| !n.is_empty());
        Ok(Player {
            account: row.id,
            name: character.unwrap_or(row.account.username),
            door: row.account.flags & silvermere_db::repo::accounts::FLAG_DOOR != 0,
        })
    }

    async fn kick(&self, player: &str, reason: String) -> Result<Value, ModError> {
        let p = self.player(player).await?;
        let (reply, rx) = std::sync::mpsc::channel();
        let req = AdminRequest::Kick {
            account: p.account,
            reason,
            reply,
        };
        match self.ask(req, rx).await? {
            Some(name) => Ok(json!({ "kicked": name })),
            None => Err(ModError::NotConnected),
        }
    }

    async fn tell(&self, player: &str, text: String) -> Result<Value, ModError> {
        let p = self.player(player).await?;
        let (reply, rx) = std::sync::mpsc::channel();
        let req = AdminRequest::Tell {
            account: p.account,
            text,
            reply,
        };
        match self.ask(req, rx).await? {
            Some(name) => Ok(json!({ "told": name })),
            None => Err(ModError::NotConnected),
        }
    }

    async fn put(
        &self,
        player: &str,
        kind: Kind,
        for_secs: Option<u64>,
        reason: String,
        by: String,
    ) -> Result<Value, ModError> {
        let p = self.player(player).await?;
        if kind == Kind::Ban && p.door {
            return Err(ModError::DoorAccount);
        }
        let now = Timestamp::now();
        let until = until_of(now, for_secs);
        self.db
            .moderation()
            .set(&Sanction {
                account: p.account,
                kind: stored_kind(kind),
                until,
                reason: reason.clone(),
                by_whom: by,
                created: now,
            })
            .await
            .map_err(store)?;
        let online = match kind {
            // a banned player in the game is disconnected, told why
            Kind::Ban => {
                let (reply, rx) = std::sync::mpsc::channel();
                // "Reason: banned until 2026-12-01 18:00 UTC: spamming gossip"
                let mut text = format!(
                    "banned{}",
                    silvermere_engine::moderation::until_text(until.map(|u| u.as_second()))
                );
                if !reason.is_empty() {
                    text.push_str(&format!(": {reason}"));
                }
                let req = AdminRequest::Kick {
                    account: p.account,
                    reason: text,
                    reply,
                };
                self.ask(req, rx).await?.is_some()
            }
            Kind::Jail | Kind::Mute => {
                let (reply, rx) = std::sync::mpsc::channel();
                let req = AdminRequest::Sanction {
                    account: p.account,
                    kind: if kind == Kind::Jail {
                        EngineKind::Jail
                    } else {
                        EngineKind::Mute
                    },
                    sanction: Some(EngineSanction {
                        until: until.map(|u| u.as_second()),
                        reason,
                    }),
                    reply,
                };
                self.ask(req, rx).await?.is_some()
            }
        };
        Ok(json!({
            "player": p.name,
            "account": p.account.0,
            "kind": kind.as_str(),
            "until": rfc(until),
            "online": online,
        }))
    }

    async fn lift(&self, player: &str, kind: Kind) -> Result<Value, ModError> {
        let p = self.player(player).await?;
        let lifted = self
            .db
            .moderation()
            .lift(p.account, stored_kind(kind))
            .await
            .map_err(store)?;
        if !lifted {
            return Err(ModError::NotSanctioned);
        }
        let online = match kind {
            Kind::Ban => false,
            Kind::Jail | Kind::Mute => {
                let (reply, rx) = std::sync::mpsc::channel();
                let req = AdminRequest::Sanction {
                    account: p.account,
                    kind: if kind == Kind::Jail {
                        EngineKind::Jail
                    } else {
                        EngineKind::Mute
                    },
                    sanction: None,
                    reply,
                };
                self.ask(req, rx).await?.is_some()
            }
        };
        Ok(json!({
            "player": p.name,
            "account": p.account.0,
            "kind": kind.as_str(),
            "lifted": true,
            "online": online,
        }))
    }

    async fn ban_ip(
        &self,
        network: &str,
        for_secs: Option<u64>,
        reason: String,
        by: String,
    ) -> Result<Value, ModError> {
        let net: Network = network.parse().map_err(|_| ModError::BadNetwork)?;
        let canonical = net.to_string();
        let now = Timestamp::now();
        let until = until_of(now, for_secs);
        let id = self
            .db
            .moderation()
            .ban_ip(&canonical, until, &reason, &by, now)
            .await
            .map_err(store)?;
        self.ip_bans.load(&self.db).await.map_err(store)?;
        let kicked = self.kick_from(&net, until, &reason).await?;
        Ok(json!({ "id": id, "network": canonical, "until": rfc(until), "kicked": kicked }))
    }

    /// Disconnects the players in the realm whose telnet connection comes from `net`, told
    /// why and saved first. Door callers all come from the BBS and are never checked.
    async fn kick_from(
        &self,
        net: &Network,
        until: Option<Timestamp>,
        reason: &str,
    ) -> Result<Vec<String>, ModError> {
        let (reply, rx) = std::sync::mpsc::channel();
        let who = self.ask(AdminRequest::Who { reply }, rx).await?;
        let mut text = format!(
            "banned{}",
            silvermere_engine::moderation::until_text(until.map(|u| u.as_second()))
        );
        if !reason.is_empty() {
            text.push_str(&format!(": {reason}"));
        }
        let mut kicked = Vec::new();
        for row in who {
            let (Some(ip), Some(account)) = (row.ip, row.account) else {
                continue;
            };
            if !net.contains(silvermere_net::network::canonical(ip)) {
                continue;
            }
            let door = self
                .db
                .accounts()
                .get(account)
                .await
                .map_err(store)?
                .is_some_and(|a| a.account.flags & silvermere_db::repo::accounts::FLAG_DOOR != 0);
            if door {
                continue;
            }
            let (reply, rx) = std::sync::mpsc::channel();
            let req = AdminRequest::Kick {
                account,
                reason: text.clone(),
                reply,
            };
            if let Some(name) = self.ask(req, rx).await? {
                kicked.push(name);
            }
        }
        Ok(kicked)
    }

    async fn unban_ip(&self, network: &str) -> Result<Value, ModError> {
        let net: Network = network.parse().map_err(|_| ModError::BadNetwork)?;
        let canonical = net.to_string();
        if !self
            .db
            .moderation()
            .unban_ip(&canonical)
            .await
            .map_err(store)?
        {
            return Err(ModError::NoSuchBan);
        }
        self.ip_bans.load(&self.db).await.map_err(store)?;
        Ok(json!({ "network": canonical, "lifted": true }))
    }

    async fn close_realm(&self, reason: String, by: String) -> Result<Value, ModError> {
        self.db
            .moderation()
            .close_realm(&RealmClosed {
                reason: reason.clone(),
                by_whom: by,
                created: Timestamp::now(),
            })
            .await
            .map_err(store)?;
        // the login first, so nobody gets in behind the disconnections
        self.realm.close(&reason);
        let (reply, rx) = std::sync::mpsc::channel();
        let disconnected = self
            .ask(AdminRequest::CloseRealm { reason, reply }, rx)
            .await?;
        Ok(json!({ "closed": true, "disconnected": disconnected }))
    }

    async fn open_realm(&self) -> Result<Value, ModError> {
        let was_closed = self.db.moderation().open_realm().await.map_err(store)?;
        self.realm.open();
        Ok(json!({ "closed": false, "was_closed": was_closed }))
    }

    /// The realm, the sanctions in force (with the player's name) and the IP bans.
    pub async fn snapshot(&self) -> Result<Value, ModError> {
        let m = self.db.moderation();
        let now = Timestamp::now();
        let realm = match m.realm_closed().await.map_err(store)? {
            Some(c) => {
                json!({ "closed": true, "reason": c.reason, "by": c.by_whom, "at": c.created.to_string() })
            }
            None => json!({ "closed": false }),
        };
        let mut sanctions = Vec::new();
        for s in m.active(now).await.map_err(store)? {
            let account = self.db.accounts().get(s.account).await.map_err(store)?;
            let character = self
                .db
                .characters()
                .by_account(s.account)
                .await
                .map_err(store)?
                .map(|c| c.character.firstname);
            sanctions.push(json!({
                "account": s.account.0,
                "username": account.map(|a| a.account.username),
                "character": character,
                "kind": s.kind.as_str(),
                "until": rfc(s.until),
                "reason": s.reason,
                "by": s.by_whom,
                "at": s.created.to_string(),
            }));
        }
        let ip_bans: Vec<Value> = m
            .ip_bans()
            .await
            .map_err(store)?
            .into_iter()
            .filter(|b| b.active_at(now))
            .map(|b| {
                json!({
                    "id": b.id,
                    "network": b.network,
                    "until": rfc(b.until),
                    "reason": b.reason,
                    "by": b.by_whom,
                    "at": b.created.to_string(),
                })
            })
            .collect();
        Ok(json!({ "realm": realm, "sanctions": sanctions, "ip_bans": ip_bans }))
    }

    /// Does `action` for `by`.
    pub async fn run(&self, action: Action, by: String) -> Result<Value, ModError> {
        match action {
            Action::Kick { player, reason } => self.kick(&player, reason).await,
            Action::Tell { player, text } => self.tell(&player, text).await,
            Action::Put {
                player,
                kind,
                for_secs,
                reason,
            } => self.put(&player, kind, for_secs, reason, by).await,
            Action::Lift { player, kind } => self.lift(&player, kind).await,
            Action::BanIp {
                network,
                for_secs,
                reason,
            } => self.ban_ip(&network, for_secs, reason, by).await,
            Action::UnbanIp { network } => self.unban_ip(&network).await,
            Action::CloseRealm { reason } => self.close_realm(reason, by).await,
            Action::OpenRealm => self.open_realm().await,
            Action::Input { player, line } => self.input(&player, &line).await,
            Action::SysopLine { line } => self.sysop_line(&line, &by).await,
        }
    }
}

impl silvermere_api::Moderator for Operator {
    fn act(&self, action: Action, by: String) -> BoxFuture<'_, Result<Value, ModError>> {
        Box::pin(self.run(action, by))
    }

    fn status(&self) -> BoxFuture<'_, Result<Value, ModError>> {
        Box::pin(self.snapshot())
    }

    fn watch(&self, player: String) -> BoxFuture<'_, Result<silvermere_api::Watch, ModError>> {
        Box::pin(async move { Operator::watch(self, &player).await })
    }
}
