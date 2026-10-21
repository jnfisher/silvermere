//! The `.MSG` texts as the package text reads them (T-011, T-138): the text blocks of
//! `WCCMMUD.MSG` and `WCCTEXT.MSG`, the help topics, the two character-creation forms, and
//! `docs/config-defaults.md`. The text comes from the stock package (`crate::package`).
//!
//! The texts are rendered here exactly as the game printed them and compared
//! with the bytes of the T-005 capture `entry-existing` (the same constants as `menu.rs`)
//! and with the pattern registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;
use silvermere_text::catalogue::{Catalogue, Entry, HELP_END_MARK, HelpTopics, TemplatePiece};
use silvermere_text::menu::{MUDVER, SHORTVER};
use silvermere_text::prompt::prompt_text;
use silvermere_text::strip::strip_ansi;
use silvermere_text::{cp437, finish};
use silvermere_types::config::GameConfig;

/// `entry-existing/raw.bin`: the menu through the prompt (also in `menu.rs`).
const CAPTURED_MENU: &[u8] = b"\x1b[0m\x1b[2J\r\n \x1b[1mM A J O R  M U D \x1b[33mv1.11p-WG (Dec 30 2005 14:20:05)\r\n\x1b[0m{Realm Of Legends}\r\n\x1b[1;35m*ANSI RECOMMENDED*\r\n\r\n\x1b[30m[\x1b[37mE\x1b[30m] . \x1b[0mEnter the Realm\r\n\x1b[1;30m[\x1b[37mH\x1b[30m] . \x1b[0mHelp\r\n\x1b[1;30m[\x1b[37mA\x1b[30m] . \x1b[0mAbout the Game\r\n\x1b[1;30m[\x1b[37mC\x1b[30m] . \x1b[0mGame Counts\r\n\x1b[1;30m[\x1b[37mT\x1b[30m] . \x1b[0mTopten Adventurers\r\n\x1b[1;30m[\x1b[37mG\x1b[30m] . \x1b[0mTopten Gangs\r\n\x1b[1;30m[\x1b[37mW\x1b[30m] . \x1b[0mWho's in the Realm\r\n\x1b[1;30m[\x1b[37mR\x1b[30m] . \x1b[0mRelease Notes\r\n\x1b[1;30m[\x1b[37mI\x1b[30m] . \x1b[0mInformation\r\n\x1b[1;30m[\x1b[37mX\x1b[30m] . \x1b[0mExit Game\r\n\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: ";

/// `entry-existing/raw.bin` after the player typed `E`: `USRFOLED`.
const CAPTURED_BANNER: &[u8] = b"\x1b[0m\x1b[2J\r\n\x1b[1;33mMajorMUD - ROL\r\n\r\n\x1b[0m\x1b[1;30m=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=\r\n\r\n";

/// `entry-existing/raw.bin` after the password: `USRLOGIN`.
const CAPTURED_GREETING: &[u8] =
    b"\r\n\x1b[1mFantasy awaits you in \x1b[33mMajorMUD v1.11p-WG!\r\n\r\n";

// SHA-256 of the generated document, so a hand edit is noticed. Regenerate it with
// tools/msgconv/README.md and update this.
const SHA_CONFIG_DEFAULTS: &str =
    "2681048e1ba467f6a1d831a603ec1aea2f2fab59c150cc8e4a25a206e94c1e01";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_dir() -> PathBuf {
    crate_dir().join("../..")
}

// ---- rendering a converted entry the way the game printed it --------------------------

/// The bytes the player receives for a catalogue entry: arguments substituted, then the
/// host's if-ANSI, line-end and CP437 steps (`render::finish`).
fn render(entry: &Entry, key: &str, args: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::new();
    for piece in entry.pieces(key).expect("template parses") {
        match piece {
            TemplatePiece::Text(t) => text.push_str(&t),
            TemplatePiece::Arg(name) => {
                let value = args
                    .iter()
                    .find(|(n, _)| *n == name)
                    .unwrap_or_else(|| panic!("{key}: no value for {name}"))
                    .1;
                text.push_str(value);
            }
            other => panic!("{key}: unexpected {other:?}"),
        }
    }
    finish(&cp437::encode(&text), None)
}

fn lines(bytes: &[u8]) -> Vec<String> {
    cp437::decode(&strip_ansi(bytes))
        .split("\r\n")
        .map(|l| l.trim_end().to_string())
        .collect()
}

#[derive(Deserialize)]
struct Registry {
    #[serde(rename = "static")]
    patterns: Vec<Pattern>,
}

