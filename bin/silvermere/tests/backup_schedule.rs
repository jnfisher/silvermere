//! The daily backup in the server (T-150): the schedule against a clock set just before
//! `backup.at`, a failed run kept as the status while the schedule carries on, and the
//! admin API showing it.

use crate::common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use silvermere::backups::schedule::{self, WallClock};
use silvermere::backups::{self, Run, Shared};
use silvermere::config::{AdminCfg, Config, TokenCfg, TokenScope};
use silvermere_engine::{SeededRng, SystemClock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sm-backupsched-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A clock that reads `start` when the test begins and runs at the real rate from there.
struct From {
    start: jiff::Timestamp,
    began: Instant,
}

impl WallClock for From {
    fn now(&self) -> jiff::Timestamp {
        let span = jiff::SignedDuration::try_from(self.began.elapsed()).unwrap();
        self.start.checked_add(span).unwrap()
    }
}

fn at(s: &str) -> From {
    From {
        start: s.parse().unwrap(),
        began: Instant::now(),
    }
}

/// A migrated SQLite database in `dir/data`, and the config that backs it up at 04:30 UTC.
async fn setup(dir: &Path, keep: u32) -> Config {
    let mut cfg = Config {
        data_dir: dir.join("data"),
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    cfg.database.url = format!("sqlite://{}", cfg.data_dir.join("silvermere.db").display());
    let db = silvermere_db::Db::connect_and_migrate(&cfg.database.url)
        .await
        .unwrap();
    db.close().await;
    cfg.game.cleanup.tz = "UTC".into();
    cfg.backup.at = "04:30".into();
    cfg.backup.keep = keep;
    cfg
}

/// Waits (up to 20 s) for the schedule's first run to be recorded.
async fn first_run(shared: &Shared) -> Run {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(r) = shared.state.lock().unwrap().last.clone() {
            return r;
        }
        assert!(Instant::now() < deadline, "no scheduled run within 20 s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| silvermere_db::backup::is_backup_name(n))
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn the_schedule_fires_at_backup_at_and_prunes_to_backup_keep() {
    let d = tmp("fires");
    let cfg = setup(&d, 2).await;
    let folder = backups::folder(&cfg);
    std::fs::create_dir_all(&folder).unwrap();
    for old in [
        "silvermere-20240101-043000.db",
        "silvermere-20250101-043000.db",
    ] {
        std::fs::write(folder.join(old), b"old").unwrap();
    }
    let shared = Shared::load(&cfg);
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(schedule::run(
        shared.clone(),
        at("2026-10-27T04:29:58Z"),
        stop,
    ));
    // before it is due: armed for today's 04:30, nothing written
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        shared.state.lock().unwrap().next.as_deref(),
        Some("2026-10-27T04:30:00Z")
    );
    assert_eq!(names(&folder).len(), 2);

    let run = first_run(&shared).await;
    assert!(run.ok, "{run:?}");
    let made = run.file.clone().unwrap();
    assert_eq!(
        made.file_name().unwrap().to_str().unwrap(),
        "silvermere-20261027-043000.db"
    );
    assert!(
        silvermere_db::backup::format_of(&made).is_some(),
        "a database copy"
    );
    // pruned to 2: the oldest went
    assert_eq!(
        names(&folder),
        [
            "silvermere-20250101-043000.db",
            "silvermere-20261027-043000.db"
        ]
    );
    assert_eq!(run.pruned, [folder.join("silvermere-20240101-043000.db")]);
    // the status file says the same, and the next run is tomorrow's
    assert_eq!(backups::read_status(&folder), Some(run));
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.state.lock().unwrap().next.as_deref() != Some("2026-10-28T04:30:00Z") {
        assert!(Instant::now() < deadline, "not re-armed for tomorrow");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop_tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("stops when told")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_is_kept_as_the_status_and_the_schedule_carries_on() {
    let d = tmp("fails");
    let mut cfg = setup(&d, 30).await;
    // a database that cannot be opened
    cfg.database.url = format!(
        "sqlite://{}",
        d.join("no-such-folder/silvermere.db").display()
    );
    let shared = Shared::load(&cfg);
    let (_stop_tx, stop) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(schedule::run(
        shared.clone(),
        at("2026-10-27T04:29:59Z"),
        stop,
    ));
    let run = first_run(&shared).await;
    assert!(
        !run.ok && run.file.is_none() && run.hook.is_none(),
        "{run:?}"
    );
    assert!(run.error.is_some(), "{run:?}");
    assert_eq!(backups::read_status(&backups::folder(&cfg)), Some(run));
    // still running, armed for tomorrow
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.state.lock().unwrap().next.as_deref() != Some("2026-10-28T04:30:00Z") {
        assert!(Instant::now() < deadline, "not re-armed for tomorrow");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!task.is_finished());
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_enabled_false_runs_nothing() {
    let d = tmp("off");
    let mut cfg = setup(&d, 30).await;
    cfg.backup.enabled = false;
    let shared = Shared::load(&cfg);
    let (_stop_tx, stop) = tokio::sync::watch::channel(false);
    tokio::time::timeout(
        Duration::from_secs(5),
        schedule::run(shared.clone(), at("2026-10-27T04:29:59Z"), stop),
    )
    .await
    .expect("returns at once");
    assert!(shared.state.lock().unwrap().next.is_none());
}

async fn get(addr: std::net::SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut raw))
        .await
        .unwrap()
        .unwrap();
    let code = raw.split(' ').nth(1).unwrap().parse().unwrap();
    (
        code,
        raw.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_owned(),
    )
}

/// The last scheduled run (a failure here) is in `/healthz` and `/v1/backup`, and the server
/// stays healthy.
#[tokio::test]
async fn the_admin_api_shows_the_last_run_and_the_backups() {
    let d = tmp("api");
    let (_t, url) = common::seeded(&[]).await;
    let mut cfg = Config {
        data_dir: d.join("data"),
        ..Default::default()
    };
    cfg.database.url = url;
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.admin = AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: None,
            token: Some("t".into()),
            scopes: vec![TokenScope::Read],
        }],
        legacy_env_token: false,
    };
    let folder = backups::folder(&cfg);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("silvermere-20261026-043000.db"), b"x").unwrap();
    let failed = Run {
        at: "2026-10-27T04:30:00Z".into(),
        ok: false,
        file: None,
        bytes: None,
        error: Some("backup refused: disk full".into()),
        pruned: vec![],
        hook: None,
    };
    backups::write_status(&folder, &failed).unwrap();

    let running = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(1))
        .await
        .unwrap();
    let addr = running.admin_addr.unwrap();
    let (code, body) = get(addr, "/healthz", None).await;
    assert_eq!(code, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["backup"], "failed");

    assert_eq!(get(addr, "/v1/backup", None).await.0, 401);
    let (code, body) = get(addr, "/v1/backup", Some("t")).await;
    assert_eq!(code, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["enabled"], true);
    assert_eq!(v["at"], "04:30");
    assert_eq!(v["keep"], 30);
    assert_eq!(v["after"], false);
    assert_eq!(v["last"]["ok"], false);
    assert_eq!(v["last"]["error"], "backup refused: disk full");
    assert_eq!(v["backups"][0]["name"], "silvermere-20261026-043000.db");
    assert_eq!(v["backups"][0]["bytes"], 1);
    running.stop().await.unwrap();
}

