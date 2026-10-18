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
use silvermere_testkit::shared_template::shared_file;

pub const WAIT: Duration = Duration::from_secs(30);

static TEMPLATE: OnceLock<PathBuf> = OnceLock::new();

fn stock_dir() -> PathBuf {
    silvermere_testkit::stock::vir_dir()
}

/// The imported stock world as a SQLite file, built once per build of the test binary and shared
/// by every test process (nextest runs each test in its own).
fn template() -> &'static Path {
    TEMPLATE.get_or_init(|| {
        // under target/ so `cargo clean` takes it
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let from = stock_dir();
        shared_file(base, "silvermere-m1-", "stock.db", &[&from], |path| {
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

/// A throwaway directory, removed when dropped.
pub struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A copy of the stock world: `(its directory, database url)`.
pub fn fresh_db() -> (TempDir, String) {
    let t = template().to_owned();
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = t
        .parent()
        .unwrap()
        .join(format!("t{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");
    std::fs::copy(&t, &path).unwrap();
    let url = format!("sqlite://{}", path.display());
    (TempDir(dir), url)
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
    let running = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
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
    c.send_line("x").await.unwrap();
    c.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    c.send_line("X").await.unwrap();
    c.until(r"Make your selection", WAIT).await.unwrap();
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
    let (tx, rx) = std::sync::mpsc::channel();
    r.to_engine
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
