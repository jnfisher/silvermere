//! Shared by the M4 tests (T-057): the stock world imported once per test binary, a copy of
//! it per test, an in-process server on that copy, accounts with roles, the line-mode
//! creation of a character, and a closure run on the world thread.
//!
//! The backend follows `SILVERMERE_DATABASE_URL`: a `postgres:` URL gets a template database
//! (`CREATE DATABASE ... TEMPLATE`) copied per test; anything else (unset, `sqlite://`) a
//! template SQLite file copied per test. Same shape as T-029's `bin/silvermere/tests/common/
//! stock.rs`, which a follow-up can merge with this one.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{EngineInput, SeededRng, SessionId, SystemClock, World};
use silvermere_import::{ImportOptions, Importer};
use silvermere_testkit::TelnetClient;
use silvermere_testkit::shared_template::shared_file;

pub const WAIT: Duration = Duration::from_secs(30);
/// The prompt, `[HP=..]:` (with or without `/MA=`, `(Resting)` and the like).
pub const PROMPT: &str = r"\[HP=-?\d+[^\]\n]*\]:";

fn stock_dir() -> PathBuf {
    silvermere_testkit::stock::vir_dir()
}

fn pg_base() -> Option<String> {
    match std::env::var("SILVERMERE_DATABASE_URL") {
        Ok(u) if u.starts_with("postgres") => Some(u),
        _ => None,
    }
}

/// `postgres://u@h:p/db?x` with the database name replaced.
fn pg_with_db(base: &str, db: &str) -> String {
    let (head, query) = match base.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (base, None),
    };
    let after_scheme = head.find("://").map_or(0, |i| i + 3);
    let root = match head[after_scheme..].find('/') {
        Some(i) => &head[..after_scheme + i],
        None => head,
    };
    match query {
        Some(q) => format!("{root}/{db}?{q}"),
        None => format!("{root}/{db}"),
    }
}

fn block_on<T: Send + 'static>(f: impl std::future::Future<Output = T> + Send + 'static) -> T {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    })
    .join()
    .unwrap()
}

async fn pg_exec(base: &str, sql: String) {
    use sqlx::Connection;
    sqlx::any::install_default_drivers();
    let mut c = sqlx::AnyConnection::connect(base).await.unwrap();
    sqlx::Executor::execute(&mut c, sqlx::AssertSqlSafe(sql))
        .await
        .unwrap();
    c.close().await.unwrap();
}

/// Drops the `sm_m4_*` databases an earlier test process left (its template, and copies
/// a panic kept): a test binary has no exit hook to drop its own. Those of a process that
/// still runs stay.
async fn drop_stale(base: &str) {
    use sqlx::{Connection, Row};
    sqlx::any::install_default_drivers();
    let mut c = sqlx::AnyConnection::connect(base).await.unwrap();
    let rows = sqlx::query("SELECT datname::text FROM pg_database WHERE datname LIKE 'sm_m4_%'")
        .fetch_all(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    for r in rows {
        let name: String = r.get(0);
        // sm_m4_tpl_<pid> and sm_m4_<pid>_<n>
        let pid = name
            .trim_start_matches("sm_m4_")
            .trim_start_matches("tpl_")
            .split('_')
            .next()
            .and_then(|p| p.parse().ok());
        if pid.is_some_and(|p| !process_alive(p)) {
            pg_exec(base, format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).await;
        }
    }
}

/// Whether test process `pid` still runs, so its template and copies are left alone
/// (another test binary may run at the same time). Without `/proc` nothing counts as gone.
fn process_alive(pid: u32) -> bool {
    pid == std::process::id()
        || !Path::new("/proc").is_dir()
        || Path::new("/proc").join(pid.to_string()).exists()
}

async fn import_into(url: String) {
    let db = Db::connect_and_migrate(&url).await.unwrap();
    Importer::new(ImportOptions::new(stock_dir()), Some(&db))
        .run()
        .await
        .expect("stock import");
    db.close().await;
}

enum Template {
    Sqlite(PathBuf),
    Pg { base: String, name: String },
}

static TEMPLATE: OnceLock<Template> = OnceLock::new();

/// The imported stock world, built once per test binary.
fn template() -> &'static Template {
    TEMPLATE.get_or_init(|| {
        if let Some(base) = pg_base() {
            let name = format!("sm_m4_tpl_{}", std::process::id());
            let b = base.clone();
            let n = name.clone();
            block_on(async move {
                drop_stale(&b).await;
                pg_exec(&b, format!("DROP DATABASE IF EXISTS {n}")).await;
                pg_exec(&b, format!("CREATE DATABASE {n}")).await;
                import_into(pg_with_db(&b, &n)).await;
            });
            return Template::Pg { base, name };
        }
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let from = stock_dir();
        let path = shared_file(base, "silvermere-m4-", "stock.db", &[&from], |path| {
            block_on(import_into(format!("sqlite://{}", path.display())));
        });
        Template::Sqlite(path)
    })
}

