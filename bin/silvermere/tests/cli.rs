//! CLI behaviour: help, config precedence and validation, `serve` with no world, stubs.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs the binary in `dir` with a clean `SILVERMERE_*` environment plus `env`.
fn run(dir: &PathBuf, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.envs(env.iter().copied()).output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn help_lists_subcommands() {
    let d = tmp("help");
    let o = run(&d, &[], &["--help"]);
    assert!(o.status.success());
    for s in ["serve", "import", "migrate", "account", "admin", "sysop"] {
        assert!(out(&o).contains(s), "missing {s}");
    }
}

#[test]
fn print_config_defaults() {
    let d = tmp("defaults");
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&o);
    assert!(t.contains("inactive_secs = 3600"));
    assert!(t.contains("death_hp = -200"));
    assert!(t.contains("telnet_bind = \"0.0.0.0:2323\""));
    assert!(t.contains("characters_per_account = 1"));
    assert!(t.contains("open_signup = true"));
}

/// `rng_seed` is absent by default and set by `--seed`, `--set` or the environment.
#[test]
fn rng_seed_layers() {
    let d = tmp("seed");
    let none = run(&d, &[], &["serve", "--print-config"]);
    assert!(!out(&none).contains("rng_seed"), "{}", out(&none));
    let flag = run(&d, &[], &["serve", "--print-config", "--seed", "7"]);
    assert!(
        out(&flag).contains("rng_seed = 7"),
        "{}{}",
        out(&flag),
        err(&flag)
    );
    let set = run(
        &d,
        &[],
        &["serve", "--print-config", "--set", "game.rng_seed=8"],
    );
    assert!(
        out(&set).contains("rng_seed = 8"),
        "{}{}",
        out(&set),
        err(&set)
    );
    let env = run(
        &d,
        &[("SILVERMERE_GAME__RNG_SEED", "9")],
        &["serve", "--print-config"],
    );
    assert!(
        out(&env).contains("rng_seed = 9"),
        "{}{}",
        out(&env),
        err(&env)
    );
    let both = run(
        &d,
        &[("SILVERMERE_GAME__RNG_SEED", "9")],
        &["serve", "--print-config", "--seed", "7"],
    );
    assert!(out(&both).contains("rng_seed = 7"), "flags beat env");
}

