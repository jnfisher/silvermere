//! The world a server reads from a stock SQLite database, cached by the database's content.
//!
//! Most integration tests start a server on a copy of one of a few template databases (the
//! stock world, with Aldric, with a team). Reading the world out of the copy takes seconds
//! each time, the same seconds for every copy of the same template, and under `cargo nextest`
//! every test is a process of its own. [`snapshot`] reads it once per template content and
//! build, keeps it as JSON next to the templates, and hands every later start a copy decoded
//! from the file instead.
//!
//! Only the content of a file [`remember_template`] named is kept: a copy a test changed
//! before starting (a character edited, an account added) is read from the database as
//! before, so the cache never holds a world only one test would start.

use std::collections::HashMap;
use std::hash::Hasher as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use silvermere_engine::WorldSnapshot;
use silvermere_types::ids::AccountId;
use silvermere_types::records::{
    Action, ActiveMonster, BankBook, Character, Class, EvilRecord, Gang, Item, ItemOwnership,
    KnownMonster, Message, PermanentRecord, Race, Room, Shop, Spell, TextBlock,
};

use crate::shared_template::shared_source_file;

/// [`WorldSnapshot`] as stored. The conversions name every field, so a field added to the
/// snapshot fails to compile here rather than going missing from the cache.
#[derive(Serialize, Deserialize)]
struct Stored {
    rooms: Vec<Room>,
    items: Vec<Item>,
    monsters: Vec<KnownMonster>,
    active_monsters: Vec<ActiveMonster>,
    spells: Vec<Spell>,
    classes: Vec<Class>,
    races: Vec<Race>,
    shops: Vec<Shop>,
    messages: Vec<Message>,
    text_blocks: Vec<TextBlock>,
    actions: Vec<Action>,
    gangs: Vec<Gang>,
    item_ownership: Vec<ItemOwnership>,
    bank_books: Vec<(AccountId, BankBook)>,
    profiles: Vec<(AccountId, PermanentRecord)>,
    evil_points: Vec<(AccountId, EvilRecord)>,
    top_characters: Vec<Character>,
}

impl From<WorldSnapshot> for Stored {
    fn from(s: WorldSnapshot) -> Self {
        let WorldSnapshot {
            rooms,
            items,
            monsters,
            active_monsters,
            spells,
            classes,
            races,
            shops,
            messages,
            text_blocks,
            actions,
            gangs,
            item_ownership,
            bank_books,
            profiles,
            evil_points,
            top_characters,
        } = s;
        Self {
            rooms,
            items,
            monsters,
            active_monsters,
            spells,
            classes,
            races,
            shops,
            messages,
            text_blocks,
            actions,
            gangs,
            item_ownership,
            bank_books,
            profiles,
            evil_points,
            top_characters,
        }
    }
}

impl From<Stored> for WorldSnapshot {
    fn from(s: Stored) -> Self {
        let Stored {
            rooms,
            items,
            monsters,
            active_monsters,
            spells,
            classes,
            races,
            shops,
            messages,
            text_blocks,
            actions,
            gangs,
            item_ownership,
            bank_books,
            profiles,
            evil_points,
            top_characters,
        } = s;
        Self {
            rooms,
            items,
            monsters,
            active_monsters,
            spells,
            classes,
            races,
            shops,
            messages,
            text_blocks,
            actions,
            gangs,
            item_ownership,
            bank_books,
            profiles,
            evil_points,
            top_characters,
        }
    }
}

/// The content keys of the templates [`remember_template`] named, by template path.
static TEMPLATES: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);

/// A hash of the SQLite database at `path`: its file and its write-ahead log, if any.
/// `None` when the file cannot be read.
pub fn file_key(path: &Path) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(&std::fs::read(path).ok()?);
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    if let Ok(bytes) = std::fs::read(PathBuf::from(wal)) {
        h.write(&bytes);
    }
    Some(h.finish())
}

/// Marks the content of the template database at `template` as worth caching: tests start
/// servers on copies of it.
pub fn remember_template(template: &Path) {
    let mut t = TEMPLATES.lock().unwrap_or_else(|e| e.into_inner());
    let t = t.get_or_insert_with(HashMap::new);
    if !t.contains_key(template)
        && let Some(key) = file_key(template)
    {
        t.insert(template.to_owned(), key);
    }
}

fn is_template(key: u64) -> bool {
    TEMPLATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|t| t.values().any(|&k| k == key))
}

/// The world in the database at `url`, as `load` reads it. For a SQLite file whose content
/// is that of a template [`remember_template`] named, `load` runs once per content and per
/// fingerprint of `sources` (the code that reads it as well as the fixtures, as for
/// [`shared_source_file`]) across every test process, and every other call decodes the kept
/// copy from `base`. Any other database is read by `load` every time.
///
/// Blocking: call it from `spawn_blocking`.
pub fn snapshot(
    url: &str,
    base: &Path,
    sources: &[&Path],
    load: impl FnOnce() -> WorldSnapshot,
) -> WorldSnapshot {
    let Some(path) = url.strip_prefix("sqlite://") else {
        return load();
    };
    let path = Path::new(path.split('?').next().unwrap_or(path));
    let Some(key) = file_key(path).filter(|&k| is_template(k)) else {
        return load();
    };
    let mut loaded = None;
    let file = shared_source_file(
        base,
        "silvermere-world-",
        &format!("{key:016x}.json"),
        sources,
        |out| {
            let s = load();
            let f = std::fs::File::create(out).expect("create the kept world");
            let mut w = std::io::BufWriter::new(f);
            serde_json::to_writer(&mut w, &Stored::from(s.clone())).expect("write the kept world");
            std::io::Write::flush(&mut w).expect("write the kept world");
            loaded = Some(s);
        },
    );
    if let Some(s) = loaded {
        return s;
    }
    let bytes = std::fs::read(&file).expect("read the kept world");
    let stored: Stored = serde_json::from_slice(&bytes).expect("decode the kept world");
    stored.into()
}