/// A copy of the stock world, removed when dropped.
pub struct StockDb {
    pub url: String,
    dir: Option<PathBuf>,
    pg: Option<(String, String)>,
}

impl Drop for StockDb {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        if let Some((base, name)) = self.pg.take() {
            block_on(async move {
                pg_exec(
                    &base,
                    format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
                )
                .await;
            });
        }
    }
}

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A fresh copy of the stock world.
pub fn stock_db() -> StockDb {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    match template() {
        Template::Sqlite(t) => {
            let dir = t
                .parent()
                .unwrap()
                .join(format!("t{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("test.db");
            std::fs::copy(t, &path).unwrap();
            StockDb {
                url: format!("sqlite://{}", path.display()),
                dir: Some(dir),
                pg: None,
            }
        }
        Template::Pg { base, name } => {
            let copy = format!("sm_m4_{}_{n}", std::process::id());
            let (b, tpl, c) = (base.clone(), name.clone(), copy.clone());
            // a template may be copied by one session at a time
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            block_on(async move {
                pg_exec(&b, format!("CREATE DATABASE {c} TEMPLATE {tpl}")).await;
            });
            StockDb {
                url: pg_with_db(base, &copy),
                dir: None,
                pg: Some((base.clone(), copy)),
            }
        }
    }
}

/// The server on `url` with seed `seed`; nothing is generated (no wanderers) and the
/// monsters of the start-up pass (T-076) are taken out, so a scenario places the monsters
/// it needs. The fast tick is left to the test: [`ticker`] or
/// [`tick`].
pub async fn start(url: &str, seed: u64) -> silvermere::Running {
    start_with(url, seed, |_| {}).await
}

/// [`start`] with the configuration adjusted first.
pub async fn start_with(
    url: &str,
    seed: u64,
    edit: impl FnOnce(&mut Config),
) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.to_owned();
    cfg.data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    edit(&mut cfg);
    let running = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(seed))
        .await
        .expect("starts");
    on_world(&running, |w| {
        w.monqref.clear();
        for n in w.active_monsters.numbers_in_use() {
            silvermere_engine::monsters::active::remove_active_monster(w, n, false);
        }
        for st in w.room_state.values_mut() {
            st.specalive = false;
        }
    })
    .await;
    // those writes are committed before a test logs in (a name check waits for them)
    assert!(
        running.settle(Duration::from_secs(600)).await,
        "start-up writes committed"
    );
    running
}

/// The server on `url` as `silvermere serve` starts it: the start-up pass's monsters left in
/// place, the quick reference kept.
pub async fn start_as_served(url: &str, seed: u64) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.to_owned();
    cfg.data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(seed))
        .await
        .expect("starts")
}

/// Creates the account `user`/`password` with `roles` (Player is always added).
pub async fn add_account(url: &str, user: &str, password: &str, roles: &[Role]) {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let mut all = vec![Role::Player];
    all.extend_from_slice(roles);
    create_with_cost(&db, user, password, &all, Cost::Low)
        .await
        .unwrap();
    db.close().await;
}

/// Logs in and answers the menus up to the `[MAJORMUD]:` prompt.
pub async fn login_to_menu(c: &mut TelnetClient, user: &str, password: &str) {
    c.until("Username:", WAIT).await.unwrap();
    c.send_line(user).await.unwrap();
    c.until("Password:", WAIT).await.unwrap();
    c.send_line(password).await.unwrap();
    c.until(r"Make your selection \(X to exit\):", WAIT)
        .await
        .unwrap();
    c.send_line("M").await.unwrap();
    c.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
}

