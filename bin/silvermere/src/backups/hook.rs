//! The after-backup command (`backup.after`, T-150): run after each successful backup,
//! scheduled or manual, to copy the file somewhere else (rclone, restic, rsync...).
//!
//! It runs without a shell: the setting is split into words ([`split`]), `{file}` and
//! `{dir}` in each word become the backup's path and the backups folder, and the first word
//! is the program. A command that needs a shell says so itself (`sh -c '...'`). It runs with
//! the server's environment less every variable that carries the database URL, its password
//! or an admin token, so it is not given them (as the server's user it can still read what
//! the server can); `SILVERMERE_BACKUP_FILE` and
//! `SILVERMERE_BACKUP_DIR` carry the two paths. Its output is kept to the last
//! [`TAIL_LINES`] lines of at most [`LINE_BYTES`] each, and it is killed (with anything it started) after
//! `backup.after_timeout_secs` (on Windows only the command itself is killed, not what it
//! started). Its failure is reported, never undone: the local backup stays.

use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The lines of the command's output kept for the log and the status.
pub const TAIL_LINES: usize = 20;

/// The bytes kept of one line; the rest of a longer line is dropped, marked with `…`.
pub const LINE_BYTES: usize = 1024;

/// Splits output into lines as it comes, at `\n` or `\r` (a progress meter redraws its line
/// with `\r`), keeping at most [`LINE_BYTES`] of each, so no output grows memory without
/// limit.
#[derive(Default)]
struct Lines {
    line: Vec<u8>,
    cut: bool,
}

impl Lines {
    /// The lines `bytes` completes.
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for &b in bytes {
            if b == b'\n' || b == b'\r' {
                done.extend(self.end());
            } else if self.line.len() < LINE_BYTES {
                self.line.push(b);
            } else {
                self.cut = true;
            }
        }
        done
    }

    /// The line left at the end of the output, if any.
    fn finish(&mut self) -> Vec<String> {
        self.end().into_iter().collect()
    }

    fn end(&mut self) -> Option<String> {
        let mut text = String::from_utf8_lossy(&self.line).trim_end().to_owned();
        if std::mem::take(&mut self.cut) {
            text.push('…');
        }
        self.line.clear();
        // `\r\n` and a redraw to an empty line end nothing
        (!text.is_empty()).then_some(text)
    }
}

/// Appends `lines` to the tail, keeping the last [`TAIL_LINES`].
fn push(tail: &Mutex<VecDeque<String>>, lines: Vec<String>) {
    if let Ok(mut t) = tail.lock() {
        t.extend(lines);
        while t.len() > TAIL_LINES {
            t.pop_front();
        }
    }
}

/// Variables never passed to the command: they carry the database URL, its password or an
/// admin token.
const SCRUBBED: [&str; 6] = [
    "SILVERMERE_DATABASE_URL",
    "SILVERMERE_DATABASE__URL",
    "SILVERMERE_ADMIN_TOKEN",
    "SILVERMERE_ADMIN_TOKENS",
    "PGPASSWORD",
    "DATABASE_URL",
];

/// A secret shorter than this is not looked for in the variables' values: a short password
/// would match ordinary values (`PATH`, say) and strip them. The named variables go anyway.
const MIN_SECRET: usize = 6;

/// How the command ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutcome {
    /// It ran and exited 0.
    pub ok: bool,
    /// Its exit code, when it exited (not when it was killed or did not start).
    pub exit: Option<i32>,
    /// It ran past the timeout and was killed.
    pub timed_out: bool,
    /// Why it did not start (the program not found, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The last lines of its output, stdout and stderr together.
    pub tail: Vec<String>,
}

impl HookOutcome {
    /// One line for the console and the log: `exit 0`, `exit 3`, `killed after 30 s`...
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            format!("did not start: {e}")
        } else if self.timed_out {
            "killed at the timeout (backup.after_timeout_secs)".to_owned()
        } else {
            match self.exit {
                Some(c) => format!("exit {c}"),
                None => "ended by a signal".to_owned(),
            }
        }
    }
}

