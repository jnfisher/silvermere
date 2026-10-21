//! `silvermere` with no subcommand (T-109): an empty data directory builds the stock world,
//! asks for a sysop on stdin and serves; a populated one serves at once; a fatal error with
//! a stdin that is not a console exits without waiting for Enter.

use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// A fresh directory whose data directory (`./data`) already caches the stock distribution,
/// as after a first download (T-120), so the first start imports without the network.
fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    let cache = d.join("data/cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::copy(
        silvermere_testkit::stock::zip(),
        cache.join(silvermere_testkit::stock::FILE_NAME),
    )
    .unwrap();
    d
}

/// The binary in `dir` with a clean `SILVERMERE_*` environment plus `env`.
fn silvermere(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.envs(env.iter().copied());
    c
}

const EPHEMERAL: &[(&str, &str)] = &[
    ("SILVERMERE_SERVER__TELNET_BIND", "127.0.0.1:0"),
    ("SILVERMERE_SERVER__ADMIN_BIND", "127.0.0.1:0"),
];

/// A first start running in `dir`, its stdout and stderr lines arriving on `lines`.
struct Started {
    child: Child,
    lines: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl Started {
    fn spawn(dir: &Path, stdin: &str) -> Self {
        let mut child = silvermere(dir, EPHEMERAL, &["--data-dir", "data"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        input.write_all(stdin.as_bytes()).unwrap();
        drop(input);
        let (tx, lines) = mpsc::channel();
        for r in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(r).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });
        }
        Self {
            child,
            lines,
            seen: Vec::new(),
        }
    }