#[derive(Deserialize)]
struct Pattern {
    name: String,
    rx: String,
}

fn pattern(name: &str) -> Regex {
    let registry: Registry =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json"))
            .expect("patterns.json parses");
    let p = registry
        .patterns
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no pattern {name}"));
    Regex::new(&format!("^(?:{})$", p.rx)).expect("pattern compiles")
}

fn wccmmud() -> Catalogue {
    crate::package().wccmmud().clone()
}

fn wcctext() -> Catalogue {
    crate::package().wcctext().clone()
}

// ---- the text blocks ----------------------------------------------------------------

// USRLOGIN, USRLOGOF, USRFOLED, USREXIT, LEAVEMNU, INACHUP
#[test]
fn wccmmud_has_the_six_text_blocks() {
    let cat = wccmmud();
    let keys: Vec<&str> = cat.messages.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "inachup", "leavemnu", "usrexit", "usrfoled", "usrlogin", "usrlogof"
        ]
    );
}

// eight text blocks; EDITCHA1 and EDTCHAN2 are FSD templates (assets), so six
#[test]
fn wcctext_has_the_six_text_blocks() {
    let cat = wcctext();
    let keys: Vec<&str> = cat.messages.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "chsclass", "chsrace", "goodpmpt", "goodtext", "menupmpt", "menutext"
        ]
    );
}

// The .MSG compiler keeps every LF (as `\r`) and drops CR: no entry holds a bare LF or CR LF.
// docs/text.md records the rule; the captures below are what settles it.
#[test]
fn texts_use_cr_line_ends_only() {
    for cat in [wccmmud(), wcctext()] {
        for (key, entry) in &cat.messages {
            assert!(!entry.text.contains('\n'), "{key} holds an LF");
            assert!(entry.text.is_ascii(), "{key} holds a non-ASCII character");
        }
    }
}

// USRLOGIN with the bare version, after the password
#[test]
fn usrlogin_equals_the_capture() {
    let cat = wccmmud();
    let entry = &cat.messages["usrlogin"];
    let bytes = render(entry, "usrlogin", &[("version", SHORTVER)]);
    assert_eq!(bytes, CAPTURED_GREETING);
    let plain = lines(&bytes);
    assert!(pattern("login_greeting").is_match(&plain[1]), "{plain:?}");
    // the example in the catalogue is the same version string
    assert_eq!(entry.example["version"], SHORTVER);
    // the hand-entered text of prompt.toml agrees (until the prompt reads these files)
    assert_eq!(entry.text, prompt_text("login_greeting"));
}

// USRFOLED, on entering the realm
#[test]
fn usrfoled_equals_the_capture() {
    let cat = wccmmud();
    let entry = &cat.messages["usrfoled"];
    let bytes = render(entry, "usrfoled", &[]);
    assert_eq!(bytes, CAPTURED_BANNER);
    let plain = lines(&bytes);
    assert!(pattern("enter_banner").is_match(&plain[1]));
    assert!(pattern("enter_rule").is_match(&plain[3]));
    // the rule is 77 characters wide: 38 `=-` pairs and a closing `=`
    assert_eq!(plain[3].len(), 77);
    assert_eq!(entry.text, prompt_text("entry_banner"));
}

// INACHUP, the inactivity hang-up: three spaces before Bye!
#[test]
fn inachup_matches_the_registry() {
    let cat = wccmmud();
    let bytes = render(&cat.messages["inachup"], "inachup", &[]);
    let plain = lines(&bytes);
    assert_eq!(plain[1], "You have been inactive for too long.   Bye!");
    assert!(pattern("inactive_hangup").is_match(&plain[1]));
    assert_eq!(
        bytes,
        b"\x1b[0m\r\nYou have been inactive for too long.   Bye!\r\n\r\n"
    );
}

// USREXIT, on leaving the game
#[test]
fn usrexit_text() {
    let cat = wccmmud();
    let bytes = render(&cat.messages["usrexit"], "usrexit", &[]);
    assert_eq!(
        bytes,
        b"\x1b[0m\r\nYour character has been saved. If you have any comments or suggestions, please\r\nleave them in E-mail to Sysop. Thanks.\r\n".as_slice()
    );
}

// LEAVEMNU: a space and a line end; USRLOGOF is ESC[0m and a line end
#[test]
fn unused_hooks_keep_their_text() {
    let cat = wccmmud();
    assert_eq!(cat.messages["leavemnu"].text, " \r");
    assert_eq!(cat.messages["usrlogof"].text, "\u{1b}[0m\r");
}