/// The words of `cmd`: split on whitespace; `'...'` is taken as it is, `"..."` keeps its
/// spaces with `\"` and `\\` inside, and a backslash outside quotes escapes a space, a quote
/// or a backslash (any other backslash is kept, so a Windows path reads as written).
pub fn split(cmd: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("a ' quote is not closed".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                            word.extend(chars.next());
                        }
                        Some(c) => word.push(c),
                        None => return Err("a \" quote is not closed".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.peek() {
                    Some(&n) if n.is_whitespace() || matches!(n, '\'' | '"' | '\\') => {
                        word.extend(chars.next());
                    }
                    _ => word.push('\\'),
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// The command's words with `{file}` and `{dir}` filled in; `None` when `cmd` is empty.
pub fn argv(cmd: &str, file: &Path, dir: &Path) -> Result<Option<Vec<String>>, String> {
    let file = file.to_string_lossy();
    let dir = dir.to_string_lossy();
    let words = split(cmd)?;
    if words.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        words
            .into_iter()
            .map(|w| w.replace("{file}", &file).replace("{dir}", &dir))
            .collect(),
    ))
}

/// Runs `cmd` for the backup `file` in `dir` and waits for it, up to `timeout`. `secrets`
/// (the database URL and its password) are values no variable passed to it may contain
/// (looked for when at least [`MIN_SECRET`] long).
/// `None` when `cmd` is empty.
pub fn run(
    cmd: &str,
    file: &Path,
    dir: &Path,
    timeout: Duration,
    secrets: &[String],
) -> Option<HookOutcome> {
    let failed = |e: String| HookOutcome {
        ok: false,
        exit: None,
        timed_out: false,
        error: Some(e),
        tail: Vec::new(),
    };
    let words = match argv(cmd, file, dir) {
        Ok(Some(w)) => w,
        Ok(None) => return None,
        Err(e) => return Some(failed(format!("backup.after: {e}"))),
    };
    let mut c = Command::new(&words[0]);
    c.args(&words[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("SILVERMERE_BACKUP_FILE", file)
        .env("SILVERMERE_BACKUP_DIR", dir);
    for (k, v) in std::env::vars_os() {
        let v = v.to_string_lossy();
        let named = k
            .to_str()
            .is_some_and(|k| SCRUBBED.contains(&k) || k.starts_with("SILVERMERE_ADMIN"));
        if named
            || secrets
                .iter()
                .any(|s| s.len() >= MIN_SECRET && v.contains(s.as_str()))
        {
            c.env_remove(&k);
        }
    }
    // its own process group, so the timeout kills what it started too
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    let mut child = match c.spawn() {
        Ok(child) => child,
        Err(e) => return Some(failed(format!("{}: {e}", words[0]))),
    };
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let (done_tx, done_rx) = mpsc::channel();
    let mut readers = 0;
    let out: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|s| Box::new(s) as _),
        child.stderr.take().map(|s| Box::new(s) as _),
    ];
    for stream in out.into_iter().flatten() {
        readers += 1;
        let tail = tail.clone();
        let done = done_tx.clone();
        std::thread::spawn(move || {
            let mut lines = Lines::default();
            let mut chunk = [0u8; 8192];
            let mut r = stream;
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let done = lines.feed(&chunk[..n]);
                push(&tail, done);
            }
            push(&tail, lines.finish());
            let _ = done.send(());
        });
    }
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                kill(&mut child);
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                kill(&mut child);
                let _ = child.wait();
                return Some(failed(format!("waiting for {}: {e}", words[0])));
            }
        }
    };
    // the output that is still on its way (a process it left behind may hold the pipe: then
    // what came so far)
    let grace = Instant::now() + Duration::from_secs(2);
    for _ in 0..readers {
        let left = grace.saturating_duration_since(Instant::now());
        if done_rx.recv_timeout(left).is_err() {
            break;
        }
    }
    let tail: Vec<String> = tail
        .lock()
        .map(|t| t.iter().cloned().collect())
        .unwrap_or_default();
    let exit = if timed_out {
        None
    } else {
        status.and_then(|s| s.code())
    };
    Some(HookOutcome {
        ok: !timed_out && exit == Some(0),
        exit,
        timed_out,
        error: None,
        tail,
    })
}

