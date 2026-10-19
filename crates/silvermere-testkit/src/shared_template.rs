//! A database file built once and shared by every test process that needs it.
//!
//! The integration suites import the stock world (or add characters to it) once per test
//! binary and copy the file per test. Under `cargo test` a binary is one process, so that is
//! one import; under `cargo nextest` every test is its own process, so each would repeat the
//! import. [`shared_file`] builds the file once per build of the test binary instead: the first
//! process takes a lock file and builds, the others wait for the result.
//!
//! The directory name carries a fingerprint of the test executable and the fixture files, so a
//! rebuilt binary or an edited fixture never reuses an older file. [`shared_source_file`]
//! leaves the executable out and fingerprints only the sources it is given, so every test binary
//! of a build shares one file: one build per run instead of one per binary. Directories of other
//! fingerprints under the same prefix are removed when they are old enough that no test of a
//! finished run can still be using them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long a process waits for another's build before it gives up.
const WAIT: Duration = Duration::from_secs(600);
/// Directories of older fingerprints this old are removed.
const STALE: Duration = Duration::from_secs(3600);

/// Whether process `pid` still runs. Without `/proc` nothing counts as gone.
pub fn process_alive(pid: u32) -> bool {
    pid == std::process::id()
        || !Path::new("/proc").is_dir()
        || Path::new("/proc").join(pid.to_string()).exists()
}

fn mtime_ns(m: &fs::Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Folds `path`'s size and modification time (all files below it, for a directory) into `h`.
fn fold(h: &mut u64, path: &Path) {
    let Ok(m) = fs::metadata(path) else { return };
    let mut mix = |v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(m.len());
    mix(mtime_ns(&m) as u64);
    if m.is_dir() {
        let mut names: Vec<_> = fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        names.sort();
        for p in names {
            fold(h, &p);
        }
    }
}

/// `dir_prefix<fingerprint>/file`, built by `build` (given the path to write) the first time
/// any process asks for it, then reused. `inputs` are the files or directories the content
/// depends on besides the test executable.
pub fn shared_file(
    base: &Path,
    dir_prefix: &str,
    file: &str,
    inputs: &[&Path],
    build: impl FnOnce(&Path),
) -> PathBuf {
    shared(base, dir_prefix, file, inputs, true, build)
}

/// [`shared_file`] shared by every test binary: the fingerprint is `inputs` alone, not the test
/// executable, so `inputs` must hold every source the content depends on (the code that builds
/// it as well as the fixtures).
pub fn shared_source_file(
    base: &Path,
    dir_prefix: &str,
    file: &str,
    inputs: &[&Path],
    build: impl FnOnce(&Path),
) -> PathBuf {
    shared(base, dir_prefix, file, inputs, false, build)
}

fn shared(
    base: &Path,
    dir_prefix: &str,
    file: &str,
    inputs: &[&Path],
    with_exe: bool,
    build: impl FnOnce(&Path),
) -> PathBuf {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    if with_exe && let Ok(exe) = std::env::current_exe() {
        fold(&mut h, &exe);
    }
    for i in inputs {
        fold(&mut h, i);
    }
    let name = format!("{dir_prefix}{h:016x}");
    remove_stale(base, dir_prefix, &name);
    let dir = base.join(&name);
    fs::create_dir_all(&dir).expect("shared template dir");
    let done = dir.join(file);
    let lock = dir.join(format!("{file}.lock"));
    let started = std::time::Instant::now();
    loop {
        if done.exists() {
            return done;
        }
        // The lock appears with its owner's pid already in it: the pid goes to a private file,
        // which is hard-linked to the lock name (fails if the lock exists, like `create_new`), so
        // no process can read an empty lock and take over a live builder's.
        let mine = dir.join(format!("{file}.pid{}", std::process::id()));
        fs::write(&mine, std::process::id().to_string()).expect("write lock owner");
        let linked = fs::hard_link(&mine, &lock);
        let _ = fs::remove_file(&mine);
        match linked {
            Ok(()) => {
                if !done.exists() {
                    let tmp = dir.join(format!("{file}.tmp{}", std::process::id()));
                    build(&tmp);
                    fs::rename(&tmp, &done).expect("publish shared template");
                }
                let _ = fs::remove_file(&lock);
                return done;
            }
            Err(_) => {
                // a builder that died leaves its lock: take it over
                let owner = fs::read_to_string(&lock)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                if owner.is_some_and(|p| !process_alive(p)) {
                    let _ = fs::remove_file(&lock);
                }
                assert!(
                    started.elapsed() < WAIT,
                    "waited {WAIT:?} for another test process to build {}",
                    done.display()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn remove_stale(base: &Path, dir_prefix: &str, keep: &str) {
    let Ok(rd) = fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n == keep || !n.starts_with(dir_prefix) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}
