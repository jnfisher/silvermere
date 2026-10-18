//! Backup and restore.
//!
//! SQLite: `VACUUM INTO` writes a consistent copy while the server keeps writing (the
//! copy is taken inside one read transaction). It fails when the destination exists, so
//! the copy goes to a `.partial` name and is renamed when complete. PostgreSQL: `pg_dump
//! --format=custom` when the tool is on `PATH`. The schema holds no session-only state
//! (groups, following), so a backup never contains any.
//!
//! Restore is a file swap for SQLite, with the refusals checked in the order the CLI
//! reports them ([`check_restorable`]); PostgreSQL goes through `pg_restore`. A backup's
//! kind is read from its first bytes ([`format_of`]), not its name.
//!
//! The room description, map and menu text files (`data/assets/`) go beside the database
//! copy, in `<backup>.assets/` ([`assets_dir`], T-108).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use sqlx::{AnyConnection, AnyPool, Connection, Executor, Row};

use crate::DbError;
use crate::dialect::{Dialect, split_password};

/// Prefix of the files `silvermere backup` names and `--keep` prunes.
pub const NAME_PREFIX: &str = "silvermere-";
/// Their suffix on SQLite (a database file) and PostgreSQL (a `pg_dump` archive).
pub const NAME_SUFFIX: &str = ".db";
pub const PG_NAME_SUFFIX: &str = ".dump";

/// What a backup file holds, read from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A SQLite database (`SQLite format 3\0`).
    Sqlite,
    /// A `pg_dump --format=custom` archive (`PGDMP`).
    PgDump,
}

impl Format {
    /// The backend whose `restore` takes this format.
    pub fn dialect(self) -> Dialect {
        match self {
            Self::Sqlite => Dialect::Sqlite,
            Self::PgDump => Dialect::Postgres,
        }
    }
}

/// The format of the backup at `file`, `None` when it is neither (or unreadable).
pub fn format_of(file: &Path) -> Option<Format> {
    let mut magic = [0u8; 16];
    let n = fs::File::open(file)
        .and_then(|mut f| io::Read::read(&mut f, &mut magic))
        .ok()?;
    if n == 16 && &magic == b"SQLite format 3\0" {
        Some(Format::Sqlite)
    } else if magic[..n].starts_with(b"PGDMP") {
        Some(Format::PgDump)
    } else {
        None
    }
}

/// `base`, or `base-2`, `base-3`... when that is taken: two restores in one second do not
/// collide on their `.pre-restore-<time>` names (T-108).
pub fn free_name(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }
    (2..)
        .map(|i| sibling(&base, &format!("-{i}")))
        .find(|p| !p.exists())
        .expect("a free name")
}

/// Where the asset files of the backup `file` go: `<file>.assets/` beside it.
pub fn assets_dir(file: &Path) -> PathBuf {
    sibling(file, ".assets")
}

/// Copies the files of `from` (with its subdirectories) into `to`, which must not exist:
/// to a `.partial` name first, renamed when complete, so `to` is whole or absent. Symbolic
/// links are followed; a dangling one fails the copy, naming it. Returns the number of
/// files.
pub fn copy_assets(from: &Path, to: &Path) -> io::Result<usize> {
    fn copy_tree(from: &Path, to: &Path) -> io::Result<usize> {
        fs::create_dir_all(to)?;
        let mut n = 0;
        for entry in fs::read_dir(from)? {
            let path = entry?.path();
            let target = to.join(path.file_name().unwrap_or_default());
            let meta = fs::metadata(&path)
                .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
            if meta.is_dir() {
                n += copy_tree(&path, &target)?;
            } else {
                fs::copy(&path, &target)
                    .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
                n += 1;
            }
        }
        Ok(n)
    }
    let tmp = partial(to);
    let _ = fs::remove_dir_all(&tmp);
    let n = copy_tree(from, &tmp).inspect_err(|_| {
        let _ = fs::remove_dir_all(&tmp);
    })?;
    fs::rename(&tmp, to)?;
    Ok(n)
}