#[test]
fn env_overrides_defaults() {
    let d = tmp("env");
    let o = run(
        &d,
        &[("SILVERMERE_GAME__DEATH_HP", "-100")],
        &["serve", "--print-config"],
    );
    assert!(
        out(&o).contains("death_hp = -100"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn database_url_alias() {
    let d = tmp("alias");
    let o = run(
        &d,
        &[("SILVERMERE_DATABASE_URL", "postgres://u:p@h/db")],
        &["serve", "--print-config"],
    );
    assert!(
        out(&o).contains("url = \"postgres://u:****@h/db\""),
        "{}",
        err(&o)
    );
}

#[test]
fn precedence_flag_over_env_over_toml() {
    let d = tmp("prec");
    std::fs::write(d.join("silvermere.toml"), "[game]\ninactive_secs = 900\n").unwrap();
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert!(out(&o).contains("inactive_secs = 900"));
    let env = [("SILVERMERE_GAME__INACTIVE_SECS", "1200")];
    let o = run(&d, &env, &["serve", "--print-config"]);
    assert!(out(&o).contains("inactive_secs = 1200"));
    let o = run(
        &d,
        &env,
        &["serve", "--print-config", "--game.inactive-secs", "1500"],
    );
    assert!(out(&o).contains("inactive_secs = 1500"), "{}", err(&o));
}

#[test]
fn out_of_range_names_key_and_range() {
    let d = tmp("range");
    std::fs::write(d.join("silvermere.toml"), "[game]\ninactive_secs = 10\n").unwrap();
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("game.inactive_secs") && err(&o).contains("60..=32000"));
}

#[test]
fn serve_without_world_exits_3() {
    let d = tmp("noworld");
    let o = run(&d, &[], &["serve"]);
    assert_eq!(o.status.code(), Some(3), "{}", err(&o));
    assert!(err(&o).contains("No world data found"));
}

#[test]
fn unmerged_crates_exit_2() {
    let d = tmp("stubs");
    let o = run(&d, &[], &["sysop", "who"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("not implemented yet (T-018)"));
}

fn run_stdin(dir: &PathBuf, args: &[&str], stdin: &str) -> Output {
    use std::io::Write as _;
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    let mut child = c
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn account_create_grant_list_delete() {
    let d = tmp("account");
    let o = run_stdin(
        &d,
        &["account", "create", "Bob", "--sysop", "--password-stdin"],
        "secret\n",
    );
    assert!(o.status.success(), "{}", err(&o));
    let o = run_stdin(&d, &["account", "create", "bob"], "other\n");
    assert_eq!(o.status.code(), Some(1), "names are unique ignoring case");
    let o = run(&d, &[], &["account", "grant", "bob", "SYS_GOTO_KEY"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = run(&d, &[], &["account", "revoke", "bob", "wizard"]);
    assert_eq!(o.status.code(), Some(1));
    let o = run_stdin(&d, &["account", "passwd", "BOB"], "newsecret\n");
    assert!(o.status.success(), "{}", err(&o));
    let o = run(&d, &[], &["account", "list"]);
    assert!(o.status.success(), "{}", err(&o));
    let list = out(&o);
    assert!(
        list.contains("Bob") && list.contains("player,sysop,SYS_GOTO_KEY"),
        "{list}"
    );
    let o = run(&d, &[], &["account", "delete", "bob"]);
    assert_eq!(o.status.code(), Some(1), "needs --yes");
    // nothing on the admin port: deleted directly (T-069)
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let admin = closed.local_addr().unwrap().to_string();
    drop(closed);
    let o = run(
        &d,
        &[("SILVERMERE_SERVER__ADMIN_BIND", &admin)],
        &["account", "delete", "bob", "--yes"],
    );
    assert!(o.status.success(), "{}", err(&o));
    let o = run(&d, &[], &["account", "list"]);
    assert_eq!(out(&o), "");
}

#[tokio::test]
async fn migrate_creates_database_with_tables() {
    let d = tmp("migrate");
    let o = run(&d, &[], &["migrate"]);
    assert!(o.status.success(), "{}", err(&o));
    let db = d.join("data/silvermere.db");
    assert!(db.exists());
    let url = format!("sqlite://{}?mode=ro", db.display());
    let db = silvermere_db::Db::connect(&url).await.expect("open");
    for t in ["rooms", "accounts", "characters", "events"] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {t}")))
            .fetch_one(&db.pool())
            .await
            .unwrap_or_else(|e| panic!("table {t}: {e}"));
        assert_eq!(n, 0);
    }
    // A second run is a no-op.
    assert!(run(&d, &[], &["migrate"]).status.success());
}

/// Runs against PostgreSQL when CI points `SILVERMERE_DATABASE_URL` at one.
#[test]
fn migrate_postgres_when_configured() {
    let Ok(url) = std::env::var("SILVERMERE_DATABASE_URL") else {
        return;
    };
    if !url.starts_with("postgres") {
        return;
    }
    let d = tmp("migrate-pg");
    let o = run(&d, &[("SILVERMERE_DATABASE_URL", &url)], &["migrate"]);
    assert!(o.status.success(), "{}", err(&o));
    // T-108: the success line names the database without its password, and Neon's
    // `channel_binding` parameter is dropped before the driver can warn about it
    let sep = if url.contains('?') { '&' } else { '?' };
    let neon = format!("{url}{sep}channel_binding=require");
    let o = run(&d, &[("SILVERMERE_DATABASE_URL", &neon)], &["migrate"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("migrations applied to "), "{}", err(&o));
    if let (_, Some(password)) = silvermere_db::split_password(&url) {
        assert!(err(&o).contains(":****@"), "{}", err(&o));
        assert!(!err(&o).contains(&format!(":{password}@")), "{}", err(&o));
    }
    assert!(!err(&o).contains("channel_binding"), "{}", err(&o));
}

/// Every key in the printed config appears in silvermere.example.toml. (docs/config.md is
/// checked against the printed config by `docs_in_sync.rs`.)
#[test]
fn example_covers_every_key() {
    let d = tmp("docs");
    let o = run(&d, &[], &["serve", "--print-config"]);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ex = std::fs::read_to_string(root.join("silvermere.example.toml")).unwrap();
    for line in out(&o).lines() {
        if let Some((k, _)) = line.split_once(" = ") {
            let k = k.trim();
            assert!(ex.contains(&format!("{k} =")), "example lacks {k}");
        }
    }
}

/// The example file parses and equals the defaults.
#[test]
fn example_toml_matches_defaults() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let d = tmp("example");
    std::fs::copy(
        root.join("silvermere.example.toml"),
        d.join("silvermere.toml"),
    )
    .unwrap();
    let with = run(&d, &[], &["serve", "--print-config"]);
    let d2 = tmp("example-none");
    let without = run(&d2, &[], &["serve", "--print-config"]);
    assert!(with.status.success(), "{}", err(&with));
    assert_eq!(out(&with), out(&without));
}

/// Seeds a temp SQLite under `dir` with the fixture world (minus `without`) and returns
/// the environment that points `serve` at it.
fn seeded_env(dir: &std::path::Path, without: &[u32]) -> Vec<(String, String)> {
    let url = format!("sqlite://{}", dir.join("world.db").display());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = silvermere_db::Db::connect_and_migrate(&url).await.unwrap();
        common::seed(&db, without).await;
        db.close().await;
    });
    vec![("SILVERMERE_DATABASE__URL".to_string(), url)]
}

/// Layer 3: `serve` starts against a real world, listens, and exits cleanly on SIGTERM.
#[test]
fn serve_starts_and_stops_on_sigterm() {
    use std::io::BufRead;
    let d = tmp("sigterm");
    let env = seeded_env(&d, &[]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&d)
        .args([
            "serve",
            "--telnet-bind",
            "127.0.0.1:0",
            "--admin-bind",
            "127.0.0.1:0",
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null());
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut child = c.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if line.starts_with("silvermere: telnet listening on ") {
                let _ = tx.send(line.clone());
            }
            lines.push(line);
        }
        lines
    });
    let listening = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the listening line within 20 s");
    let addr = listening.trim_start_matches("silvermere: telnet listening on ");
    std::net::TcpStream::connect(addr).expect("connect to the telnet port");
    let kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "serve did not exit on SIGTERM"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let lines = reader.join().unwrap();
    assert!(status.success(), "exit {status}: {lines:?}");
}

/// Layer 3: `serve` refuses to start when a special room is missing from the world.
#[test]
fn serve_refuses_when_a_special_room_is_missing() {
    let d = tmp("no-jail");
    let env = seeded_env(&d, &[376]);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let o = run(
        &d,
        &env,
        &[
            "serve",
            "--telnet-bind",
            "127.0.0.1:0",
            "--admin-bind",
            "127.0.0.1:0",
        ],
    );
    assert!(!o.status.success());
    assert!(
        err(&o).contains("game.jail_room = 376: room does not exist on map 1"),
        "{}",
        err(&o)
    );
}

/// A local HTTP server standing in for the download site: answers every request with `body`
/// and counts the requests. Returns the package's URL on it.
fn fixture_server(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/{}",
        listener.local_addr().unwrap(),
        silvermere_testkit::stock::FILE_NAME
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            counted.fetch_add(1, Ordering::SeqCst);
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.ends_with(b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(&body);
        }
    });
    (url, hits)
}