/// A scheduled backup inside a running server: it is made while the server serves, and
/// `/healthz` and `/v1/backup` show it.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_backup_runs_inside_a_serving_server() {
    let d = tmp("in-server");
    let (_t, url) = common::seeded(&[]).await;
    if !url.starts_with("sqlite:") {
        return; // pg_dump may not be installed where the PostgreSQL suite runs
    }
    let mut cfg = Config {
        data_dir: d.join("data"),
        ..Default::default()
    };
    cfg.database.url = url;
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.game.cleanup.tz = "UTC".into();
    cfg.admin = AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: None,
            token: Some("t".into()),
            scopes: vec![TokenScope::Read],
        }],
        legacy_env_token: false,
    };
    let running = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(1))
        .await
        .unwrap();
    let addr = running.admin_addr.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&get(addr, "/healthz", None).await.1).unwrap()["backup"],
        "none"
    );
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(schedule::run(
        running.backups.clone(),
        at("2026-10-27T04:29:59Z"),
        stop,
    ));
    let run = first_run(&running.backups).await;
    assert!(run.ok, "{run:?}");
    let made = run.file.clone().unwrap();
    assert!(
        silvermere_db::backup::format_of(&made).is_some(),
        "{made:?}"
    );
    let (code, body) = get(addr, "/healthz", None).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["backup"],
        "ok"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while running.backups.state.lock().unwrap().next.as_deref() != Some("2026-10-28T04:30:00Z") {
        assert!(Instant::now() < deadline, "not re-armed for tomorrow");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let v: serde_json::Value =
        serde_json::from_str(&get(addr, "/v1/backup", Some("t")).await.1).unwrap();
    assert_eq!(v["last"]["ok"], true);
    assert_eq!(v["next"], "2026-10-28T04:30:00Z");
    assert_eq!(
        v["backups"][0]["name"],
        made.file_name().unwrap().to_str().unwrap()
    );
    stop_tx.send(true).unwrap();
    task.await.unwrap();
    running.stop().await.unwrap();
}
