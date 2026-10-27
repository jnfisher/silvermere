//! The stock world on a throwaway SQLite, a running in-process server, and the scripted
//! creation of Aldric Stonebrook (Human Warrior) that the T-005 captures start from.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{SeededRng, SystemClock};
use silvermere_import::{ImportOptions, Importer};
use silvermere_testkit::TelnetClient;
use silvermere_testkit::shared_template::shared_source_file;

pub const WAIT: Duration = Duration::from_secs(30);

static TEMPLATE: OnceLock<PathBuf> = OnceLock::new();
static ALDRIC: OnceLock<PathBuf> = OnceLock::new();

fn stock_dir() -> PathBuf {
    silvermere_testkit::stock::vir_dir()
}

/// What the shared database templates are made from: the stock `.VIR` files, and the sources of
/// the code that imports them and creates characters in them (the workspace's crates, this
/// binary's code, these helpers, the migrations and the lock file). One template serves every
/// test binary of a build.
pub fn template_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut v = vec![stock_dir()];
    for p in [
        "crates",
        "bin/silvermere/src",
        "bin/silvermere/Cargo.toml",
        "bin/silvermere/tests/common",
        "migrations",
        "patches",
        "Cargo.lock",
        "Cargo.toml",
    ] {
        v.push(root.join(p));
    }
    v
}

/// `file` under `dir_prefix<fingerprint of template_sources>/`, built by `build` once for every
/// test process of every test binary (nextest runs each test in its own process).
pub fn shared_template(dir_prefix: &str, file: &str, build: impl FnOnce(&Path)) -> PathBuf {
    // under target/ so `cargo clean` takes it
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let sources = template_sources();
    let inputs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    shared_source_file(base, dir_prefix, file, &inputs, build)
}

/// The imported stock world as a SQLite file, built once per build and shared by every test
/// process.
fn template() -> &'static Path {
    TEMPLATE.get_or_init(|| {
        let from = stock_dir();
        shared_template("silvermere-m1-", "stock.db", |path| {
            let url = format!("sqlite://{}", path.display());
            let from = from.clone();
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let db = Db::connect_and_migrate(&url).await.unwrap();
                        Importer::new(ImportOptions::new(&from), Some(&db))
                            .run()
                            .await
                            .expect("stock import");
                        db.close().await;
                    });
            })
            .join()
            .unwrap();
        })
    })
}

/// The stock world with Aldric created by [`create_aldric`], built once per build like the
/// stock world itself.
fn aldric_template() -> &'static Path {
    ALDRIC.get_or_init(|| {
        shared_template("silvermere-aldric-", "aldric.db", |path| {
            let (stock_dir, stock_url) = fresh_db();
            std::fs::copy(stock_url.trim_start_matches("sqlite://"), path).unwrap();
            drop(stock_dir);
            let url = format!("sqlite://{}", path.display());
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(create_aldric(&url));
            })
            .join()
            .unwrap();
        })
    })
}

/// A throwaway directory, removed when dropped.
pub struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A copy of the stock world: `(its directory, database url)`.
pub fn fresh_db() -> (TempDir, String) {
    copy_of(template())
}

/// A copy of the stock world in which [`create_aldric`] has run: `(its directory, url)`.
pub fn aldric_db() -> (TempDir, String) {
    copy_of(aldric_template())
}

/// A copy of the template `t` in a directory of its own: `(the directory, database url)`.
fn copy_of(t: &Path) -> (TempDir, String) {
    let t = t.to_owned();
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = t
        .parent()
        .unwrap()
        .join(format!("t{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");
    std::fs::copy(&t, &path).unwrap();
    silvermere_testkit::world_cache::remember_template(&t);
    let url = format!("sqlite://{}", path.display());
    (TempDir(dir), url)
}

/// The world in the database at `url`, read once for every copy of a template
/// ([`silvermere_testkit::world_cache`]).
pub async fn world_of(url: &str) -> silvermere_engine::WorldSnapshot {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let sources = template_sources();
        let inputs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
        silvermere_testkit::world_cache::snapshot(&url, base, &inputs, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let db = Db::connect(&url).await.unwrap();
                    let s = silvermere::bridge::load_snapshot(&db).await.unwrap();
                    db.close().await;
                    s
                })
        })
    })
    .await
    .unwrap()
}

pub async fn start(url: &str) -> silvermere::Running {
    start_with(url, |_| {}).await
}

/// [`start`] with the configuration adjusted first.
pub async fn start_with(url: &str, edit: impl FnOnce(&mut Config)) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.to_owned();
    cfg.data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    edit(&mut cfg);
    // the game's long text, as a server with imported assets installs it (T-138)
    silvermere_text::package::load_and_install(&silvermere_testkit::stock::assets_dir())
        .expect("the stock package's text");
    let world = world_of(url).await;
    let running =
        silvermere::serve_snapshot_in_process(cfg, Some(world), SystemClock, SeededRng::new(7))
            .await
            .expect("starts");
    // The M1 walks are about rooms, exits and screens, not about monsters (T-035 spawns them,
    // T-040 compares their rate): an empty quick reference means nothing is generated, so no
    // wanderer attacks the walker or adds "You hear movement" lines the captures do not have.
    // The monsters the start-up pass made (lairs, shops, specific monsters; T-076)
    // are taken out for the same reason.
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