/// Why a backup or restore did not happen.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("database: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// `pg_dump` or `pg_restore` is not on `PATH`.
    #[error("{tool} not found: install postgresql-client or back up with your own tooling")]
    ToolNotFound { tool: &'static str },
    /// `pg_dump` or `pg_restore` ran and failed.
    #[error("{tool} failed: {status}")]
    ToolFailed { tool: &'static str, status: String },
    /// The file to restore is not usable; the message says why.
    #[error("{0}")]
    Refused(String),
}

/// `silvermere-YYYYMMDD-HHMMSS.db` (SQLite) or `.dump` (PostgreSQL) for `at` (UTC).
pub fn backup_name(at: jiff::Timestamp, dialect: Dialect) -> String {
    let suffix = match dialect {
        Dialect::Sqlite => NAME_SUFFIX,
        Dialect::Postgres => PG_NAME_SUFFIX,
    };
    format!("{NAME_PREFIX}{}{suffix}", at.strftime("%Y%m%d-%H%M%S"))
}

/// Whether `name` is a file `silvermere backup` would have named, on either backend.
pub fn is_backup_name(name: &str) -> bool {
    let Some(mid) = name.strip_prefix(NAME_PREFIX).and_then(|n| {
        n.strip_suffix(NAME_SUFFIX)
            .or_else(|| n.strip_suffix(PG_NAME_SUFFIX))
    }) else {
        return false;
    };
    let b = mid.as_bytes();
    b.len() == 15
        && b[8] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// Deletes the backups in `dir` beyond the newest `keep` (the names sort by time), each
/// with its `.assets` directory, and any `.assets` directory whose backup is gone. Only
/// files count as backups. Returns the paths removed.
pub fn prune(dir: &Path, keep: usize) -> io::Result<Vec<PathBuf>> {
    let entries: Vec<(String, bool)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let is_file = e.file_type().ok()?.is_file();
            Some((e.file_name().into_string().ok()?, is_file))
        })
        .collect();
    let mut names: Vec<String> = entries
        .iter()
        .filter(|(n, is_file)| *is_file && is_backup_name(n))
        .map(|(n, _)| n.clone())
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for name in names.into_iter().take(excess) {
        let path = dir.join(name);
        fs::remove_file(&path)?;
        let assets = assets_dir(&path);
        if assets.is_dir() {
            fs::remove_dir_all(&assets)?;
        }
        removed.push(path);
    }
    // an `.assets` directory left without its backup (the file removed by hand)
    for (name, is_file) in &entries {
        let Some(backup) = name.strip_suffix(".assets") else {
            continue;
        };
        let path = dir.join(name);
        if !is_file && is_backup_name(backup) && !dir.join(backup).exists() && path.is_dir() {
            fs::remove_dir_all(&path)?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// `PRAGMA wal_checkpoint(TRUNCATE)` on SQLite, so no `-wal` file is left beside the
/// database at a clean stop; nothing on PostgreSQL.
pub async fn checkpoint(db: &crate::Db) -> Result<(), BackupError> {
    if db.dialect() == Dialect::Sqlite {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&db.pool())
            .await?;
    }
    Ok(())
}

fn partial(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

/// Writes a consistent copy of the SQLite database behind `pool` to `dest`.
pub async fn backup_sqlite(pool: &AnyPool, dest: &Path) -> Result<(), BackupError> {
    if dest.exists() {
        return Err(BackupError::Refused(format!(
            "{} already exists",
            dest.display()
        )));
    }
    if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = partial(dest);
    let _ = fs::remove_file(&tmp);
    let result = sqlx::query("VACUUM INTO ?")
        .bind(tmp.to_string_lossy().into_owned())
        .execute(pool)
        .await;
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn run_tool(tool: &'static str, cmd: &mut Command) -> Result<(), BackupError> {
    match cmd.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(BackupError::ToolFailed {
            tool,
            status: s.to_string(),
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(BackupError::ToolNotFound { tool }),
        Err(e) => Err(e.into()),
    }
}

/// `pg_dump --format=custom --file DEST --dbname URL` (the password in `PGPASSWORD`), when `pg_dump` is on `PATH`.
pub fn backup_postgres(url: &str, dest: &Path) -> Result<(), BackupError> {
    if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = partial(dest);
    run_tool(
        "pg_dump",
        pg_tool("pg_dump", url)
            .arg("--format=custom")
            .arg("--file")
            .arg(&tmp)
            .arg("--dbname")
            .arg(split_password(url).0),
    )
    .inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

/// `pg_restore --clean --if-exists --no-owner -d URL FILE`.
pub fn restore_postgres(url: &str, file: &Path) -> Result<(), BackupError> {
    run_tool(
        "pg_restore",
        pg_tool("pg_restore", url)
            .args(["--clean", "--if-exists", "--no-owner", "-d"])
            .arg(split_password(url).0)
            .arg(file),
    )
}

/// `tool` with the URL's password in `PGPASSWORD`, so the command line others can see in
/// `ps` holds none (T-108); the caller passes the URL without it.
fn pg_tool(tool: &str, url: &str) -> Command {
    let mut c = Command::new(tool);
    if let (_, Some(password)) = split_password(url) {
        c.env("PGPASSWORD", password);
    }
    c
}

fn sqlite_url(path: &Path, mode: &str) -> String {
    format!("sqlite://{}?mode={mode}", path.display())
}

/// The newest applied migration recorded in the database file (0 when none).
async fn latest_migration(file: &Path) -> Result<i64, BackupError> {
    sqlx::any::install_default_drivers();
    let mut conn = AnyConnection::connect(&sqlite_url(file, "ro")).await?;
    let row = sqlx::query(
        "SELECT COALESCE(MAX(version), 0) AS v FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut conn)
    .await;
    let version = match row {
        Ok(r) => r.try_get::<i64, _>("v")?,
        // No migrations table: a SQLite file that is not one of ours.
        Err(_) => {
            let _ = conn.close().await;
            return Err(BackupError::Refused(
                "the file is a SQLite database but has no silvermere schema".into(),
            ));
        }
    };
    let _ = conn.close().await;
    Ok(version)
}

async fn integrity_ok(file: &Path) -> Result<bool, BackupError> {
    sqlx::any::install_default_drivers();
    let mut conn = AnyConnection::connect(&sqlite_url(file, "ro")).await?;
    let rows = sqlx::query("PRAGMA integrity_check")
        .fetch_all(&mut conn)
        .await?;
    let ok = rows.len() == 1 && rows[0].try_get::<String, _>(0).is_ok_and(|s| s == "ok");
    let _ = conn.close().await;
    Ok(ok)
}

/// Fails with "server appears to be running" when something else holds the database at
/// `live`: opens it with `busy_timeout=0` and exclusive locking and takes `BEGIN
/// EXCLUSIVE`. A missing file is fine (nothing holds it).
pub async fn ensure_not_in_use(live: &Path) -> Result<(), BackupError> {
    if !live.exists() {
        return Ok(());
    }
    sqlx::any::install_default_drivers();
    let busy = || {
        BackupError::Refused(format!(
            "server appears to be running: {} is in use. Stop it first.",
            live.display()
        ))
    };
    let mut conn = AnyConnection::connect(&sqlite_url(live, "rw"))
        .await
        .map_err(|_| busy())?;
    let held = async {
        conn.execute("PRAGMA busy_timeout = 0").await?;
        conn.execute("PRAGMA locking_mode = EXCLUSIVE").await?;
        conn.execute("BEGIN EXCLUSIVE").await?;
        conn.execute("ROLLBACK").await?;
        Ok::<_, sqlx::Error>(())
    }
    .await;
    let _ = conn.close().await;
    held.map_err(|_| busy())
}

/// The refusals in the order the CLI reports them: `file` is a SQLite database, its
/// `PRAGMA integrity_check` is `ok`, its newest migration is not newer than the binary's,
/// and nothing holds the live database at `live`.
pub async fn check_restorable(file: &Path, live: &Path) -> Result<(), BackupError> {
    if format_of(file) != Some(Format::Sqlite) {
        return Err(BackupError::Refused(format!(
            "{} is not a SQLite database",
            file.display()
        )));
    }
    if !integrity_ok(file).await? {
        return Err(BackupError::Refused(format!(
            "{} failed PRAGMA integrity_check",
            file.display()
        )));
    }
    let theirs = latest_migration(file).await?;
    let ours = Dialect::Sqlite
        .migrator()
        .migrations
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    if theirs > ours {
        return Err(BackupError::Refused(format!(
            "the backup was made by a newer silvermere (schema version {theirs}, this binary knows {ours}). Upgrade first."
        )));
    }
    ensure_not_in_use(live).await
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The asset files of a backup, copied beside the live directory and ready to swap in.
#[derive(Debug)]
pub struct StagedAssets {
    path: PathBuf,
    /// How many files.
    pub files: usize,
}

/// Copies the asset files of the backup `file` (its `.assets` directory) to a staging
/// directory beside `live` (`data/assets/`), before anything live is touched; `None` when
/// the backup has none.
pub fn stage_assets(file: &Path, live: &Path) -> Result<Option<StagedAssets>, BackupError> {
    let from = assets_dir(file);
    if !from.is_dir() {
        return Ok(None);
    }
    let path = sibling(live, ".restore-tmp");
    let _ = fs::remove_dir_all(&path);
    let files = copy_assets(&from, &path)?;
    Ok(Some(StagedAssets { path, files }))
}

impl StagedAssets {
    /// Puts the staged files in place of `live`, which moves to `<live>.pre-restore-<at>`
    /// (returned). When the second rename fails, the first is undone and `live` is as it was.
    pub fn swap_in(self, live: &Path, at: jiff::Timestamp) -> Result<Option<PathBuf>, BackupError> {
        let mut moved = None;
        if live.exists() {
            let ts = at.strftime("%Y%m%d-%H%M%S").to_string();
            let pre = free_name(sibling(live, &format!(".pre-restore-{ts}")));
            fs::rename(live, &pre)?;
            moved = Some(pre);
        }
        if let Err(e) = fs::rename(&self.path, live) {
            if let Some(pre) = &moved {
                let _ = fs::rename(pre, live);
            }
            return Err(e.into());
        }
        Ok(moved)
    }

    /// Removes the staged copy (the restore did not go ahead).
    pub fn discard(self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Puts the asset files of the backup `file` in place of `live`: [`stage_assets`] then
/// [`StagedAssets::swap_in`]. Returns where the old directory went and the number of files,
/// or `None` when the backup has no assets.
pub fn restore_assets(
    file: &Path,
    live: &Path,
    at: jiff::Timestamp,
) -> Result<Option<(Option<PathBuf>, usize)>, BackupError> {
    let Some(staged) = stage_assets(file, live)? else {
        return Ok(None);
    };
    let files = staged.files;
    Ok(Some((staged.swap_in(live, at)?, files)))
}

/// Undoes [`restore_sqlite`]: the restored database moves to
/// `<live>.failed-restore-<at>` (returned) and the previous one (`moved`) comes back.
pub fn undo_restore_sqlite(
    live: &Path,
    moved: Option<&Path>,
    at: jiff::Timestamp,
) -> Result<PathBuf, BackupError> {
    let ts = at.strftime("%Y%m%d-%H%M%S").to_string();
    let failed = free_name(sibling(live, &format!(".failed-restore-{ts}")));
    fs::rename(live, &failed)?;
    if let Some(old) = moved {
        // the restored file is aside; say where both are if the old one cannot come back
        fs::rename(old, live).map_err(|e| {
            BackupError::Refused(format!(
                "undoing the restore: {} could not be moved back to {} ({e}); \
                 the restored copy is {}. Move one of them to {} by hand",
                old.display(),
                live.display(),
                failed.display(),
                live.display()
            ))
        })?;
        for ext in ["-wal", "-shm"] {
            let w = sibling(old, ext);
            if w.exists() {
                fs::rename(&w, sibling(live, ext))?;
            }
        }
    }
    Ok(failed)
}

/// Puts `file` in place of the live SQLite database: the live file moves to
/// `<live>.pre-restore-<ts>`, `file` is copied to a temp name beside it and renamed into
/// place, stale `-wal` and `-shm` files go. Returns the path the old database moved to
/// (`None` when there was none). Run [`check_restorable`] first.
pub fn restore_sqlite(
    file: &Path,
    live: &Path,
    at: jiff::Timestamp,
) -> Result<Option<PathBuf>, BackupError> {
    let ts = at.strftime("%Y%m%d-%H%M%S").to_string();
    let mut moved = None;
    if live.exists() {
        let pre = free_name(sibling(live, &format!(".pre-restore-{ts}")));
        fs::rename(live, &pre)?;
        // A leftover WAL belongs to the old database: keep it beside it.
        for ext in ["-wal", "-shm"] {
            let w = sibling(live, ext);
            if w.exists() {
                fs::rename(&w, sibling(&pre, ext))?;
            }
        }
        moved = Some(pre);
    }
    let tmp = sibling(live, ".restore-tmp");
    fs::copy(file, &tmp)?;
    fs::rename(&tmp, live)?;
    for ext in ["-wal", "-shm"] {
        let _ = fs::remove_file(sibling(live, ext));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_use_utc_and_sort_by_time() {
        let t: jiff::Timestamp = "2026-10-15T01:02:03Z".parse().unwrap();
        assert_eq!(
            backup_name(t, Dialect::Sqlite),
            "silvermere-20261015-010203.db"
        );
        assert_eq!(
            backup_name(t, Dialect::Postgres),
            "silvermere-20261015-010203.dump"
        );
        assert!(is_backup_name("silvermere-20261015-010203.db"));
        assert!(is_backup_name("silvermere-20261015-010203.dump"));
        assert!(!is_backup_name("silvermere-20261015-010203.db.assets"));
        assert!(!is_backup_name("silvermere.db"));
        assert!(!is_backup_name("silvermere-20261015-010203.db.partial"));
        assert!(!is_backup_name("silvermere-2026101x-010203.db"));
    }

    /// T-108: the kind of a backup comes from its first bytes, whatever its name.
    #[test]
    fn formats_are_read_from_the_magic_bytes() {
        let dir = std::env::temp_dir().join(format!("sm-format-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sqlite = dir.join("named-like-a.dump");
        fs::write(&sqlite, b"SQLite format 3\0rest of the header").unwrap();
        let pg = dir.join("named-like-a.db");
        fs::write(&pg, b"PGDMP\x01\x0e\x00rest").unwrap();
        let other = dir.join("other.db");
        fs::write(&other, b"hello").unwrap();
        assert_eq!(format_of(&sqlite), Some(Format::Sqlite));
        assert_eq!(format_of(&pg), Some(Format::PgDump));
        assert_eq!(format_of(&other), None);
        assert_eq!(format_of(&dir.join("missing")), None);
        let empty = dir.join("empty.db");
        fs::write(&empty, b"").unwrap();
        assert_eq!(format_of(&empty), None);
        let short = dir.join("short.dump");
        fs::write(&short, b"PGD").unwrap();
        assert_eq!(format_of(&short), None);
        assert_eq!(format_of(&dir), None, "a directory is not a backup");
        assert_eq!(Format::PgDump.dialect(), Dialect::Postgres);
        let _ = fs::remove_dir_all(&dir);
    }

    /// T-108: assets go beside a backup and come back on restore, the old ones kept.
    #[test]
    fn assets_are_copied_and_restored() {
        let dir = std::env::temp_dir().join(format!("sm-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let live = dir.join("data/assets");
        fs::create_dir_all(live.join("sub")).unwrap();
        fs::write(live.join("WCC1393.STO"), b"room text").unwrap();
        fs::write(live.join("sub/map.txt"), b"map").unwrap();
        let backup = dir.join("silvermere-20260101-000000.db");
        fs::write(&backup, b"SQLite format 3\0").unwrap();
        assert_eq!(copy_assets(&live, &assets_dir(&backup)).unwrap(), 2);
        assert_eq!(
            fs::read(dir.join("silvermere-20260101-000000.db.assets/sub/map.txt")).unwrap(),
            b"map"
        );
        // the live files change, then the backup's come back
        fs::write(live.join("WCC1393.STO"), b"changed").unwrap();
        let t: jiff::Timestamp = "2026-10-17T23:00:00Z".parse().unwrap();
        let (moved, n) = restore_assets(&backup, &live, t).unwrap().unwrap();
        assert_eq!(n, 2);
        assert_eq!(fs::read(live.join("WCC1393.STO")).unwrap(), b"room text");
        let moved = moved.unwrap();
        assert_eq!(moved, dir.join("data/assets.pre-restore-20261017-230000"));
        assert_eq!(fs::read(moved.join("WCC1393.STO")).unwrap(), b"changed");
        // a second restore in the same second gets its own name
        let (moved2, _) = restore_assets(&backup, &live, t).unwrap().unwrap();
        assert_eq!(
            moved2.unwrap(),
            dir.join("data/assets.pre-restore-20261017-230000-2")
        );
        // a backup without assets leaves the live ones alone
        let bare = dir.join("silvermere-20260102-000000.db");
        fs::write(&bare, b"SQLite format 3\0").unwrap();
        assert!(restore_assets(&bare, &live, t).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// T-108: links are followed; a dangling one fails the copy and leaves nothing behind.
    #[cfg(unix)]
    #[test]
    fn asset_links_are_followed_and_a_dangling_one_fails_whole() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("sm-assetlinks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let real = dir.join("real");
        fs::create_dir_all(real.join("maps")).unwrap();
        fs::write(real.join("maps/m.txt"), b"map").unwrap();
        fs::write(real.join("room.txt"), b"room").unwrap();
        let live = dir.join("assets");
        fs::create_dir_all(&live).unwrap();
        symlink(real.join("room.txt"), live.join("room.txt")).unwrap();
        symlink(real.join("maps"), live.join("maps")).unwrap();
        let to = dir.join("b.db.assets");
        assert_eq!(copy_assets(&live, &to).unwrap(), 2);
        assert_eq!(fs::read(to.join("maps/m.txt")).unwrap(), b"map");
        assert!(
            !fs::symlink_metadata(to.join("room.txt"))
                .unwrap()
                .is_symlink()
        );

        symlink(dir.join("nowhere"), live.join("gone.txt")).unwrap();
        let to2 = dir.join("c.db.assets");
        let err = copy_assets(&live, &to2).unwrap_err().to_string();
        assert!(err.contains("gone.txt"), "{err}");
        assert!(
            !to2.exists() && !partial(&to2).exists(),
            "no half copy left"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// T-108: a failed swap puts the live assets back as they were.
    #[test]
    fn a_failed_swap_leaves_the_live_assets() {
        let dir = std::env::temp_dir().join(format!("sm-swapfail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let live = dir.join("data/assets");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("a.txt"), b"live").unwrap();
        let staged = StagedAssets {
            path: dir.join("data/no-such-staging"),
            files: 0,
        };
        let t: jiff::Timestamp = "2026-10-17T23:00:00Z".parse().unwrap();
        assert!(staged.swap_in(&live, t).is_err());
        assert_eq!(fs::read(live.join("a.txt")).unwrap(), b"live");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_the_newest_n_and_leaves_other_files() {
        let dir = std::env::temp_dir().join(format!("sm-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for n in ["20260101-000000", "20260102-000000"] {
            fs::write(dir.join(format!("silvermere-{n}.db")), b"x").unwrap();
        }
        // a PostgreSQL backup, newest, with its assets
        fs::write(dir.join("silvermere-20260103-000000.dump"), b"x").unwrap();
        fs::create_dir_all(dir.join("silvermere-20260101-000000.db.assets")).unwrap();
        fs::write(dir.join("silvermere-20260101-000000.db.assets/a.txt"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        // an orphaned .assets, and a directory with a backup's name: neither is a backup
        fs::create_dir_all(dir.join("silvermere-20250101-000000.dump.assets")).unwrap();
        fs::create_dir_all(dir.join("silvermere-20990101-000000.db")).unwrap();
        let removed = prune(&dir, 2).unwrap();
        assert_eq!(
            removed,
            [
                dir.join("silvermere-20260101-000000.db"),
                dir.join("silvermere-20250101-000000.dump.assets"),
            ]
        );
        assert!(!dir.join("silvermere-20260101-000000.db.assets").exists());
        assert!(dir.join("silvermere-20990101-000000.db").is_dir());
        assert!(dir.join("silvermere-20260102-000000.db").exists());
        assert!(dir.join("silvermere-20260103-000000.dump").exists());
        assert!(dir.join("notes.txt").exists());
        assert!(prune(&dir, 5).unwrap().is_empty());
        // a .dump goes with its .dump.assets
        fs::create_dir_all(dir.join("silvermere-20260103-000000.dump.assets")).unwrap();
        let removed = prune(&dir, 1).unwrap();
        assert_eq!(removed, [dir.join("silvermere-20260102-000000.db")]);
        let removed = prune(&dir, 0).unwrap();
        assert_eq!(removed, [dir.join("silvermere-20260103-000000.dump")]);
        assert!(!dir.join("silvermere-20260103-000000.dump.assets").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