/// A new account `user` whose character `user Surname` (race 1 Human, `class`) is created
/// through the line-mode dialogue and stands in the realm; returns the client.
pub async fn new_character(
    r: &silvermere::Running,
    url: &str,
    user: &str,
    class: u16,
    roles: &[Role],
) -> TelnetClient {
    add_account(url, user, &format!("{}1", user.to_lowercase()), roles).await;
    let mut c = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut c, user, &format!("{}1", user.to_lowercase())).await;
    let class = class.to_string();
    for (line, next) in [
        ("E", "choose your race"),
        ("1", "choose your class"),
        (class.as_str(), r"Lawful\?|Given name"),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    if c.plain().ends_with("? ") || c.plain().contains("Lawful?") {
        c.send_line("N").await.unwrap();
        c.until("Given name", WAIT).await.unwrap();
    }
    for (line, next) in [
        ("", "Family name"),
        ("Testcase", "Strength"),
        ("", "Intellect"),
        ("", "Willpower"),
        ("", "Agility"),
        ("", "Health"),
        ("", "Charm"),
        ("", "Hair length"),
        ("", "Hair colour"),
        ("", "Eye colour"),
        ("", r"SAVE or QUIT"),
        ("", PROMPT),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    c
}

/// Runs `f` on the world thread and returns what it returns.
pub async fn on_world<T: Send + 'static>(
    r: &silvermere::Running,
    f: impl FnOnce(&mut World) -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    r.to_engine
        .send(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                let _ = tx.send(f(w));
            }),
        })
        .unwrap();
    tokio::task::spawn_blocking(move || rx.recv().unwrap())
        .await
        .unwrap()
}

/// The session whose character's first name is `name`.
pub fn session_of(w: &World, name: &str) -> SessionId {
    w.session_ids()
        .into_iter()
        .find(|&id| {
            w.session(id)
                .and_then(|s| s.character.as_ref())
                .is_some_and(|p| p.firstname.eq_ignore_ascii_case(name))
        })
        .unwrap_or_else(|| panic!("no session for {name}"))
}

/// Runs `f` on `name`'s character.
pub async fn edit_player(
    r: &silvermere::Running,
    name: &str,
    f: impl FnOnce(&mut silvermere_types::records::Character) + Send + 'static,
) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        f(w.session_mut(id).unwrap().character.as_mut().unwrap());
    })
    .await;
}

/// `n` fast ticks, now.
pub async fn tick(r: &silvermere::Running, n: u32) {
    for _ in 0..n {
        r.to_engine.send(EngineInput::Tick).unwrap();
    }
    on_world(r, |_| ()).await;
}

/// Feeds the engine a fast tick every `every` until the handle is aborted: rounds and
/// cycles at test speed.
pub fn ticker(r: &silvermere::Running, every: Duration) -> tokio::task::JoinHandle<()> {
    let tx = r.to_engine.clone();
    tokio::spawn(async move {
        while tx.send(EngineInput::Tick).is_ok() {
            tokio::time::sleep(every).await;
        }
    })
}

/// How long the text, prompts apart, must sit still before a command with no reply is taken
/// as done (`SILVERMERE_TEST_SILENT_MS` overrides it on slow runners).
fn silent() -> Duration {
    static MS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
        std::env::var("SILVERMERE_TEST_SILENT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400)
    });
    Duration::from_millis(*MS)
}

/// A prompt as drawn on the wire: `[HP=..]:`, then a state tag such as ` (Resting)`.
fn prompt_rx() -> &'static regex::Regex {
    static RX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(&format!(r"{PROMPT}(?:\s*\([A-Za-z ]*\))?")).unwrap()
    });
    &RX
}

