//! Helpers for the M3 integration tests (T-048): a database template holding three created
//! characters (Aldric, Brynja, Isolde), a named player on a telnet client, and the
//! classification of whatever a command printed.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use super::stock::{
    WAIT, add_account, aldric_db, login_to_menu, quit_to_menu, shared_template, start,
};
use silvermere_db::Db;
use silvermere_testkit::capture_compare::{events_of, masked_lines};
use silvermere_testkit::client::Wait;
use silvermere_testkit::{Event, MaskSpec, PatternSet, TelnetClient};

pub const PROMPT: &str = r"\[HP=-?\d+[^\]\n]*\]:";

/// A very short wait for "nothing arrives": only used to show that something did not happen
/// after a positive event already proved the server had caught up.
pub const QUIET: Duration = Duration::from_millis(400);

static TEAM: OnceLock<PathBuf> = OnceLock::new();

/// The stock world with the characters Aldric, Brynja and Isolde created (Human Warriors,
/// Strength and Health 50, as `creation-fsd`), built once per build for every test binary.
fn team_template() -> &'static Path {
    TEAM.get_or_init(|| {
        shared_template("silvermere-m3-", "team.db", |path| {
            // Aldric is the M1 template's; Brynja and Isolde are made the same way
            let (aldric_dir, aldric_url) = aldric_db();
            let from = aldric_url.trim_start_matches("sqlite://").to_owned();
            std::fs::copy(&from, path).unwrap();
            drop(aldric_dir);
            let url = format!("sqlite://{}", path.display());
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        for name in ["Brynja", "Isolde"] {
                            create_character(&url, name).await;
                        }
                    });
            })
            .join()
            .unwrap();
        })
    })
}

/// A copy of the stock world holding Aldric, Brynja and Isolde: `(its directory, url)`.
pub fn team_db() -> (Scratch, String) {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let t = team_template().to_owned();
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = t
        .parent()
        .unwrap()
        .join(format!("t{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");
    std::fs::copy(&t, &path).unwrap();
    silvermere_testkit::world_cache::remember_template(&t);
    (Scratch(dir), format!("sqlite://{}", path.display()))
}

/// A throwaway directory, removed when dropped.
pub struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The password of the account `name` (`aldric1`).
pub fn password(name: &str) -> String {
    format!("{}1", name.to_lowercase())
}

/// The line-mode creation of `name` (a Human Warrior; the account has the same name, which
/// the given-name prompt defaults to), as `create_aldric` does for Aldric.
pub async fn create_character(url: &str, name: &str) {
    add_account(url, name, &password(name)).await;
    let r = start(url).await;
    let mut c = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut c, name, &password(name)).await;
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
        ("", PROMPT),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    quit_to_menu(&mut c, &r.to_engine, name).await;
    c.disconnect().await;
    r.stop().await.unwrap();
}

/// Changes `name`'s stored record (while no server is running on the database).
pub async fn edit_char(
    url: &str,
    name: &str,
    edit: impl FnOnce(&mut silvermere_types::records::Character),
) {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let mut row = db
        .characters()
        .by_first_name(name)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{name} exists"));
    edit(&mut row.character);
    assert!(
        db.characters()
            .update(row.id, &row.character)
            .await
            .unwrap()
    );
    db.close().await;
}

/// Puts `name` in map 1 room `room` (before the server starts), as a character who
/// walked there and logged off: the newest trail entry is the room itself. (A character
/// just made has an empty trail, so a follower of his first step would not find the way:
/// a new character is put in the start room without `set_user_location`.)
pub async fn place(url: &str, name: &str, room: u32) {
    edit_char(url, name, |c| {
        c.map = silvermere_types::ids::MapId(1);
        c.room = silvermere_types::ids::RoomId(room);
        c.trailmaps[0] = silvermere_types::ids::MapId(1);
        c.trailrooms[0] = silvermere_types::ids::RoomId(room);
    })
    .await;
}

/// A logged-in character on its own telnet client.
pub struct Player {
    pub name: String,
    pub c: TelnetClient,
    to_engine: std::sync::mpsc::Sender<silvermere_engine::EngineInput>,
}