// MENUTEXT with the full version string, then MENUPMPT
#[test]
fn menu_texts_equal_the_capture() {
    let cat = wcctext();
    let mut bytes = render(
        &cat.messages["menutext"],
        "menutext",
        &[("version", MUDVER)],
    );
    bytes.extend(render(&cat.messages["menupmpt"], "menupmpt", &[]));
    assert_eq!(bytes, CAPTURED_MENU);

    let plain = lines(&bytes);
    assert!(pattern("mud_title").is_match(plain[1].trim_start()));
    assert!(pattern("mud_world").is_match(&plain[2]));
    assert!(pattern("mud_ansi_note").is_match(&plain[3]));
    let items = pattern("mud_menu_item");
    for line in &plain[5..15] {
        assert!(items.is_match(line), "{line}");
    }
    assert!(pattern("mud_menu_prompt").is_match(&plain[16]));

    // the placeholder is the version only; `{Realm Of Legends}` is literal text
    assert_eq!(
        cat.messages["menutext"].text,
        prompt_text("menu_text").replace("{Realm Of Legends}", "{{Realm Of Legends}}")
    );
    assert_eq!(cat.messages["menupmpt"].text, prompt_text("menu_prompt"));
}

// CHSRACE, CHSCLASS, GOODTEXT and GOODPMPT, the character-creation texts
#[test]
fn creation_prompts() {
    let cat = wcctext();
    assert_eq!(
        render(&cat.messages["chsrace"], "chsrace", &[]),
        b"\x1b[0m\r\n\x1b[32mPlease choose your race [ ? for help ] : ".as_slice()
    );
    assert_eq!(
        render(&cat.messages["chsclass"], "chsclass", &[]),
        b"\x1b[0m\r\n\x1b[32mPlease choose your class [ ? for help ] : ".as_slice()
    );
    assert_eq!(
        render(&cat.messages["goodpmpt"], "goodpmpt", &[]),
        b"\x1b[0m\r\nDo you want to be \x1b[1mLawful\x1b[0m?  [Yes/No] ".as_slice()
    );
    let good = render(&cat.messages["goodtext"], "goodtext", &[]);
    assert!(good.starts_with(b"\x1b[0m\r\nYou must now choose if you want to be a truly 'lawful'"));
    assert!(good.ends_with(b"unless you start a new character.\r\n"));
}

// EDITCHA1/EDTCHAN2, the full-screen displays: forms, not catalogue entries
#[test]
fn fsd_templates_are_forms() {
    let package = crate::package();
    for (name, text, marker) in [
        ("editcha1", package.editcha1(), "$$"),
        ("edtchan2", package.edtchan2(), "\r"),
    ] {
        assert!(text.starts_with("\u{1b}[0m"));
        assert!(text.contains(marker));
        // every character has a CP437 byte, so the template round-trips to the original
        assert!(
            text.chars().all(|c| cp437::encode_char(c).is_some()),
            "{name}"
        );
        assert!(!wccmmud().messages.contains_key(name) && !wcctext().messages.contains_key(name));
    }
}

// ---- the help topics ----------------------------------------------------------------

// WCCMMHLP.MSG: 263 messages = HL1B0 + 131 topic strings + 131 texts (the card says 263 rows:
// that is the message count); the last topic is the end mark
#[test]
fn help_topics_load() {
    let help = crate::package().help();
    assert_eq!(help.topics.len(), 131);
    assert_eq!(1 + 2 * help.topics.len(), 263);
    let first = &help.topics[0];
    assert_eq!(first.key, "HL1T1");
    assert_eq!(first.words, "Commands comands");
    assert!(first.words.starts_with("Commands"));
    assert!(help.topics[1].words.starts_with("Communications telepaths"));
    assert!(
        help.topics
            .iter()
            .any(|t| t.key == "HL1T2" && t.words.starts_with("Combat attacking bash punch"))
    );
    let last = help.topics.last().unwrap();
    assert_eq!(last.words, HELP_END_MARK);
    assert_eq!(last.key, "HL1T9999");
    // HL1B0 starts with the clear screen
    assert!(help.overview.starts_with("\u{1b}[0m\u{1b}[2JType "));
    // the topic string of a pair and its text are adjacent messages even where the numbers
    // differ (HL1T60 is followed by HL1B61)
    let t60 = help.topics.iter().find(|t| t.key == "HL1T60").unwrap();
    assert_eq!(t60.body_key, "HL1B61");
    assert!(t60.words.starts_with("HP "));
}