/// The regex finding the echo of `line`: a whole line, at the start of a line or straight
/// after a prompt, and followed by a line end, the end of the text or a redrawn prompt (so
/// `w` is not found inside "arrives from the west."). An empty `line` (Enter alone) echoes
/// as nothing: the first prompt that is followed by a line end or another prompt.
fn echo_rx(line: &str) -> regex::Regex {
    regex::Regex::new(&format!(
        r"(?m)(?:^|{PROMPT}(?:\s*\([A-Za-z ]*\))?[ ]?){}[ \t]*(?:\r?$|{PROMPT})",
        regex::escape(line)
    ))
    .unwrap()
}

/// The text after the echo found by `echo`, if it is there yet.
fn after_echo<'a>(text: &'a str, echo: &regex::Regex, line: &str) -> Option<&'a str> {
    // the match may swallow the next prompt; the echo ends where `line` does
    let m = echo.find(text)?;
    let at = m.start() + text[m.start()..m.end()].find(line)?;
    Some(&text[at + line.len()..])
}

fn reply_after(rest: &str) -> bool {
    let prompt = prompt_rx();
    let tail = rest.trim_end();
    let ends_in_prompt = prompt
        .find_iter(tail)
        .last()
        .is_some_and(|m| m.end() == tail.len());
    ends_in_prompt && !prompt.replace_all(rest, "").trim().is_empty()
}

/// True once `text` (what arrived since `line` was sent) holds the reply to `line`: after
/// the echo of the line, some output that is not a prompt, then a prompt. Prompts redrawn
/// by the test ticker before the reply are skipped.
pub fn has_reply(text: &str, line: &str) -> bool {
    after_echo(text, &echo_rx(line), line).is_some_and(reply_after)
}

/// True when the text after the echo is nothing but prompts (a command with no reply, or
/// one whose reply has not come).
fn only_prompts(rest: &str) -> bool {
    let prompt = prompt_rx();
    prompt.is_match(rest) && prompt.replace_all(rest, "").trim().is_empty()
}

/// Pumps the stream for about `ms` (`read` waits for quiet, which redrawn prompts never allow).
pub async fn pump(c: &mut TelnetClient, ms: u64) {
    let _ = c
        .wait_from(
            c.position(),
            &silvermere_testkit::client::Wait {
                until: Some("\\A\\z.".into()),
                timeout: Duration::from_millis(ms),
                ..Default::default()
            },
        )
        .await;
}

/// Sends `line` (empty is Enter alone) and waits for its reply (see [`has_reply`]); returns
/// the plain text in between. A command that prints nothing is done once only prompts have arrived and the
/// text, prompts apart, has been still for [`silent`]: prompts the engine redraws on its
/// own (resting, a room spell, the test ticker) do not count as activity.
pub async fn cmd(c: &mut TelnetClient, line: &str) -> String {
    let at = c.position();
    c.send_line(line).await.unwrap();
    let echo = echo_rx(line);
    let t0 = tokio::time::Instant::now();
    let mut still = t0;
    let mut seen = String::new();
    loop {
        let text = c.plain_from(at);
        let rest = after_echo(&text, &echo, line);
        if rest.is_some_and(reply_after) {
            return text;
        }
        let bare: String = prompt_rx()
            .replace_all(&text, "")
            .split_whitespace()
            .collect();
        if bare != seen {
            seen = bare;
            still = tokio::time::Instant::now();
        } else if rest.is_some_and(only_prompts) && still.elapsed() >= silent() {
            return text;
        }
        if t0.elapsed() >= WAIT {
            let from = text.len().saturating_sub(500);
            let from = (from..text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(text.len());
            panic!(
                "no reply to `{line}` within {WAIT:?}; last 500 chars:\n{}",
                &text[from..]
            );
        }
        pump(c, 20).await;
    }
}

/// Sends `line` and waits for `regex`; returns the plain text in between.
pub async fn cmd_until(c: &mut TelnetClient, line: &str, regex: &str) -> String {
    let at = c.position();
    c.send_line(line).await.unwrap();
    c.until(regex, WAIT).await.unwrap();
    c.plain_from(at)
}

/// Puts `name` in `map:room` (what `sysop goto` does, `set_user_location`).
pub async fn place(r: &silvermere::Running, name: &str, map: u32, room: u32) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        silvermere_engine::movement::move_user::set_user_location(
            w,
            id,
            silvermere_types::ids::RoomKey::new(map, room),
            false,
        );
    })
    .await;
}