/// Runs the engine's fast ticks from the test, every 5 ms, instead of waiting for the move
/// delay, and tops the walkers up to full health after each: the walks are about rooms, exits
/// and screens, and at this speed a river room (or any other room effect) would kill the
/// walker within a few steps.
///
/// Each tick, the heal and a reset of the dice happen in one engine turn, so no command can run
/// between the tick and the reset: whatever a command draws (a room script's `random` clause
/// such as the arena's "A cheer of many voices ...", a wanderer's noise) it draws from the same
/// state at every step and on every run. The server must have no tick of its own
/// ([`start_ticked_by_test`]); with the wall clock supplying some of them, the draws a command
/// saw depended on how many had run, and a line the captures do not have landed in a room
/// screen now and then.
pub fn walker_ticker(r: &silvermere::Running) -> tokio::task::JoinHandle<()> {
    use silvermere_engine::EngineInput;
    let tx = r.to_engine.clone();
    tokio::spawn(async move {
        loop {
            let tick = EngineInput::Custom {
                session: None,
                run: Box::new(|w| {
                    silvermere_engine::scheduler::on_fast_tick(w);
                    for id in w.session_ids() {
                        if let Some(p) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
                            p.hp = p.maxhp;
                        }
                    }
                    w.rng = silvermere_engine::SeededRng::new(7);
                }),
            };
            if tx.send(tick).is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
}

/// [`start`] with the server's own fast tick pushed out of reach (a day), for the tests whose
/// ticks come from [`walker_ticker`] alone.
pub async fn start_ticked_by_test(url: &str) -> silvermere::Running {
    start_with(url, |cfg| cfg.game.sec_fast = 86_400).await
}

/// Creates the account `user`/`password` in the database.
pub async fn add_account(url: &str, user: &str, password: &str) {
    let db = Db::connect_and_migrate(url).await.unwrap();
    create_with_cost(&db, user, password, &[Role::Player], Cost::Low)
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

/// The line-mode creation of Aldric Stonebrook (the stats of `creation-fsd`: Strength and
/// Health raised to 50), ending at the first room's prompt and then out through `x`, so the
/// character is saved with the clean logoff.
pub async fn create_aldric(url: &str) {
    add_account(url, "Aldric", "aldric1").await;
    let r = start(url).await;
    let mut c = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    for (line, next) in [
        ("E", "choose your race"),
        ("1", "choose your class"),
        ("1", r"Lawful\?"),
        ("N", "Given name"),
        ("", "Family name"),
        ("Stonebrook", "Strength"),
        ("50", "Intellect"),
        ("", "Willpower"),
        ("", "Agility"),
        ("", "Health"),
        ("50", "Charm"),
        ("", "Hair length"),
        ("", "Hair colour"),
        ("", "Eye colour"),
        ("", r"SAVE or QUIT"),
        ("", r"\[HP=-?\d+[^\]\n]*\]:"),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    quit_to_menu(&mut c, &r.to_engine, "Aldric").await;
    c.disconnect().await;
    r.stop().await.unwrap();
}

/// Changes Aldric's stored record (while no server is running on the database).
pub async fn edit_aldric(url: &str, edit: impl FnOnce(&mut silvermere_types::records::Character)) {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let mut row = db
        .characters()
        .by_first_name("Aldric")
        .await
        .unwrap()
        .expect("Aldric exists");
    edit(&mut row.character);
    assert!(
        db.characters()
            .update(row.id, &row.character)
            .await
            .unwrap()
    );
    db.close().await;
}

/// Runs `f` on the world thread and returns what it returns.
pub async fn on_world<T: Send + 'static>(
    r: &silvermere::Running,
    f: impl FnOnce(&mut silvermere_engine::World) -> T + Send + 'static,
) -> T {
    on_engine(&r.to_engine, f).await
}

/// [`on_world`] given only the server's engine input.
pub async fn on_engine<T: Send + 'static>(
    to_engine: &std::sync::mpsc::Sender<silvermere_engine::EngineInput>,
    f: impl FnOnce(&mut silvermere_engine::World) -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    to_engine
        .send(silvermere_engine::EngineInput::Custom {
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

/// Logs `c` (in the realm as `name`) off with `x` and `X`: the clean logoff, which saves the
/// character. The silent meditation before the exit counts down one step per fast cycle; the
/// test steps `name`'s countdown to its end instead of waiting for the real-time cycles, so the
/// same dots and the same exit arrive without the wait.
pub async fn quit_to_menu(
    c: &mut TelnetClient,
    to_engine: &std::sync::mpsc::Sender<silvermere_engine::EngineInput>,
    name: &str,
) {
    c.send_line("x").await.unwrap();
    c.until("silent meditation", WAIT).await.unwrap();
    let name = name.to_owned();
    on_engine(to_engine, move |w| {
        use silvermere_engine::state::login::leave::{exit_countdown, is_exiting};
        let id = w
            .session_ids()
            .into_iter()
            .find(|&id| {
                w.session(id)
                    .and_then(|s| s.character.as_ref())
                    .is_some_and(|p| p.firstname == name)
            })
            .unwrap_or_else(|| panic!("{name} is not in the game"));
        // the real cycles may finish it first; the delay is a handful of cycles
        for _ in 0..1000 {
            if !is_exiting(w, id) {
                break;
            }
            exit_countdown(w, id);
        }
    })
    .await;
    c.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    c.send_line("X").await.unwrap();
    c.until(r"Make your selection", WAIT).await.unwrap();
}