// the two %c of the texts are the say and yell characters
#[test]
fn help_texts_take_the_say_and_yell_characters() {
    let help = crate::package().help();
    let with_chars = help
        .topics
        .iter()
        .filter(|t| t.text.contains("{talk_char}"))
        .count();
    assert!(with_chars >= 1);
    for t in &help.topics {
        assert!(!t.text.contains('\n'), "{} holds an LF", t.key);
        assert!(
            t.text.chars().all(|c| cp437::encode_char(c).is_some()),
            "{} has a character CP437 cannot show",
            t.key
        );
    }
    // a topic set without the end marker, or with a bad argument, is refused
    let mut no_end = help.clone();
    no_end.topics.pop();
    assert!(no_end.validated().is_err());
    let mut bad = help.clone();
    let t = bad
        .topics
        .iter_mut()
        .find(|t| t.text.contains("{talk_char}"))
        .unwrap();
    t.text = t.text.replacen("{talk_char}", "{who}", 1);
    assert!(bad.validated().is_err());
    assert!(HelpTopics::validated(help.clone()).is_ok());
}

// ---- the generated document is the converter's output -------------------------------

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

#[test]
fn sha256_known_answers() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// Without MMUD_DIST_ZIP: the committed document is what the converter wrote (stored hash).
#[test]
fn generated_files_are_unedited() {
    let doc = std::fs::read(repo_dir().join("docs/config-defaults.md")).unwrap();
    assert_eq!(sha256_hex(&doc), SHA_CONFIG_DEFAULTS);
}

