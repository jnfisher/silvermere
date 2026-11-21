//! Graceful shutdown (T-059): the real binary on SIGTERM, and an in-process server whose
//! player is told before the socket closes and whose character is saved.

use crate::common;

#[cfg(unix)]
use std::io::{BufRead, Read, Write};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{SeededRng, SystemClock};
use silvermere_testkit::TelnetClient;
use silvermere_types::flags::PlayerStatus;

// The real binary on SIGTERM, and its helpers: Unix only.
#[cfg(unix)]
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sm-shutdown-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    // `serve` completes the game's text from the package in `data/cache/`, downloading it
    // when it is not there: the test's copy comes from the shared cache, so no start waits on
    // the network
    let cache = d.join("data/cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::copy(
        silvermere_testkit::stock::zip(),
        cache.join(silvermere_testkit::stock::FILE_NAME),
    )
    .unwrap();
    d
}

#[cfg(unix)]
fn healthz(addr: &str) -> bool {
    let Ok(mut s) = std::net::TcpStream::connect(addr) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    raw.starts_with("HTTP/1.1 200")
}

#[cfg(unix)]
struct Serve {
    child: std::process::Child,
    lines: std::thread::JoinHandle<Vec<String>>,
    db_path: PathBuf,
    dir: PathBuf,
}

/// The file log (`data/logs/silvermere.log.<date>`): what is on disk once the process is gone.
#[cfg(unix)]
fn file_log(dir: &Path) -> String {
    let logs = dir.join("data").join("logs");
    let mut text = String::new();
    for e in std::fs::read_dir(&logs).into_iter().flatten().flatten() {
        if e.file_name()
            .to_string_lossy()
            .starts_with("silvermere.log")
        {
            text.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
        }
    }
    text
}

#[cfg(unix)]
/// Starts `silvermere serve` on a seeded temp data dir and waits for `/healthz`.
fn start_serve(name: &str) -> Serve {
    start_serve_with(name, &[]).0
}

#[cfg(unix)]
/// [`start_serve`] with these environment variables too; also the admin address.
fn start_serve_with(name: &str, env: &[(&str, &str)]) -> (Serve, String) {
    let d = tmp(name);
    let db_path = d.join("world.db");
    let url = format!("sqlite://{}", db_path.display());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        common::seed(&db, &[]).await;
        db.close().await;
    });
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&d)
        .args([
            "serve",
            "--telnet-bind",
            "127.0.0.1:0",
            "--admin-bind",
            "127.0.0.1:0",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null());
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.env("SILVERMERE_DATABASE__URL", &url);
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let lines = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if let Some(a) = line.strip_prefix("silvermere: admin listening on ") {
                let _ = tx.send(a.to_string());
            }
            lines.push(line);
        }
        lines
    });
    let admin = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("admin line");
    let up = Instant::now() + Duration::from_secs(20);
    while !healthz(&admin) {
        assert!(Instant::now() < up, "/healthz never answered");
        std::thread::sleep(Duration::from_millis(100));
    }
    (
        Serve {
            child,
            lines,
            db_path,
            dir: d,
        },
        admin,
    )
}

#[cfg(unix)]
fn term(child: &std::process::Child) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(unix)]
fn wait_exit(child: &mut std::process::Child, within: Duration) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s;
        }
        assert!(started.elapsed() < within, "no exit within {within:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// SIGTERM stops `serve` cleanly within 15 s, exit code 0.
#[cfg(unix)]
#[test]
fn sigterm_exits_zero_with_the_database_checkpointed() {
    let mut s = start_serve("sigterm");
    term(&s.child);
    let status = wait_exit(&mut s.child, Duration::from_secs(15));
    let lines = s.lines.join().unwrap();
    assert_eq!(status.code(), Some(0), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("shutdown complete: 0 characters saved in ")),
        "{lines:?}"
    );
    // T-111: the file log holds the save and the last line, flushed before the exit
    let log = file_log(&s.dir);
    assert!(log.contains("characters saved: 0 committed"), "{log}");
    assert!(
        log.contains("shutdown complete: 0 characters saved"),
        "{log}"
    );
    // T-111: each phase is logged as it starts, and the commit comes before the drain
    let at = |needle: &str| {
        log.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} in {log}"))
    };
    let order = [
        "shutting down",
        "shutdown phase 1/4: saving characters",
        "shutdown phase 2/4: 0 saves queued",
        "characters saved: 0 committed",
        "shutdown phase 3/4: draining sessions",
        "shutdown phase 4/4: stopping the engine",
        "shutdown complete",
    ];
    for w in order.windows(2) {
        assert!(at(w[0]) < at(w[1]), "{:?} before {:?} in {log}", w[0], w[1]);
    }
    let wal = Path::new(&format!("{}-wal", s.db_path.display())).to_path_buf();
    assert!(!wal.exists(), "no -wal file left beside the database");
}