/// Generates known monster `monster` in `map:room` (`generate_monster`, forced); returns
/// its active number.
pub async fn spawn(
    r: &silvermere::Running,
    map: u32,
    room: u32,
    monster: u16,
) -> silvermere_types::ids::ActiveMonsterId {
    on_world(r, move |w| {
        use silvermere_engine::monsters::generate::{Request, generate_monster};
        generate_monster(
            w,
            silvermere_types::ids::RoomKey::new(map, room),
            Request {
                areatype: -1,
                monnum: silvermere_types::ids::MonsterNo(i32::from(monster)),
                minlevel: 0,
                maxlevel: 0,
                owner: None,
                ally: false,
                force: true,
            },
        )
        .expect("monster generated")
    })
    .await
}

/// [`spawn`] that may find the room full.
pub async fn try_spawn(
    r: &silvermere::Running,
    map: u32,
    room: u32,
    monster: u16,
) -> Option<silvermere_types::ids::ActiveMonsterId> {
    on_world(r, move |w| {
        use silvermere_engine::monsters::generate::{Request, generate_monster};
        generate_monster(
            w,
            silvermere_types::ids::RoomKey::new(map, room),
            Request {
                areatype: -1,
                monnum: silvermere_types::ids::MonsterNo(i32::from(monster)),
                minlevel: 0,
                maxlevel: 0,
                owner: None,
                ally: false,
                force: true,
            },
        )
    })
    .await
}

/// The hit points of active monster `m` (`None` once it is gone).
pub async fn monster_hp(
    r: &silvermere::Running,
    m: silvermere_types::ids::ActiveMonsterId,
) -> Option<i32> {
    on_world(r, move |w| {
        w.active_monsters
            .values()
            .find(|a| a.number == m)
            .map(|a| i32::from(a.hp))
    })
    .await
}