/// T-107, T-120: `import` with no source downloads the stock distribution into
/// `<data_dir>/cache/`, checks it and builds the world from it, assets included; a second run
/// uses the cached copy; a cached copy that no longer matches is fetched again; a wrong
/// download is refused and nothing is kept; `import ZIP` works offline; `--live` needs `--from`.
#[test]
fn import_stock_with_no_from() {
    let d = tmp("import-stock");
    let package = std::fs::read(silvermere_testkit::stock::zip()).unwrap();
    let (url, hits) = fixture_server(package.clone());
    let env = [
        ("SILVERMERE_STOCK_URL", url.as_str()),
        ("NO_PROXY", "127.0.0.1"),
    ];
    // an interrupted import's copy (a pid no process has) is removed at the next import
    let stale = d.join("data/.import-4000000000");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("WCCMP001.VIR"), b"left over").unwrap();
    let o = run(&d, &env, &["--data-dir", "data", "import"]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let t = out(&o);
    assert!(err(&o).contains("downloading the stock MajorMUD 1.11p distribution"));
    assert!(err(&o).contains("importing the stock MajorMUD 1.11p distribution"));
    for line in [
        "rooms [WCCMP001.VIR]: 26720 records read, 26720 written, 0 skipped",
        "patches [WCCUPDAT.DAT]: 38754 records",
        "assets [WCCHSE.ZIP]: 47 files, 47 written, 0 kept",
        "assets [text files]: 5 files, 5 written, 0 kept",
    ] {
        assert!(t.contains(line), "missing {line:?} in\n{t}");
    }
    for f in [
        "silvermere.db",
        "assets/WCCMAP01.ANS",
        "assets/WCC85015.HSE",
        "assets/WCCMMSYS.NOT",
    ] {
        assert!(d.join("data").join(f).is_file(), "{f}");
    }
    let cached = d
        .join("data/cache")
        .join(silvermere_testkit::stock::FILE_NAME);
    assert_eq!(std::fs::read(&cached).unwrap(), package);
    // the unpacked copy is gone
    let left: Vec<_> = std::fs::read_dir(d.join("data"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".import-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");

    // a second run uses the cached copy
    let o = run(&d, &env, &["--data-dir", "data", "import", "--dry-run"]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(err(&o).contains("using the stock"), "{}", err(&o));

    // a corrupted cached copy is refused and fetched again
    std::fs::write(&cached, b"corrupted").unwrap();
    let o = run(&d, &env, &["--data-dir", "data", "import", "--dry-run"]);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert!(err(&o).contains("did not match its sha256"), "{}", err(&o));
    assert_eq!(std::fs::read(&cached).unwrap(), package);

    // --download fetches it again even when the cached copy is good
    let o = run(
        &d,
        &env,
        &["--data-dir", "data", "import", "--download", "--dry-run"],
    );
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    // a download that is not the package: refused, named, nothing kept
    let e = tmp("import-stock-wrong");
    let (bad_url, _) = fixture_server(package[..package.len() / 2].to_vec());
    let bad_env = [
        ("SILVERMERE_STOCK_URL", bad_url.as_str()),
        ("NO_PROXY", "127.0.0.1"),
    ];
    let o = run(&e, &bad_env, &["--data-dir", "data", "import", "--dry-run"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains(&bad_url), "{}", err(&o));
    assert!(err(&o).contains("To fetch it by hand"), "{}", err(&o));
    let kept: Vec<_> = std::fs::read_dir(e.join("data/cache"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".lock")
        .collect();
    assert!(kept.is_empty(), "{kept:?}");

    // a zip given on the command line is imported offline, as is --from
    let zip = silvermere_testkit::stock::zip();
    let zip = zip.to_str().unwrap();
    let offline = [("SILVERMERE_STOCK_URL", "http://127.0.0.1:9/nowhere.zip")];
    for args in [
        &["--data-dir", "data", "import", zip, "--dry-run"][..],
        &["--data-dir", "data", "import", "--from", zip, "--dry-run"],
    ] {
        let o = run(&e, &offline, args);
        assert!(o.status.success(), "{args:?}: {}{}", out(&o), err(&o));
        assert!(
            out(&o).contains("assets [WCCHSE.ZIP]: 47 files, 47 to write"),
            "{}",
            out(&o)
        );
    }

    let o = run(&d, &[], &["--data-dir", "data", "import", "--live"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("--from"), "{}", err(&o));
    let o = run(
        &d,
        &[],
        &["--data-dir", "data", "import", "--from", "nowhere"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("nowhere: no such directory or file"),
        "{}",
        err(&o)
    );
}

/// T-108: the database password never prints, in `--print-config` or in an error.
#[test]
fn database_password_never_prints() {
    let d = tmp("redact");
    let url = "postgres://sm:hunter2-secret@127.0.0.1:1/db?sslmode=require";
    let o = run(
        &d,
        &[("SILVERMERE_DATABASE_URL", url)],
        &["serve", "--print-config"],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("postgres://sm:****@127.0.0.1:1/db"),
        "{}",
        out(&o)
    );
    assert!(!out(&o).contains("hunter2"));
    // every command that opens the database, failing to reach it
    for args in [
        &["migrate"][..],
        &["import"],
        &["account", "create", "someone"],
        &["serve"],
        &["backup"],
    ] {
        let o = run(&d, &[("SILVERMERE_DATABASE_URL", url)], args);
        assert!(!o.status.success(), "{args:?}");
        assert!(!err(&o).contains("hunter2"), "{args:?}: {}", err(&o));
        assert!(!out(&o).contains("hunter2"), "{args:?}: {}", out(&o));
    }
    let o = run(&d, &[("SILVERMERE_DATABASE_URL", url)], &["migrate"]);
    assert!(err(&o).contains("sm:****@127.0.0.1:1"), "{}", err(&o));
}

/// T-108: Neon's copied URL ends in `channel_binding=require`, which the driver ignores
/// with a warning on every connection; the effective URL drops it.
#[test]
fn channel_binding_is_dropped() {
    let d = tmp("channel-binding");
    let url = "postgres://sm:pw@ep-a.neon.tech/db?sslmode=require&channel_binding=require";
    let o = run(
        &d,
        &[("SILVERMERE_DATABASE_URL", url)],
        &["serve", "--print-config"],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("url = \"postgres://sm:****@ep-a.neon.tech/db?sslmode=require\""),
        "{}",
        out(&o)
    );
}

/// T-108: a transaction-mode pooler is refused before connecting, with the fix named.
#[test]
fn a_pooled_database_url_is_refused() {
    let d = tmp("pooler");
    let url = "postgres://sm:hunter2-secret@ep-calm-a1b2-pooler.us-east-2.aws.neon.tech/db?sslmode=require";
    let o = run(&d, &[("SILVERMERE_DATABASE_URL", url)], &["migrate"]);
    assert!(!o.status.success());
    assert!(
        err(&o).contains("direct (unpooled) connection"),
        "{}",
        err(&o)
    );
    assert!(!err(&o).contains("hunter2"), "{}", err(&o));
}

/// T-124: `config path` names the file `serve` would read.
#[test]
fn config_path_names_the_file() {
    let d = tmp("cfg-path");
    let o = run(&d, &[], &["config", "path"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("silvermere.toml (does not exist yet"),
        "{}",
        out(&o)
    );
    // A data directory given (as Docker does) puts a new file there.
    let o = run(&d, &[("SILVERMERE_DATA_DIR", "/data")], &["config", "path"]);
    assert!(out(&o).starts_with("/data/silvermere.toml"), "{}", out(&o));
    // An existing ./silvermere.toml wins, as at startup.
    std::fs::write(d.join("silvermere.toml"), "").unwrap();
    let o = run(&d, &[("SILVERMERE_DATA_DIR", "/data")], &["config", "path"]);
    let want = format!("{} (exists)", d.join("silvermere.toml").display());
    assert_eq!(out(&o).trim(), want);
}

/// T-124: `config show` says where each value comes from and redacts secrets.
#[test]
fn config_show_has_provenance_and_redaction() {
    let d = tmp("cfg-show");
    std::fs::write(
        d.join("silvermere.toml"),
        "[game]\ndeath_hp = -50\n\n[[admin.tokens]]\nname = \"a\"\ntoken = \"hunter2-token\"\n",
    )
    .unwrap();
    let env = [
        ("SILVERMERE_SERVER__MAX_SESSIONS", "9"),
        ("SILVERMERE_DATABASE_URL", "postgres://sm:hunter2-pw@h/db"),
    ];
    let o = run(&d, &env, &["--log-level", "debug", "config", "show"]);
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&o);
    for (key, src) in [
        ("game.death_hp = -50", "# file"),
        (
            "server.max_sessions = 9",
            "# env SILVERMERE_SERVER__MAX_SESSIONS",
        ),
        ("logging.level = \"debug\"", "# flag"),
        ("game.inactive_secs = 3600", "# default"),
        (
            "database.url = \"postgres://sm:****@h/db\"",
            "# env SILVERMERE_DATABASE_URL",
        ),
        ("game.rng_seed = (not set)", "# default"),
    ] {
        let line = t
            .lines()
            .find(|l| l.starts_with(key))
            .unwrap_or_else(|| panic!("{key}\n{t}"));
        assert!(line.ends_with(src), "{line}");
    }
    assert!(!t.contains("hunter2"), "{t}");
    assert!(t.contains("<redacted>"), "{t}");
    let o = run(&d, &[], &["config", "show", "game.death_hp"]);
    assert_eq!(out(&o).lines().count(), 2, "{}", out(&o));
    // `--print-config` keeps working.
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert!(out(&o).contains("death_hp = -50"), "{}", err(&o));
}

const COMMENTED: &str =
    "# my server\n[game]\n# how low\ndeath_hp = -200 # the default\nmax_top = 50\n";

/// T-124: `set` keeps the file's comments and order, and `unset` comments the key out.
#[test]
fn config_set_unset_round_trip_keeps_comments() {
    let d = tmp("cfg-set");
    let f = d.join("silvermere.toml");
    std::fs::write(&f, COMMENTED).unwrap();
    let o = run(&d, &[], &["config", "set", "game.death_hp", "-50"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("game.death_hp: -200 -> -50"),
        "{}",
        out(&o)
    );
    assert!(
        out(&o).contains("restart the server to apply"),
        "{}",
        out(&o)
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        COMMENTED.replace("-200 #", "-50 #")
    );
    let o = run(&d, &[], &["config", "set", "game.death_hp", "-200"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), COMMENTED);
    let o = run(&d, &[], &["config", "set", "compat.charset", "utf8"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        std::fs::read_to_string(&f)
            .unwrap()
            .contains("charset = \"utf8\"")
    );
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert!(out(&o).contains("charset = \"utf8\""), "{}", err(&o));
    let o = run(&d, &[], &["config", "unset", "compat.charset"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("\"utf8\" -> \"cp437\" (default)"),
        "{}",
        out(&o)
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        format!("{COMMENTED}\n[compat]\n# charset = \"utf8\"  # unset by silvermere config\n")
    );
    let o = run(&d, &[], &["serve", "--print-config"]);
    assert!(out(&o).contains("charset = \"cp437\""), "{}", err(&o));
}

/// T-124: `unset` comments the line out and keeps the comment lines above it in place.
#[test]
fn config_unset_comments_the_line_out() {
    let d = tmp("cfg-unset");
    let f = d.join("silvermere.toml");
    std::fs::write(&f, COMMENTED).unwrap();
    let o = run(&d, &[], &["config", "unset", "game.death_hp"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("-200 -> -200 (default)"), "{}", out(&o));
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "# my server\n[game]\n# how low\n# death_hp = -200  # unset by silvermere config\nmax_top = 50\n"
    );
    // The last key of the file too.
    let o = run(&d, &[], &["config", "unset", "game.max_top"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        std::fs::read_to_string(&f)
            .unwrap()
            .ends_with("# death_hp = -200  # unset by silvermere config\n# max_top = 50  # unset by silvermere config\n"),
    );
}

/// T-124: `set` of a key that is commented out replaces that line instead of adding another.
#[test]
fn config_set_replaces_a_commented_out_line() {
    let d = tmp("cfg-reset");
    let f = d.join("silvermere.toml");
    std::fs::write(
        &f,
        "[game]\n# how low\n# death_hp = -100\nmax_top = 50\n\n[compat]\n# charset = \"utf8\"\n# the end\n",
    )
    .unwrap();
    let o = run(&d, &[], &["config", "set", "game.death_hp", "-9"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = run(&d, &[], &["config", "set", "compat.charset", "utf8"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "[game]\n# how low\ndeath_hp = -9\nmax_top = 50\n\n[compat]\ncharset = \"utf8\"\n# the end\n"
    );
    // unset then set leaves one line.
    for args in [["unset", "game.max_top", ""], ["set", "game.max_top", "20"]] {
        let args: Vec<&str> = std::iter::once("config")
            .chain(args)
            .filter(|a| !a.is_empty())
            .collect();
        let o = run(&d, &[], &args);
        assert!(o.status.success(), "{}", err(&o));
    }
    let text = std::fs::read_to_string(&f).unwrap();
    assert_eq!(text.matches("max_top").count(), 1, "{text}");
    assert!(text.contains("death_hp = -9\nmax_top = 20\n"), "{text}");
}

/// T-124: a new file is created mode 600.
#[cfg(unix)]
#[test]
fn config_set_creates_the_file_mode_600() {
    use std::os::unix::fs::PermissionsExt as _;
    let d = tmp("cfg-new");
    let o = run(&d, &[], &["config", "set", "game.max_top", "20"]);
    assert!(o.status.success(), "{}", err(&o));
    let f = d.join("silvermere.toml");
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "[game]\nmax_top = 20\n"
    );
    assert_eq!(
        std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// T-124: an out-of-range value is refused with startup's message and nothing is written.
#[test]
fn config_set_out_of_range_is_refused() {
    let d = tmp("cfg-range");
    let f = d.join("silvermere.toml");
    std::fs::write(&f, COMMENTED).unwrap();
    let o = run(&d, &[], &["config", "set", "game.inactive_secs", "10"]);
    assert_eq!(o.status.code(), Some(1));
    std::fs::write(d.join("probe.toml"), "[game]\ninactive_secs = 10\n").unwrap();
    let startup = run(
        &d,
        &[],
        &["--config", "probe.toml", "serve", "--print-config"],
    );
    let msg = err(&startup);
    let msg = msg.trim().trim_start_matches("error: ");
    assert!(msg.contains("game.inactive_secs"), "{msg}");
    assert!(err(&o).contains(msg), "{}\nwant {msg}", err(&o));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), COMMENTED);
}

/// T-124: unknown keys and keys holding secrets are refused, with the way to set them.
#[test]
fn config_set_refuses_unknown_and_secret_keys() {
    let d = tmp("cfg-refuse");
    let o = run(&d, &[], &["config", "set", "game.no_such_key", "1"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("unknown key `game.no_such_key`"),
        "{}",
        err(&o)
    );
    let o = run(&d, &[], &["config", "set", "admin.tokens", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("silvermere admin token new"),
        "{}",
        err(&o)
    );
    let o = run(
        &d,
        &[],
        &["config", "set", "database.url", "postgres://a:b@h/d"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("--database-url-from-stdin"), "{}", err(&o));
    assert!(!d.join("silvermere.toml").exists());
}

/// T-124: a set that an environment variable overrides says so.
#[test]
fn config_set_reports_an_env_override() {
    let d = tmp("cfg-env");
    let env = [("SILVERMERE_GAME__DEATH_HP", "-7")];
    let o = run(&d, &env, &["config", "set", "game.death_hp", "-50"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("SILVERMERE_GAME__DEATH_HP is set and overrides the file"),
        "{}",
        out(&o)
    );
}

/// A fake editor: the n-th time it runs it writes what `$FAKE_EDITS` lists n-th (`|` between
/// files, `;` for a newline).
#[cfg(unix)]
fn fake_editor(dir: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let script = dir.join("fake-editor.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nn=$(cat \"$0.count\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"$0.count\"\n\
         printf '%s' \"$FAKE_EDITS\" | cut -d'|' -f$n | tr ';' '\\n' > \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
fn run_edit(dir: &PathBuf, edits: &str, answers: &str) -> Output {
    use std::io::Write as _;
    let editor = fake_editor(dir);
    let _ = std::fs::remove_file(editor.with_extension("sh.count"));
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(["config", "edit"]);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    let mut child = c
        .env("VISUAL", &editor)
        .env_remove("EDITOR")
        .env("FAKE_EDITS", edits)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answers.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// T-124: `config edit` saves a valid edit; an invalid one is shown and re-opened, and the
/// file changes only once the edit is valid; declining leaves the file as it was.
#[cfg(unix)]
#[test]
fn config_edit_valid_invalid_then_fixed() {
    let d = tmp("cfg-edit");
    let f = d.join("silvermere.toml");
    std::fs::write(&f, COMMENTED).unwrap();

    let o = run_edit(&d, "[game];death_hp = -9", "");
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("restart the server to apply"),
        "{}",
        out(&o)
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "[game]\ndeath_hp = -9\n"
    );

    let o = run_edit(&d, "[server];max_sessions = 0|[game];death_hp = -8", "y\n");
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("server.max_sessions = 0 is out of range"),
        "{}",
        err(&o)
    );
    assert!(err(&o).contains("edit it again?"), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "[game]\ndeath_hp = -8\n"
    );

    let o = run_edit(&d, "[game;", "n\n");
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("not saved"), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "[game]\ndeath_hp = -8\n"
    );
}

/// T-124: a set replaces the file through a temporary file and keeps an existing file's mode.
#[cfg(unix)]
#[test]
fn config_set_keeps_an_existing_files_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let d = tmp("cfg-mode");
    let f = d.join("silvermere.toml");
    std::fs::write(&f, COMMENTED).unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
    let o = run(&d, &[], &["config", "set", "game.max_top", "20"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(
        std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        std::fs::read_to_string(&f)
            .unwrap()
            .contains("max_top = 20")
    );
    let left: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "silvermere.toml")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

/// T-124: `unset` of a secret key is refused with its own way out.
#[test]
fn config_unset_refuses_secret_keys() {
    let d = tmp("cfg-unset-secret");
    let o = run(&d, &[], &["config", "unset", "admin.tokens"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("`config unset` does not touch it"),
        "{}",
        err(&o)
    );
    let o = run(&d, &[], &["config", "unset", "database.url"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).contains("SILVERMERE_DATABASE_URL line"),
        "{}",
        err(&o)
    );
}