impl Player {
    /// Connects, logs in as `name` and enters the realm.
    pub async fn enter(r: &silvermere::Running, name: &str) -> Self {
        let mut c = TelnetClient::connect(r.telnet_addr)
            .await
            .unwrap()
            .with_naws(79, 24);
        login_to_menu(&mut c, name, &password(name)).await;
        c.send_line("E").await.unwrap();
        c.until(PROMPT, WAIT).await.unwrap();
        Self {
            name: name.to_owned(),
            c,
            to_engine: r.to_engine.clone(),
        }
    }

    /// Sends `line`, waits for its echo and the prompt behind the reply, and returns what
    /// arrived since, ANSI stripped. Waiting for the echo (not just a prompt) matters when
    /// messages for this player are still unread: each of them ends in a prompt too, and the
    /// reply to `line` comes after all of them, so the return is a barrier for them. A tick's
    /// redraw may sit between the typed text and its CR echo (T-094: the echo goes out when
    /// the engine starts the line, as a timer could run between the bytes on the original),
    /// and so may a message from another player ("/isolde ping 49Isolde telepaths: ping
    /// 48\n[HP=35]:\n--- Telepath Sent ..."; T-119). Each of those ends in a prompt and starts
    /// with a clear-line, never a line end, so the first line end straight after the typed
    /// text or after one of their prompts is the CR echo.
    pub async fn cmd(&mut self, line: &str) -> String {
        let at = self.c.position();
        self.c.send_line(line).await.unwrap();
        let until = format!(
            r"(?s)(?:^|\n|\]:){}(?:.*?{PROMPT}(?: \(Resting\))?)*?\r?\n.*?{PROMPT}",
            escape(line)
        );
        let got = self
            .c
            .wait_from(
                at,
                &Wait {
                    until: Some(until.clone()),
                    timeout: WAIT,
                    ..Wait::default()
                },
            )
            .await
            .unwrap();
        assert!(
            got.matched,
            "{}: no reply to {line:?}; got {:?}",
            self.name, got.plain
        );
        self.c.plain_from(at)
    }

    /// `x`, `X`: leaves the realm by the clean logoff (which saves the character) and hangs up.
    /// The meditation before the exit is stepped by the test ([`quit_to_menu`]).
    pub async fn leave(mut self) {
        quit_to_menu(&mut self.c, &self.to_engine, &self.name).await;
        self.c.disconnect().await;
    }

    /// Sends `line` without waiting.
    pub async fn say(&mut self, line: &str) {
        self.c.send_line(line).await.unwrap();
    }

    /// Waits until `pattern` shows in what arrived (the text since the last collection) and
    /// the prompt behind it; returns the plain text since `at`. Polls the socket, no sleeps.
    pub async fn wait_for(&mut self, at: usize, pattern: &str) -> String {
        self.c
            .until(&format!(r"(?s)(?:{pattern}).*?{PROMPT}"), WAIT)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", self.name));
        self.c.plain_from(at)
    }

    /// [`Player::leave`] on a server whose cycles are driven by hand: the silent meditation
    /// before the exit takes fast ticks.
    pub async fn leave_ticking(mut self, r: &silvermere::Running) {
        self.c.send_line("x").await.unwrap();
        self.c.until("silent meditation", WAIT).await.unwrap();
        tick(r, 30).await;
        self.c.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
        self.c.send_line("X").await.unwrap();
        self.c.until("Make your selection", WAIT).await.unwrap();
        self.c.disconnect().await;
    }

    /// Sends a movement command on a hand-driven server and ticks until `title` shows (the
    /// move is queued for the fast cycle); returns the plain text since the command.
    pub async fn walk_ticking(
        &mut self,
        r: &silvermere::Running,
        dir: &str,
        title: &str,
    ) -> String {
        let at = self.c.position();
        self.c.send_line(dir).await.unwrap();
        for _ in 0..200 {
            tick(r, 1).await;
            let got = self
                .c
                .wait_from(
                    at,
                    &Wait {
                        until: Some(format!("(?s){}.*?{PROMPT}", escape(title))),
                        timeout: Duration::from_millis(100),
                        ..Wait::default()
                    },
                )
                .await
                .unwrap();
            if got.matched {
                return self.c.plain_from(at);
            }
        }
        panic!("{}: never saw {title:?} after {dir:?}", self.name);
    }

    /// Like [`Player::wait_for`] for a message that is sent without a prompt behind it.
    pub async fn wait_text(&mut self, at: usize, pattern: &str) -> String {
        self.c
            .until(pattern, WAIT)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", self.name));
        self.c.plain_from(at)
    }

