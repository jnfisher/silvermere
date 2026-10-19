//! `silvermere backup` and `restore` through the binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sm-backupcli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.env(
        "SILVERMERE_DATABASE__URL",
        format!("sqlite://{}", dir.join("data/silvermere.db").display()),
    )
    .output()
    .unwrap()
}

fn events(dir: &Path) -> i64 {
    let url = format!("sqlite://{}", dir.join("data/silvermere.db").display());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db = silvermere_db::Db::connect(&url).await.unwrap();
        let n = sqlx::query_scalar("SELECT COUNT(*) FROM events")
            .fetch_one(&db.pool())
            .await
            .unwrap();
        db.close().await;
        n
    })
}

fn add_event(dir: &Path) {
    let url = format!("sqlite://{}", dir.join("data/silvermere.db").display());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db = silvermere_db::Db::connect(&url).await.unwrap();
        sqlx::query(
            "INSERT INTO events (at, kind, payload) VALUES ('2026-10-15T00:00:00Z', 'x', '{}')",
        )
        .execute(&db.pool())
        .await
        .unwrap();
        db.close().await;
    });
}

#[test]
fn backup_writes_to_the_default_folder_prunes_and_restore_puts_it_back() {
    let d = tmp("flow");
    assert!(run(&d, &["migrate"]).status.success());
    add_event(&d);
    let o = run(&d, &["backup", "--keep", "1"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let printed = String::from_utf8_lossy(&o.stdout).into_owned();
    let backups = d.join("data/backups");
    let files: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().collect();
    assert_eq!(files.len(), 1, "{printed}");
    let name = files[0].file_name().into_string().unwrap();
    assert!(silvermere_db::backup::is_backup_name(&name), "{name}");
    assert!(
        printed.contains(&name) && printed.contains("bytes"),
        "{printed}"
    );

    // Older backups beyond --keep go.
    std::fs::write(backups.join("silvermere-20200101-000000.db"), b"old").unwrap();
    let o = run(
        &d,
        &[
            "backup",
            "--keep",
            "1",
            "--to",
            backups
                .join("silvermere-29990101-000000.db")
                .to_str()
                .unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let mut left: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["silvermere-29990101-000000.db"]);

    // Change the database, restore, and the backed-up state is back.
    add_event(&d);
    assert_eq!(events(&d), 2);
    let o = run(
        &d,
        &[
            "restore",
            backups
                .join("silvermere-29990101-000000.db")
                .to_str()
                .unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(events(&d), 1);
    let kept = std::fs::read_dir(d.join("data"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".pre-restore-"));
    assert!(kept, "the previous database is kept");

    // A file that is not a database is refused with exit code 1.
    let junk = d.join("junk");
    std::fs::write(&junk, b"definitely not sqlite, just some text").unwrap();
    let o = run(&d, &["restore", junk.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a SQLite database"));
}

/// T-108: the asset files go beside the backup and come back with it; a restore reads the
/// file's kind from its bytes, not its name.
#[test]
fn assets_round_trip_and_the_format_comes_from_the_bytes() {
    let d = tmp("assets");
    assert!(run(&d, &["migrate"]).status.success());
    let assets = d.join("data/assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("WCC1393.STO"), b"the room text").unwrap();
    let dest = d.join("data/backups/silvermere-29990101-000000.db");
    let o = run(&d, &["backup", "--to", dest.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        out.contains("silvermere-29990101-000000.db.assets (1 files)"),
        "{out}"
    );
    // an `.assets` directory left after its database file was deleted is replaced
    std::fs::remove_file(&dest).unwrap();
    let o = run(&d, &["backup", "--to", dest.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // a new box: no assets; the restore brings them back
    std::fs::remove_dir_all(&assets).unwrap();
    let o = run(&d, &["restore", dest.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("restored 1 asset files"),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert_eq!(
        std::fs::read(assets.join("WCC1393.STO")).unwrap(),
        b"the room text"
    );

    // the same SQLite backup under a `.dump` name is still a SQLite backup
    let renamed = d.join("data/backups/silvermere-29990102-000000.dump");
    std::fs::copy(&dest, &renamed).unwrap();
    let o = run(&d, &["restore", renamed.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // a pg_dump archive is refused by a SQLite restore, saying what it is
    let pg = d.join("data/backups/silvermere-29990103-000000.db");
    std::fs::write(
        &pg,
        b"PGDMP\x01\x0e\x00 the rest of a custom-format archive",
    )
    .unwrap();
    let o = run(&d, &["restore", pg.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("is a PostgreSQL pg_dump archive, but the database is SQLite"),
        "{err}"
    );
}

/// T-108: the assets are copied before the database; when that copy fails, neither the
/// database copy nor a half `.assets` directory is left.
#[cfg(unix)]
#[test]
fn a_failed_assets_copy_leaves_no_half_backup() {
    let d = tmp("halfbackup");
    assert!(run(&d, &["migrate"]).status.success());
    let assets = d.join("data/assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("WCC1393.STO"), b"text").unwrap();
    std::os::unix::fs::symlink(d.join("nowhere"), assets.join("gone.txt")).unwrap();
    let dest = d.join("data/backups/silvermere-29990101-000000.db");
    let o = run(&d, &["backup", "--to", dest.to_str().unwrap()]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("gone.txt"), "{err}");
    assert!(!dest.exists(), "no database copy without its assets");
    let left: Vec<_> = std::fs::read_dir(d.join("data/backups"))
        .map(|r| r.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    // an existing destination is refused before anything is written
    std::fs::remove_file(assets.join("gone.txt")).unwrap();
    assert!(
        run(&d, &["backup", "--to", dest.to_str().unwrap()])
            .status
            .success()
    );
    let o = run(&d, &["backup", "--to", dest.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read(silvermere_db::backup::assets_dir(&dest).join("WCC1393.STO")).unwrap(),
        b"text"
    );
}