/// Kills the command and its process group.
fn kill(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_quotes() {
        let w = |s: &str| split(s).unwrap();
        assert_eq!(
            w("rclone copy {file} remote:silvermere/"),
            ["rclone", "copy", "{file}", "remote:silvermere/"]
        );
        assert_eq!(
            w(r#"sh -c 'restic backup "$1"' sh {file}"#),
            ["sh", "-c", r#"restic backup "$1""#, "sh", "{file}"]
        );
        assert_eq!(w(r#"a "b c" d\ e "f\"g""#), ["a", "b c", "d e", "f\"g"]);
        assert_eq!(
            w(r"C:\rclone\rclone.exe copy {file} b2:x"),
            [r"C:\rclone\rclone.exe", "copy", "{file}", "b2:x"]
        );
        assert_eq!(w("  ''  x"), ["", "x"]);
        assert!(w("   ").is_empty());
        assert!(split("echo 'oops").is_err());
        assert!(split("echo \"oops").is_err());
    }

    #[test]
    fn fills_in_the_file_and_the_folder_as_one_word_each() {
        let a = argv(
            "cp {file} {dir}/../copy-of-{file}",
            Path::new("/b k/x.db"),
            Path::new("/b k"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a, ["cp", "/b k/x.db", "/b k/../copy-of-/b k/x.db"]);
        assert_eq!(argv("", Path::new("f"), Path::new("d")).unwrap(), None);
    }

    #[cfg(unix)]
    fn sh(script: &str, timeout: Duration, secrets: &[String]) -> HookOutcome {
        let cmd = format!("sh -c '{script}' sh {{file}} {{dir}}");
        run(
            &cmd,
            Path::new("/b/x.db"),
            Path::new("/b"),
            timeout,
            secrets,
        )
        .unwrap()
    }

    /// The command gets the backup's path and folder, as words and as variables, and its
    /// exit code and last lines come back.
    #[cfg(unix)]
    #[test]
    fn runs_with_the_paths_and_reports_its_exit_and_output() {
        let out = sh(
            r#"echo "$1 $2"; echo "$SILVERMERE_BACKUP_FILE $SILVERMERE_BACKUP_DIR" >&2"#,
            Duration::from_secs(10),
            &[],
        );
        assert!(out.ok && out.exit == Some(0) && !out.timed_out, "{out:?}");
        let mut tail = out.tail.clone();
        tail.sort();
        assert_eq!(tail, ["/b/x.db /b", "/b/x.db /b"]);

        let out = sh(
            "for i in $(seq 1 50); do echo line$i; done; exit 3",
            Duration::from_secs(10),
            &[],
        );
        assert!(!out.ok && out.exit == Some(3), "{out:?}");
        assert_eq!(out.summary(), "exit 3");
        assert_eq!(out.tail.len(), TAIL_LINES);
        assert_eq!(out.tail.last().map(String::as_str), Some("line50"));
    }

    /// Past the timeout it is killed, with what it started.
    #[cfg(unix)]
    #[test]
    fn the_timeout_kills_it_and_what_it_started() {
        let started = Instant::now();
        let out = sh(
            "sleep 30 & echo begun; sleep 30",
            Duration::from_millis(500),
            &[],
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert!(out.timed_out && !out.ok && out.exit.is_none(), "{out:?}");
        assert_eq!(out.tail, ["begun"]);
    }

    /// No variable carrying the database URL, its password or a token reaches it, by name or
    /// by value.
    #[cfg(unix)]
    #[test]
    fn it_never_sees_the_database_url_or_password() {
        // a value only one variable holds stands for the password (setting a variable here
        // would race the other tests)
        let vars: Vec<(String, String)> = std::env::vars().collect();
        let Some((secret_var, secret)) = vars.iter().find(|(k, v)| {
            v.len() >= 12
                && !v.contains(['"', '\'', '\\', '$', '`'])
                && vars
                    .iter()
                    .all(|(k2, v2)| k2 == k || !v2.contains(v.as_str()))
        }) else {
            return;
        };
        // the names that matter on one line, then the count of lines holding the secret
        let script = format!(
            r#"env | cut -d= -f1 | grep -E "^(PATH|{secret_var}|SILVERMERE_.*|PGPASSWORD|DATABASE_URL)$" | tr "\n" " "; echo; env | grep -cF -- "{secret}" || true"#
        );
        let out = sh(
            &script,
            Duration::from_secs(10),
            std::slice::from_ref(secret),
        );
        assert!(out.ok, "{out:?}");
        let [names, count] = &out.tail[..] else {
            panic!("{out:?}");
        };
        assert!(
            names.contains("PATH") && names.contains("SILVERMERE_BACKUP_FILE"),
            "{names}"
        );
        for name in names.split_whitespace() {
            assert!(
                name != secret_var
                    && !SCRUBBED.contains(&name)
                    && !name.starts_with("SILVERMERE_ADMIN"),
                "{name}"
            );
        }
        assert_eq!(count, "0", "a variable holding the secret was passed");
    }

    #[test]
    fn a_missing_program_is_reported() {
        let out = run(
            "silvermere-no-such-program {file}",
            Path::new("x.db"),
            Path::new("."),
            Duration::from_secs(5),
            &[],
        )
        .unwrap();
        assert!(!out.ok && out.error.is_some(), "{out:?}");
        assert!(out.summary().starts_with("did not start"), "{out:?}");
        assert!(
            run(
                "  ",
                Path::new("x"),
                Path::new("."),
                Duration::from_secs(1),
                &[]
            )
            .is_none()
        );
    }

    /// A line is cut at [`LINE_BYTES`], `\r` ends a line, and only the last lines are kept,
    /// however much comes.
    #[test]
    fn output_is_kept_in_bounded_lines() {
        let mut l = Lines::default();
        let mut got = l.feed(b"one\r\ntwo\r 10%\r 55%\r100%\n");
        assert_eq!(got, ["one", "two", " 10%", " 55%", "100%"]);
        let long = vec![b'x'; 10 * LINE_BYTES];
        got = l.feed(&long);
        assert!(got.is_empty());
        got = l.feed(b"\nend");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].len(), LINE_BYTES + '…'.len_utf8());
        assert!(got[0].ends_with('…'));
        assert_eq!(l.finish(), ["end"]);
        assert!(l.finish().is_empty());

        let tail = Mutex::new(VecDeque::new());
        for i in 0..1000 {
            push(&tail, vec![format!("{i}")]);
        }
        let t = tail.lock().unwrap();
        assert_eq!(t.len(), TAIL_LINES);
        assert_eq!(t.back().map(String::as_str), Some("999"));
    }

    /// A progress meter's megabytes of `\r` redraws with no newline come back as a few
    /// short lines.
    #[cfg(unix)]
    #[test]
    fn a_progress_meter_does_not_grow_the_tail() {
        let out = sh(
            "i=0; while [ $i -lt 20000 ]; do printf \"\\r%s%% %s\" $i xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done",
            Duration::from_secs(60),
            &[],
        );
        assert!(out.ok, "{out:?}");
        assert_eq!(out.tail.len(), TAIL_LINES);
        assert!(
            out.tail.iter().all(|l| l.len() <= LINE_BYTES + 3),
            "{out:?}"
        );
        assert!(out.tail.last().unwrap().starts_with("19999% "), "{out:?}");
    }

    /// A short password is not looked for in values, so `PATH` and the like stay.
    #[cfg(unix)]
    #[test]
    fn a_short_secret_does_not_strip_ordinary_variables() {
        let out = sh(
            r#"env | grep -c "^PATH=.*bin""#,
            Duration::from_secs(10),
            &["bin".to_owned()],
        );
        assert!(out.ok && out.tail == ["1"], "{out:?}");
    }
}