// A second signal while shutting down exits at once with code 130 and one line saying
// the data from the last sweep may be stale.
#[cfg(unix)]
#[test]
fn a_second_signal_during_shutdown_exits_130() {
    let mut s = start_serve("second");
    term(&s.child);
    // The drain pause keeps the first shutdown running for a moment.
    std::thread::sleep(Duration::from_millis(60));
    term(&s.child);
    let status = wait_exit(&mut s.child, Duration::from_secs(15));
    let lines = s.lines.join().unwrap();
    // On a very fast machine the first shutdown may already have finished.
    if status.code() == Some(0) {
        eprintln!("first shutdown finished before the second signal: {lines:?}");
        return;
    }
    assert_eq!(status.code(), Some(130), "{lines:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("data from the last sweep may be stale"))
            .count(),
        1,
        "{lines:?}"
    );
    // T-111: flushed before the exit
    let log = file_log(&s.dir);
    assert!(log.contains("second signal during shutdown"), "{log}");
}

async fn server(url: String) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts")
}

// A player in the realm is told, saved as a clean logoff (no hang-up mark), and the
// socket closes after the notice.
#[tokio::test]
async fn a_player_online_sees_the_notice_before_the_socket_closes() {
    const WAIT: Duration = Duration::from_secs(30);
    let (_t, url) = common::seeded(&[]).await;
    let db = Db::connect_and_migrate(&url).await.unwrap();
    let account = create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
        .await
        .unwrap();
    let c = common::stored_character("ALDRIC", "Aldric");
    db.characters().insert(account.id, &c).await.unwrap();
    db.close().await;

    let r = server(url.clone()).await;
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    client.send_line("aldric1").await.unwrap();
    client
        .until(r"Make your selection \(X to exit\):", WAIT)
        .await
        .unwrap();
    client.send_line("M").await.unwrap();
    client.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    client.send_line("E").await.unwrap();
    client.until(r"\[HP=35\]:", WAIT).await.unwrap();

    let done = r
        .shutdown_gracefully(
            Duration::from_secs(10),
            Duration::from_secs(15),
            Duration::from_secs(15),
        )
        .await
        .expect("graceful shutdown");
    assert_eq!(done.report.saved, 1);
    assert_eq!(done.report.closed, 1);
    // T-111: the character is committed first, and only then do the sessions
    // drain (Windows ends the process a few seconds after a sign-out)
    assert!(done.committed < silvermere::BARRIER_TIMEOUT, "{done:?}");
    assert!(
        done.elapsed >= done.committed + Duration::from_millis(250),
        "{done:?}"
    );
    let got = client
        .until(silvermere_engine::shutdown::NOTICE, WAIT)
        .await
        .unwrap();
    assert!(got.plain.contains(silvermere_engine::shutdown::NOTICE));
    // The server closed the socket after the notice.
    let closed = tokio::time::timeout(WAIT, client.until("NEVER-APPEARS", WAIT)).await;
    assert!(closed.is_ok_and(|r| r.is_err()), "socket closed");

    // The saved character carries no hang-up mark.
    let db = Db::connect_and_migrate(&url).await.unwrap();
    let row = db
        .characters()
        .by_account(account.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.character.status.contains(PlayerStatus::HUNGUP));
    db.close().await;
}

/// `silvermere sysop ARGS` against the server whose admin API is on `admin`; the exit
/// code and stdout.
#[cfg(unix)]
fn sysop(dir: &Path, admin: &str, token: &str, args: &[&str]) -> (Option<i32>, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).arg("sysop").args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.env("SILVERMERE_SERVER__ADMIN_BIND", admin)
        .env("SILVERMERE_ADMIN_TOKEN", token);
    let out = c.output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

