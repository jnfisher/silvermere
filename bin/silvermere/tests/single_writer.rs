//! T-072: `serve` holds the store's single-writer guard, so a second `serve` on the same
//! store refuses to start. On the backend `SILVERMERE_DATABASE_URL` names (one SQLite file
//! by default, one PostgreSQL database and schema otherwise).
//!
//! Unix only, as every test here was already: they stop the server with `kill -TERM`.
#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use silvermere_db::TestDb;

fn serve(url: &str) -> Command {
    silvermere(
        url,
        &[
            "serve",
            "--telnet-bind",
            "127.0.0.1:0",
            "--admin-bind",
            "127.0.0.1:0",
        ],
    )
}

fn silvermere(url: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(env!("CARGO_TARGET_TMPDIR"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.env("SILVERMERE_DATABASE__URL", url);
    c
}

/// Waits for the "telnet listening" line; the rest of stderr is drained in the background
/// into the returned buffer.
fn wait_listening(child: &mut Child) -> Arc<Mutex<String>> {
    let stderr = child.stderr.take().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn({
        let seen = Arc::clone(&seen);
        move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.starts_with("silvermere: telnet listening on ") {
                    let _ = tx.send(());
                }
                let mut seen = seen.lock().unwrap();
                seen.push_str(&line);
                seen.push('\n');
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(30))
        .expect("the first server starts");
    seen
}

fn stop(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let until = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > until {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_serve_on_one_store_refuses() {
    let t = TestDb::from_env().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let url = t.db().url().to_string();

    let mut first = serve(&url).spawn().unwrap();
    let seen = wait_listening(&mut first);
    // T-108: the store it serves, in the journal, without the password
    let line = format!("silvermere: database {}", silvermere_db::redact_url(&url));
    assert!(
        seen.lock().unwrap().contains(&line),
        "{}",
        seen.lock().unwrap()
    );
    let second = tokio::task::spawn_blocking({
        let url = url.clone();
        move || serve(&url).output().unwrap()
    })
    .await
    .unwrap();
    stop(&mut first);
    let err = String::from_utf8_lossy(&second.stderr);
    assert_eq!(second.status.code(), Some(1), "{err}");
    assert!(
        err.contains("another silvermere is already serving"),
        "{err}"
    );
    assert!(err.contains("held"), "{err}");

    // the guard goes with the first server: a new one starts
    let mut third = serve(&url).spawn().unwrap();
    wait_listening(&mut third);
    stop(&mut third);
    t.close().await;
}

/// T-073: `import` (like `restore` and `migrate`) refuses while a server holds the guard.
#[tokio::test(flavor = "multi_thread")]
async fn import_refuses_while_serve_holds_the_guard() {
    let t = TestDb::from_env().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let url = t.db().url().to_string();

    let mut server = serve(&url).spawn().unwrap();
    wait_listening(&mut server);
    let import = tokio::task::spawn_blocking({
        let url = url.clone();
        move || {
            silvermere(&url, &["import", "--from", "/nonexistent-t073"])
                .output()
                .unwrap()
        }
    });
    let import = import.await.unwrap();
    stop(&mut server);
    let err = String::from_utf8_lossy(&import.stderr);
    assert_eq!(import.status.code(), Some(1), "{err}");
    assert!(
        err.contains("a silvermere server is running") && err.contains("`import`"),
        "{err}"
    );
    t.close().await;
}

/// Ends the backend holding the PostgreSQL guard of `url`; returns its pid.
async fn kill_guard_backend(url: &str) -> i32 {
    let pid = silvermere_db::guard::postgres_holder(url)
        .await
        .unwrap()
        .expect("the server holds the guard");
    sqlx::any::install_default_drivers();
    let mut conn = <sqlx::AnyConnection as sqlx::Connection>::connect(url)
        .await
        .unwrap();
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 5000)")
        .bind(pid)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert!(killed, "backend {pid} ended");
    let _ = sqlx::Connection::close(conn).await;
    pid
}

/// T-073: on PostgreSQL, `serve` re-takes a guard whose connection was killed, and when
/// another process takes it first, flushes and exits non-zero.
#[tokio::test(flavor = "multi_thread")]
async fn serve_re_takes_a_killed_guard_and_stops_when_it_is_taken() {
    let t = TestDb::from_env().await.expect("test database");
    if t.db().dialect() != silvermere_db::Dialect::Postgres {
        return t.close().await;
    }
    common::seed(t.db(), &[]).await;
    let url = t.db().url().to_string();
    let holder = || silvermere_db::guard::postgres_holder(&url);

    let mut server = serve(&url)
        .env("SILVERMERE_GUARD_WATCHDOG_MS", "1500,3000")
        .spawn()
        .unwrap();
    let stderr = wait_listening(&mut server);

    // a killed connection: the server takes the lock again on a new one
    let old = kill_guard_backend(&url).await;
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match holder().await.unwrap() {
            Some(pid) if pid != old => break,
            _ if Instant::now() > until => panic!("the guard was not re-taken"),
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(server.try_wait().unwrap().is_none(), "still serving");
    // T-108: the loss and the re-take reach stderr (the journal under systemd), not only
    // the log file, so a silent recovery can be told from a silent loss
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let err = stderr.lock().unwrap().clone();
        if err.contains("writer guard connection failed; re-taking the lock")
            && err.contains("writer guard re-taken")
        {
            break;
        }
        assert!(Instant::now() < until, "no guard lines on stderr: {err}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // a competing holder takes it while it is gone: the server stops
    let mut other = None;
    for _ in 0..5 {
        kill_guard_backend(&url).await;
        if let silvermere_db::TryAcquire::Taken(g) =
            silvermere_db::WriterGuard::try_acquire(&url).await.unwrap()
        {
            other = Some(g);
            break;
        }
        // the server won the race; wait for its re-take and try again
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let other = other.expect("the competing holder took the guard");
    let until = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = server.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < until, "the server did not stop");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    // let the drain thread read the last lines
    tokio::time::sleep(Duration::from_millis(200)).await;
    let err = stderr.lock().unwrap().clone();
    assert_eq!(status.code(), Some(1), "{err}");
    assert!(
        err.contains("held elsewhere") && err.contains("flushing"),
        "{err}"
    );
    assert!(
        err.contains("shutdown complete"),
        "the queue was flushed: {err}"
    );
    other.release().await;
    t.close().await;
}