    /// Waits for the "Silvermere is running" line; returns every line so far.
    fn until_running(&mut self) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(600);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.push(line);
                    // stdout and stderr arrive on two threads: wait for both lines
                    let has = |p: &str| self.seen.iter().any(|l| l.starts_with(p));
                    if has("Silvermere is running.") && has("silvermere: telnet listening on ") {
                        return self.seen.join("\n");
                    }
                }
                Err(_) => {
                    let _ = self.child.kill();
                    panic!("never running:\n{}", self.seen.join("\n"));
                }
            }
        }
    }

    /// The bound telnet port, from `serve`'s own line.
    fn port(&self) -> u16 {
        self.seen
            .iter()
            .find_map(|l| l.strip_prefix("silvermere: telnet listening on "))
            .and_then(|a| a.rsplit(':').next())
            .and_then(|p| p.parse().ok())
            .expect("a telnet listening line")
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn first_start_builds_asks_and_serves_then_serves_at_once() {
    let d = tmp("first-start");

    // An empty data directory: the stock world, then the sysop prompt, which the end of
    // stdin skips, then serving.
    let mut first = Started::spawn(&d, "");
    let out = first.until_running();
    assert!(out.contains("First start: building the world in"), "{out}");
    assert!(
        out.contains("importing the stock MajorMUD 1.11p distribution"),
        "{out}"
    );
    assert!(out.contains("There is no sysop account yet."), "{out}");
    assert!(
        out.contains(
            "No sysop account made. Make one later with: silvermere account create NAME --sysop"
        ),
        "{out}"
    );
    let port = first.port();
    assert!(
        out.contains(&format!(
            "Connect MegaMud or a telnet client to localhost port {port}. Close this window or \
             press Ctrl-C to stop."
        )),
        "{out}"
    );
    std::net::TcpStream::connect(("127.0.0.1", port)).expect("serving telnet");
    drop(first);
    // the import wrote the game's text (T-138): nothing to copy at start-up
    assert!(!out.contains("the game's text was not in"), "{out}");
    let assets = d.join("data/assets");
    for name in silvermere_import::assets::TEXT_ASSETS {
        assert!(assets.join(name).is_file(), "{name}");
    }

    // The world is there now: no import, the prompt again (a mismatch first), serving.
    let mut second = Started::spawn(&d, "Sysop\nsecret-1\nsecret-2\nSysop\nsecret-1\nsecret-1\n");
    let out = second.until_running();
    assert!(!out.contains("First start"), "{out}");
    assert!(
        out.contains("The two passwords differ; try again."),
        "{out}"
    );
    assert!(out.contains("Created the sysop account Sysop."), "{out}");
    drop(second);
    let list = silvermere(&d, &[], &["--data-dir", "data", "account", "list"])
        .output()
        .unwrap();
    let accounts = String::from_utf8_lossy(&list.stdout);
    assert!(
        accounts
            .lines()
            .any(|l| l.contains("Sysop") && l.contains("sysop")),
        "{accounts}"
    );

    // Populated, with a sysop: no import, no prompt, straight to serving.
    let mut third = Started::spawn(&d, "");
    let out = third.until_running();
    assert!(!out.contains("First start"), "{out}");
    assert!(!out.contains("importing the stock"), "{out}");
    assert!(!out.contains("sysop account"), "{out}");
    drop(third);

    // T-138: a data directory from 0.1.3 or earlier has no text files: the start copies them
    // from the cached package, says so in one line, and the text loads.
    for name in silvermere_import::assets::TEXT_ASSETS {
        std::fs::remove_file(assets.join(name)).unwrap();
    }
    let mut upgraded = Started::spawn(&d, "");
    let out = upgraded.until_running();
    let copied: Vec<&str> = out
        .lines()
        .filter(|l| l.contains("the game's text was not in"))
        .collect();
    assert_eq!(copied.len(), 1, "{out}");
    assert!(copied[0].contains("copied WCCMMUD.MSG, WCCMMHLP.MSG, WCCTEXT.MSG, DLLTEXT.TXT"));
    drop(upgraded);
    silvermere_text::package::PackageText::load(&assets).expect("the copied text loads");

    // An import cut off after the rooms: `serve` refuses and says how to finish it; the
    // first start asks, refuses on N and builds the world again on Enter.
    interrupt_import(&d);
    let o = silvermere(&d, EPHEMERAL, &["--data-dir", "data", "serve"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(3), "{err}");
    assert!(err.contains("The last import did not finish"), "{err}");
    let o = silvermere(&d, EPHEMERAL, &["--data-dir", "data"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            c.stdin.take().unwrap().write_all(b"n\n")?;
            c.wait_with_output()
        })
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(o.status.code(), Some(1), "{all}");
    assert!(
        all.contains("Press Enter to build the stock world again"),
        "{all}"
    );
    assert!(
        all.contains("error: The last import did not finish"),
        "{all}"
    );
    let mut fourth = Started::spawn(&d, "\n");
    let out = fourth.until_running();
    assert!(out.contains("importing the stock"), "{out}");
    assert!(out.contains("The world is built."), "{out}");
    drop(fourth);
    let (items, finished) = world(&d);
    assert!(items > 0 && finished, "{items} items, finished {finished}");
}

/// What an import cut off after its first table leaves: rooms, no items, and an unfinished
/// `world_import` row.
fn interrupt_import(d: &Path) {
    db(d, async |db| {
        let pool = db.pool();
        sqlx::query("DELETE FROM items")
            .execute(&pool)
            .await
            .unwrap();
        silvermere_db::world_import::begin(db).await.unwrap();
    });
}

/// The item count and whether the last import finished.
fn world(d: &Path) -> (i64, bool) {
    db(d, async |db| {
        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&db.pool())
            .await
            .unwrap();
        let state = silvermere_db::world_import::state(db).await.unwrap();
        (items, state == silvermere_db::WorldImport::Finished)
    })
}

fn db<T>(d: &Path, f: impl AsyncFnOnce(&silvermere_db::Db) -> T) -> T {
    let url = format!("sqlite://{}", d.join("data/silvermere.db").display());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let db = silvermere_db::Db::connect(&url).await.unwrap();
            let r = f(&db).await;
            db.close().await;
            r
        })
}

/// A fatal error with a stdin that is not a console exits at once, with no Enter prompt.
#[test]
fn a_fatal_error_does_not_wait_without_a_console() {
    let d = tmp("first-start-fatal");
    let mut child = silvermere(
        &d,
        &[("SILVERMERE_GAME__INACTIVE_SECS", "not a number")],
        &["--data-dir", "data"],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    // stdin stays open: a wait for Enter would hang until the deadline
    let _stdin = child.stdin.take();
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("still waiting after a fatal error");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let o = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(err.contains("error:"), "{err}");
    assert!(!err.contains("Press Enter"), "{err}");
}