// `silvermere sysop shutdown --in 2s`: the server stops by itself when the time comes, as
// on SIGTERM (exit 0, the shutdown complete), and says it was planned.
#[cfg(unix)]
#[test]
fn a_planned_shutdown_stops_the_server_when_due() {
    const TOKEN: &str = "planned-shutdown-test-token";
    let (mut s, admin) = start_serve_with("planned", &[("SILVERMERE_ADMIN_TOKEN", TOKEN)]);
    let (code, out) = sysop(
        &s.dir,
        &admin,
        TOKEN,
        &["shutdown", "--in", "2s", "testing"],
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(
        out.contains("The server will stop at") && out.contains("in 2 seconds"),
        "{out}"
    );
    let status = wait_exit(&mut s.child, Duration::from_secs(20));
    let lines = s.lines.join().unwrap();
    assert_eq!(status.code(), Some(0), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("planned shutdown")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("shutdown complete")),
        "{lines:?}"
    );
    let _ = std::fs::remove_dir_all(&s.dir);
}

// `--cancel` calls a plan off: the server keeps running.
#[cfg(unix)]
#[test]
fn a_cancelled_shutdown_leaves_the_server_running() {
    const TOKEN: &str = "planned-shutdown-test-token";
    let (mut s, admin) = start_serve_with("cancel", &[("SILVERMERE_ADMIN_TOKEN", TOKEN)]);
    let (code, out) = sysop(&s.dir, &admin, TOKEN, &["shutdown", "--in", "3s"]);
    assert_eq!(code, Some(0), "{out}");
    let (code, out) = sysop(&s.dir, &admin, TOKEN, &["shutdown", "--cancel"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Cancelled the planned shutdown"), "{out}");
    let (_, out) = sysop(&s.dir, &admin, TOKEN, &["shutdown", "--cancel"]);
    assert!(out.contains("No shutdown was planned"), "{out}");
    std::thread::sleep(Duration::from_secs(5));
    assert!(s.child.try_wait().unwrap().is_none(), "still running");
    assert!(healthz(&admin));
    term(&s.child);
    let status = wait_exit(&mut s.child, Duration::from_secs(15));
    assert_eq!(status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&s.dir);
}

// `silvermere sysop restart --in 1s`: the server stops as on SIGTERM, takes a backup, and
// starts again in the same process (what systemd and Docker see: the process never exits),
// listening on the same admin address; `--cancel` with nothing planned says so.
#[cfg(unix)]
#[test]
fn a_planned_restart_comes_back_in_the_same_process() {
    planned_restart("restart", false);
}

// The same with `serve` run again inside the process, as on Windows.
#[cfg(unix)]
#[test]
fn a_planned_restart_in_process_comes_back_too() {
    planned_restart("restart-in", true);
}

#[cfg(unix)]
fn planned_restart(name: &str, in_process: bool) {
    use std::sync::{Arc, Mutex};
    const TOKEN: &str = "planned-restart-test-token";
    let d = tmp(name);
    let db_path = d.join("world.db");
    let url = format!("sqlite://{}", db_path.display());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        common::seed(&db, &[]).await;
        db.close().await;
    });
    // a fixed admin port, so the server comes back where the test looks
    let admin = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    };
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&d)
        .args([
            "serve",
            "--telnet-bind",
            "127.0.0.1:0",
            "--admin-bind",
            &admin,
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null());
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.env("SILVERMERE_DATABASE__URL", &url)
        .env("SILVERMERE_ADMIN_TOKEN", TOKEN);
    if in_process {
        c.env("SILVERMERE_RESTART_IN_PROCESS", "1");
    }
    let mut child = c.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let lines = lines.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                lines.lock().unwrap().push(line);
            }
        });
    }
    let count = |needle: &str| {
        lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains(needle))
            .count()
    };
    let wait = |what: &str, done: &dyn Fn() -> bool| {
        let until = Instant::now() + Duration::from_secs(60);
        while !done() {
            assert!(
                Instant::now() < until,
                "{what}: {:?}",
                lines.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    wait("first start", &|| healthz(&admin));
    let (code, out) = sysop(&d, &admin, TOKEN, &["restart", "--in", "1s", "testing"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(
        out.contains("The server will restart at") && out.contains("in 1 second"),
        "{out}"
    );
    wait("the restart", &|| count("telnet listening on") == 2);
    wait("the admin API again", &|| healthz(&admin));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the same process runs on"
    );
    assert_eq!(count("planned restart"), 1);
    assert_eq!(count("shutdown complete"), 1);
    let backups: Vec<_> = std::fs::read_dir(d.join("data/backups"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".db"))
        .collect();
    assert_eq!(backups.len(), 1, "a backup before the restart");
    let (_, out) = sysop(&d, &admin, TOKEN, &["restart", "--cancel"]);
    assert!(out.contains("No restart or shutdown was planned"), "{out}");
    term(&child);
    let status = wait_exit(&mut child, Duration::from_secs(15));
    assert_eq!(status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&d);
}