/// With `MMUD_DIST_ZIP` set (the path of `WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`), the converter is
/// run again and must write an identical document. Needs `unzip` and `python3`.
#[test]
fn converter_regenerates_identical_files() {
    let Some(zip) = std::env::var_os("MMUD_DIST_ZIP") else {
        eprintln!("MMUD_DIST_ZIP not set: skipping the regeneration check");
        return;
    };
    let work = std::env::temp_dir().join(format!("msgconv-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let unzip = std::process::Command::new("unzip")
        .args(["-o", "-q"])
        .arg(&zip)
        .args(["WCCMMUD.MSG", "-d"])
        .arg(&work)
        .status()
        .expect("unzip runs");
    assert!(unzip.success());
    let tool = repo_dir().join("tools/msgconv/msgconv.py");
    let run = |args: &[&Path]| {
        let status = std::process::Command::new("python3")
            .arg("-I")
            .arg(&tool)
            .args(args)
            .current_dir(&work)
            .status()
            .expect("python3 runs");
        assert!(status.success());
    };
    let out = |name: &str| work.join(name);
    let flag = |s: &'static str| Path::new(s);
    run(&[
        flag("--msg"),
        &out("WCCMMUD.MSG"),
        flag("--options"),
        &out("config-defaults.md"),
    ]);
    assert_eq!(
        std::fs::read(out("config-defaults.md")).unwrap(),
        std::fs::read(repo_dir().join("docs/config-defaults.md")).unwrap(),
        "config-defaults.md differs from the converter's output"
    );
    let _ = std::fs::remove_dir_all(&work);
}

// ---- docs/config-defaults.md agrees with GameConfig ---------------------------------

type Table = Vec<BTreeMap<String, String>>;

/// The rows of every table in the document, keyed by header cell, with `\|` unescaped.
fn doc_tables() -> Vec<Table> {
    let doc = std::fs::read_to_string(repo_dir().join("docs/config-defaults.md")).unwrap();
    let mut tables = Vec::new();
    let mut current: Option<(Vec<String>, Table)> = None;
    let split = |line: &str| -> Vec<String> {
        line.replace("\\|", "\u{1}")
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().replace('\u{1}', "|"))
            .collect()
    };
    for line in doc.lines() {
        if !line.starts_with('|') {
            if let Some((_, rows)) = current.take() {
                tables.push(rows);
            }
            continue;
        }
        match &mut current {
            None => current = Some((split(line), Vec::new())),
            Some((header, rows)) => {
                if line.starts_with("|---") {
                    continue;
                }
                let cells = split(line);
                rows.push(header.iter().cloned().zip(cells).collect());
            }
        }
    }
    if let Some((_, rows)) = current {
        tables.push(rows);
    }
    tables
}

fn unquote(cell: &str) -> &str {
    cell.strip_prefix('`')
        .and_then(|c| c.strip_suffix('`'))
        .unwrap_or(cell)
}

// docs/config-defaults.md "GameConfig default" column equals GameConfig::default()
#[test]
fn doc_defaults_equal_game_config() {
    let config = toml::Value::try_from(GameConfig::default()).unwrap();
    let config = config.as_table().unwrap();
    let mut checked = 0;
    let mut missing = Vec::new();
    for table in doc_tables() {
        for row in table {
            let (Some(key), Some(default)) = (row.get("TOML key"), row.get("GameConfig default"))
            else {
                continue;
            };
            let Some(key) = unquote(key).strip_prefix("game.") else {
                continue;
            };
            if default.starts_with("(not in GameConfig") {
                assert!(
                    !config.contains_key(key),
                    "{key} is in GameConfig after all"
                );
                missing.push(key.to_string());
                continue;
            }
            let want: toml::Table = toml::from_str(&format!("v = {}", unquote(default)))
                .unwrap_or_else(|e| panic!("{key}: default {default:?}: {e}"));
            // a dotted key names a nested table's field (`buffers.monsbuf`)
            let (path, last) = key.rsplit_once('.').unwrap_or(("", key));
            let got = path
                .split('.')
                .filter(|k| !k.is_empty())
                .try_fold(config, |t, k| t.get(k)?.as_table())
                .and_then(|t| t.get(last))
                .unwrap_or_else(|| panic!("{key} is not a GameConfig field"));
            assert_eq!(&want["v"], got, "{key}");
            checked += 1;
        }
    }
    assert!(checked >= 40, "only {checked} defaults checked");
    assert!(missing.is_empty(), "{missing:?}");
}

// docs/config-defaults.md lists all 34 WCCMMUD.INI options, once each
#[test]
fn doc_lists_all_34_ini_options() {
    let want = [
        "TRACK_LIMITED",
        "LOGFILE",
        "EXP_IN_TOPGANG",
        "LOG_SYSOP_COMMANDS",
        "TOURNAMENT_PLAY",
        "UPDATE_SPEED",
        "SYS_MAP_KEY",
        "SYS_REPORT_KEY",
        "SYS_GOTO_KEY",
        "SYS_LIGHTNING_KEY",
        "SYS_STATUS_KEY",
        "SYS_RESET_ROOMS",
        "SYS_ADDLIFE_KEY",
        "SYS_RETRAIN_KEY",
        "SYS_ALIGNMENT_KEY",
        "SYS_EDITEVIL_KEY",
        "SYS_JAIL_KEY",
        "SYS_SPEAK_KEY",
        "SYS_LISTUSER_KEY",
        "SYS_LISTLIMIT_KEY",
        "SYS_USERINFO_KEY",
        "SYS_CHANGE_KEY",
        "SYS_TOPTEN_KEY",
        "SYS_DISBAND_KEY",
        "SYS_BUFFERS_KEY",
        "SYS_CLEARSUICIDE_KEY",
        "MAXIMUM_BANK_BALANCE",
        "ENABLE_CLASS_CHANGE",
        "BLANK_LASTNAME_ALLOWED",
        "RACE_CHANGE_DURATION",
        "CODEC_OF_TWINKS",
        "SYS_LOG_DEATHS",
        "KID_GLOVES",
        "SYS_DISABLE_GANGS",
    ];
    assert_eq!(want.len(), 34);
    let tables = doc_tables();
    let ini = tables
        .iter()
        .find(|t| {
            t.first()
                .is_some_and(|r| r.contains_key("Status") && r.contains_key("Default"))
        })
        .expect("the INI table");
    let names: Vec<&str> = ini.iter().map(|r| unquote(&r["Option"])).collect();
    assert_eq!(names, want);
}

// docs/config-defaults.md: every .MSG option entry named here is accounted for exactly once
#[test]
fn doc_accounts_for_every_msg_option() {
    let mut names: Vec<String> = Vec::new();
    for table in doc_tables() {
        let Some(first) = table.first() else { continue };
        let msg_table = first.contains_key("Module default")
            || first.contains_key("Role mapping")
            || first.contains_key("Why");
        if !msg_table {
            continue;
        }
        names.extend(table.iter().map(|r| unquote(&r["Option"]).to_string()));
    }
    for o in [
        "DEATHHP", "INACTIVE", "PROFCHEK", "EXPINTEN", "DELUSER", "SYSOPKEY", "GAMOPKEY",
        "GOSSKEY", "AUCTKEY", "ROOMBUF", "GANGBUF", "COINSWP", "GCOINSW", "INVGLO1", "INVGLO10",
        "ACTIVATE", "DEMOLVL", "PLAYKEY", "DATADIR",
    ] {
        assert_eq!(names.iter().filter(|n| *n == o).count(), 1, "{o}");
    }
    let before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(before, names.len(), "an option appears twice");
}