    /// Everything that has arrived so far, for "nothing else came" checks. Callers use it
    /// only after a positive wait on the same connection proved the server had caught up.
    pub async fn drain(&mut self) -> String {
        let at = self.c.position();
        let _ = self.c.read(QUIET).await;
        self.c.plain_from(at)
    }
}

/// The lines of `plain` (ANSI already stripped) without the prompts (a message that arrives
/// between commands is glued to the prompt before it: `[HP=35]:Bob says "hi"`).
pub fn body_lines(plain: &str) -> Vec<String> {
    let mut text = plain.replace("\r\n", "\n").replace('\r', "\n");
    while let Some(start) = text.find("[HP=") {
        let Some(len) = text[start..].find("]:") else {
            break;
        };
        text.replace_range(start..start + len + 2, "");
    }
    text.lines()
        .map(|l| l.trim_end().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Classifies every line of `text` (raw bytes, as the client received them) with the
/// registry: `(pattern name, line)`.
pub fn classify(raw: &[u8]) -> Vec<Event> {
    let patterns = PatternSet::load_default().expect("patterns.json");
    let fields = MaskSpec::load_default().expect("masks.json");
    let lines = masked_lines(raw, &[]);
    events_of(&lines, &patterns, &fields)
}

/// Panics when a line of `raw` is not classified by the registry; the echo of each command
/// in `sent` is the one thing a server line cannot be (the client's own typing).
pub fn assert_classified(what: &str, raw: &[u8], sent: &[&str]) {
    let unknown: Vec<_> = classify(raw)
        .into_iter()
        .filter(|e| e.name == "unknown" && !sent.contains(&e.line.trim()))
        .map(|e| e.line)
        .collect();
    assert!(unknown.is_empty(), "{what}: unclassified lines {unknown:?}");
}

/// The stored record of `name` (read while no server writes to the database).
pub async fn stored(url: &str, name: &str) -> silvermere_types::records::Character {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let row = db
        .characters()
        .by_first_name(name)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{name} is stored"));
    db.close().await;
    row.character
}

/// The balance of `user`'s book at bank `bank` (`None` before the first deposit).
pub async fn stored_balance(url: &str, user: &str, bank: i32) -> Option<u32> {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let acct = db
        .accounts()
        .by_username(user)
        .await
        .unwrap()
        .expect("account");
    let book = db
        .bank_books()
        .get(acct.id, silvermere_types::ids::ShopNo(bank))
        .await
        .unwrap();
    db.close().await;
    book.map(|b| b.balance)
}

/// How many copies of item `no` the stored record of `name` carries.
pub fn copies(c: &silvermere_types::records::Character, no: i32) -> usize {
    c.items.iter().filter(|i| i.0 == no).count()
}

/// Edits the in-game record of the logged-in character `name` on the world thread.
pub async fn edit_live(
    r: &silvermere::Running,
    name: &'static str,
    edit: impl FnOnce(&mut silvermere_types::records::Character) + Send + 'static,
) {
    super::stock::on_world(r, move |w| {
        for id in w.session_ids() {
            if let Some(p) = w.session_mut(id).and_then(|s| s.character.as_mut())
                && p.firstname == name
            {
                edit(p);
                return;
            }
        }
        panic!("{name} is not in the game");
    })
    .await;
}

/// [`start`] with the real-time fast ticker switched off (`SECFAST` of a day): the cycles
/// run only when [`tick`] sends them, so a test decides exactly where a medium cycle falls.
pub async fn start_manual(url: &str) -> silvermere::Running {
    super::stock::start_with(url, |cfg| cfg.game.sec_fast = 86_400).await
}

/// Sends `n` fast ticks to the engine and waits until it has handled them.
pub async fn tick(r: &silvermere::Running, n: u32) {
    for _ in 0..n {
        r.to_engine
            .send(silvermere_engine::EngineInput::Tick)
            .expect("engine is running");
    }
    super::stock::on_world(r, |_| ()).await;
}

/// `text` as a regex that matches it literally.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if r"\.+*?()|[]{}^$#&-~".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The sum of the stored quantities on shop `shop`'s shelves.
pub async fn stored_stock_total(url: &str, shop: i32) -> u32 {
    let db = Db::connect_and_migrate(url).await.unwrap();
    let stock = db
        .shop_stock()
        .get(silvermere_types::ids::ShopNo(shop))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("shop {shop} exists"));
    db.close().await;
    stock.qty.iter().map(|&q| u32::from(q)).sum()
}