/// The lines of `text` that neither the static registry (`tests/fixtures/patterns.json`)
/// nor a WCCMSG text or room title of the stock world classifies: the Rust side of
/// `Registry.load_world` (the generated families such as `spell_cast_you`, 209 of them,
/// come from the same WCCMSG texts).
pub struct Classifier {
    set: silvermere_testkit::PatternSet,
    templates: Vec<regex::Regex>,
    /// The text blocks (talk, room command texts) with their whitespace collapsed: shown
    /// word-wrapped, so a line of one is a piece of it.
    blocks: Vec<String>,
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Classifier {
    /// The default patterns alone, with no world behind them.
    pub fn bare() -> Self {
        Self {
            set: silvermere_testkit::PatternSet::load_default().expect("patterns.json"),
            templates: Vec::new(),
            blocks: Vec::new(),
        }
    }

    pub async fn new(r: &silvermere::Running) -> Self {
        let (texts, names, blocks) = on_world(r, |w| {
            let mut t = Vec::new();
            for m in w.messages.values() {
                for s in &m.msgs {
                    t.push(
                        String::from_utf8_lossy(&silvermere_text::strip_ansi(s.as_bytes()))
                            .into_owned(),
                    );
                }
            }
            for m in w.monsters.values() {
                t.push(m.name.clone());
            }
            let names: Vec<String> = w.rooms.values().map(|r| r.name.clone()).collect();
            let blocks: Vec<String> = w
                .text_blocks
                .values()
                .map(|b| {
                    collapse(&String::from_utf8_lossy(&silvermere_text::strip_ansi(
                        b.text.as_bytes(),
                    )))
                })
                .collect();
            (t, names, blocks)
        })
        .await;
        let mut set = silvermere_testkit::PatternSet::load_default().expect("patterns.json");
        set.set_room_names(names);
        let mut templates = Vec::new();
        for t in texts {
            let t = t.trim();
            if t.len() < 4 {
                continue;
            }
            // `%s`, `%d`, `%c` and friends stand for anything
            let mut rx = String::from("^");
            let mut rest = t;
            while let Some(i) = rest.find('%') {
                rx += &regex::escape(&rest[..i]);
                let tail = &rest[i + 1..];
                let skip = tail
                    .find(|c: char| c.is_ascii_alphabetic())
                    .map_or(tail.len(), |k| k + 1);
                rx += ".*";
                rest = &tail[skip..];
            }
            rx += &regex::escape(rest);
            rx += "$";
            if let Ok(re) = regex::Regex::new(&rx) {
                templates.push(re);
            }
        }
        Self {
            set,
            templates,
            blocks,
        }
    }

    /// The unclassified lines of `text` (plain), the echoes of `sent` excepted.
    pub fn unknown(&self, text: &str, sent: &[&str]) -> Vec<String> {
        let sent: Vec<String> = sent.iter().map(|s| (*s).to_owned()).collect();
        self.set
            .classify_text(text, false, &sent)
            .into_iter()
            .filter(|e| e.name == "unknown")
            .map(|e| e.line)
            .filter(|l| {
                let l = l.trim();
                // the echo of a typed command
                !l.is_empty()
                    && !sent.iter().any(|c| c == l)
                    && !self.echo_then_known(l, &sent)
                    && !self.templates.iter().any(|re| re.is_match(l))
                    && !(l.len() >= 3 && {
                        let c = collapse(l);
                        self.blocks.iter().any(|b| b.contains(&c))
                    })
            })
            .collect()
    }

    /// `line` is the echo of a sent line followed by nothing but prompts: the prompt the
    /// room spell of a blinding room redraws on every other medium pass, which at test
    /// speed lands between a command and its echo's line end.
    fn echo_then_known(&self, line: &str, sent: &[String]) -> bool {
        sent.iter().filter(|c| !c.is_empty()).any(|c| {
            line.strip_prefix(c.as_str()).is_some_and(|rest| {
                let rest = rest.trim();
                // `classify_text` peels prompt after prompt; anything else is not a redraw
                let events = self.set.classify_text(rest, false, &[]);
                !rest.is_empty() && !events.is_empty() && events.iter().all(|e| e.name == "prompt")
            })
        })
    }

    /// The pattern names the lines of `text` classify as.
    pub fn names(&self, text: &str) -> Vec<String> {
        self.set
            .classify_text(text, false, &[])
            .into_iter()
            .map(|e| e.name)
            .collect()
    }
}

/// Puts `n` new copies of item `item` (the record's own charges: `-2`) in
/// `name`'s pack.
pub async fn give(r: &silvermere::Running, name: &str, item: i32, n: usize) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        for _ in 0..n {
            assert!(silvermere_engine::items::inventory::add_item_to_inventory(
                w,
                id,
                silvermere_types::ids::ItemNo(item),
                -2
            ));
        }
    })
    .await;
}

/// How many of item `item` `name` carries (pack and key ring).
pub async fn carried(r: &silvermere::Running, name: &str, item: i32) -> usize {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        let p = w.session(id).unwrap().character.as_ref().unwrap();
        p.items
            .iter()
            .chain(p.keys.iter())
            .filter(|i| i.0 == item)
            .count()
    })
    .await
}

/// `name`'s room number.
pub async fn room_of(r: &silvermere::Running, name: &str) -> u32 {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        w.session(id).unwrap().character.as_ref().unwrap().room.0
    })
    .await
}

/// Walks `dirs` (space-separated): each move is sent once the previous one has landed
/// (the room changed: a move waits out the movement delay on the fast tick, so its first
/// prompt can come before it lands); returns the text and the room reached.
pub async fn walk(
    r: &silvermere::Running,
    c: &mut TelnetClient,
    name: &str,
    dirs: &str,
) -> (String, u32) {
    let mut out = String::new();
    for d in dirs.split_whitespace() {
        let before = room_of(r, name).await;
        out += &cmd(c, d).await;
        let mut moved = false;
        for _ in 0..500 {
            if room_of(r, name).await != before {
                moved = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            moved,
            "`{d}` from {before} did not move (walking `{dirs}`):\n{out}"
        );
    }
    // the rest of the last room's screen
    out += &c
        .read(Duration::from_millis(100))
        .await
        .map(|g| g.plain)
        .unwrap_or_default();
    (out, room_of(r, name).await)
}
