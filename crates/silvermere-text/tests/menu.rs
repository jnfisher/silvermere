//! The menu, the entry banner (`USRFOLED`) and the logon greeting (`USRLOGIN`) against the
//! T-005 capture `entry-existing` of the original, and the optional lines against the pattern
//! registry.

use regex::Regex;
use serde::Deserialize;
use silvermere_text::cp437;
use silvermere_text::menu::{
    MUDVER, MenuInput, RaceChange, SHORTVER, render_entry_banner, render_login_greeting,
    render_menu,
};
use silvermere_text::strip::strip_ansi;

/// `entry-existing/raw.bin`, the bytes from the menu's `ESC[0m ESC[2J` through the
/// `[MAJORMUD]: ` prompt (the host's own clear-screen before it and the echo after it
/// left out).
const CAPTURED_MENU: &[u8] = b"\x1b[0m\x1b[2J\r\n \x1b[1mM A J O R  M U D \x1b[33mv1.11p-WG (Dec 30 2005 14:20:05)\r\n\x1b[0m{Realm Of Legends}\r\n\x1b[1;35m*ANSI RECOMMENDED*\r\n\r\n\x1b[30m[\x1b[37mE\x1b[30m] . \x1b[0mEnter the Realm\r\n\x1b[1;30m[\x1b[37mH\x1b[30m] . \x1b[0mHelp\r\n\x1b[1;30m[\x1b[37mA\x1b[30m] . \x1b[0mAbout the Game\r\n\x1b[1;30m[\x1b[37mC\x1b[30m] . \x1b[0mGame Counts\r\n\x1b[1;30m[\x1b[37mT\x1b[30m] . \x1b[0mTopten Adventurers\r\n\x1b[1;30m[\x1b[37mG\x1b[30m] . \x1b[0mTopten Gangs\r\n\x1b[1;30m[\x1b[37mW\x1b[30m] . \x1b[0mWho's in the Realm\r\n\x1b[1;30m[\x1b[37mR\x1b[30m] . \x1b[0mRelease Notes\r\n\x1b[1;30m[\x1b[37mI\x1b[30m] . \x1b[0mInformation\r\n\x1b[1;30m[\x1b[37mX\x1b[30m] . \x1b[0mExit Game\r\n\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: ";

/// `entry-existing/plain.txt`, the menu block: the blank line of the clear screen, then
/// ` M A J O R  M U D ...` through `[MAJORMUD]:` (the typed `E` after it left out).
const CAPTURED_PLAIN: &[&str] = &[
    "",
    " M A J O R  M U D v1.11p-WG (Dec 30 2005 14:20:05)",
    "{Realm Of Legends}",
    "*ANSI RECOMMENDED*",
    "",
    "[E] . Enter the Realm",
    "[H] . Help",
    "[A] . About the Game",
    "[C] . Game Counts",
    "[T] . Topten Adventurers",
    "[G] . Topten Gangs",
    "[W] . Who's in the Realm",
    "[R] . Release Notes",
    "[I] . Information",
    "[X] . Exit Game",
    "",
    "[MAJORMUD]:",
];

/// `entry-existing/raw.bin` after the player typed `E`: `USRFOLED`, up to the room.
const CAPTURED_BANNER: &[u8] = b"\x1b[0m\x1b[2J\r\n\x1b[1;33mMajorMUD - ROL\r\n\r\n\x1b[0m\x1b[1;30m=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=\r\n\r\n";

/// `entry-existing/raw.bin` after the password: `USRLOGIN`.
const CAPTURED_GREETING: &[u8] =
    b"\r\n\x1b[1mFantasy awaits you in \x1b[33mMajorMUD v1.11p-WG!\r\n\r\n";

fn plain_player() -> MenuInput<'static> {
    MenuInput {
        version: MUDVER,
        race_change: None,
        sysop: false,
        tournament: false,
    }
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

// MENUTEXT and MENUPMPT as the capture shows them
#[test]
fn plain_menu_equals_the_capture() {
    let menu = render_menu(plain_player());
    assert_eq!(lines(&menu), CAPTURED_PLAIN);
    assert_eq!(menu, CAPTURED_MENU);
    // no newline after the prompt
    assert!(menu.ends_with(b"\x1b[0m: "));
}

// each line of the plain menu is one the registry knows
#[test]
fn menu_lines_match_the_registry() {
    let items = pattern("mud_menu_item");
    let title = pattern("mud_title");
    let world = pattern("mud_world");
    let ansi = pattern("mud_ansi_note");
    let prompt = pattern("mud_menu_prompt");
    let menu = lines(&render_menu(plain_player()));
    assert!(title.is_match(menu[1].trim_start()));
    assert!(world.is_match(&menu[2]));
    assert!(ansi.is_match(&menu[3]));
    for line in &menu[5..15] {
        assert!(items.is_match(line), "{line}");
    }
    assert!(prompt.is_match(&menu[16]));
}

// the race change line, with the minutes left or the last chance
#[test]
fn race_change_lines() {
    let items = pattern("mud_menu_item");
    let cases = [
        (
            false,
            12,
            "[CHANGERACE] . Race Change [12 game minutes left]",
        ),
        (
            true,
            12,
            "[CHANGERACE] . Race/Class Change [12 game minutes left]",
        ),
        (false, 0, "[CHANGERACE] . Race Change [Last chance]"),
        (true, -3, "[CHANGERACE] . Race/Class Change [Last chance]"),
    ];
    for (class_too, minutes_left, want) in cases {
        let menu = render_menu(MenuInput {
            race_change: Some(RaceChange {
                class_too,
                minutes_left,
            }),
            ..plain_player()
        });
        let l = lines(&menu);
        // after the MENUTEXT block (whose blank line ends it), then its own blank line
        assert_eq!(l[16], want);
        assert_eq!(l[17], "");
        assert_eq!(l[18], "[MAJORMUD]:");
        assert!(items.is_match(&l[16]), "{}", l[16]);
    }
    let menu = render_menu(MenuInput {
        race_change: Some(RaceChange {
            class_too: false,
            minutes_left: 12,
        }),
        ..plain_player()
    });
    let at = CAPTURED_MENU.len() - b"\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: ".len();
    assert_eq!(
        &menu[at..],
        b"\x1b[1;30m[\x1b[37mCHANGERACE\x1b[30m] . \x1b[0mRace Change [12 game minutes left]\r\n\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: "
    );
}

// the sysop line and the tournament notice, in that order, after
// the race change line
#[test]
fn sysop_and_tournament_lines() {
    let items = pattern("mud_menu_item");
    let tournament = pattern("tournament_notice");
    let menu = render_menu(MenuInput {
        race_change: Some(RaceChange {
            class_too: false,
            minutes_left: 5,
        }),
        sysop: true,
        tournament: true,
        ..plain_player()
    });
    let l = lines(&menu);
    assert_eq!(l[16], "[CHANGERACE] . Race Change [5 game minutes left]");
    assert_eq!(l[18], "[SYSOP] . For Sysop Menu");
    assert!(items.is_match(&l[18]));
    assert_eq!(l[19], "");
    assert_eq!(l[20], "*** MajorMUD Tournament Play enabled! ***");
    assert!(tournament.is_match(&l[20]));
    assert_eq!(l[21], "[MAJORMUD]:");
    assert_eq!(l.len(), 22);
}

// the title shows whatever version string the config gives
#[test]
fn version_is_config() {
    let menu = lines(&render_menu(MenuInput {
        version: "2.0 (silvermere)",
        ..plain_player()
    }));
    assert_eq!(menu[1], " M A J O R  M U D v2.0 (silvermere)");
}

// USRFOLED, and USRLOGIN with the short version
#[test]
fn banner_and_greeting_equal_the_capture() {
    assert_eq!(render_entry_banner(), CAPTURED_BANNER);
    assert!(pattern("enter_banner").is_match(&lines(&render_entry_banner())[1]));
    assert!(pattern("enter_rule").is_match(&lines(&render_entry_banner())[3]));
    assert_eq!(render_login_greeting(SHORTVER), CAPTURED_GREETING);
    assert!(pattern("login_greeting").is_match(&lines(&render_login_greeting(SHORTVER))[1]));
}

/// The renderers' bytes are found in the committed capture file itself.
#[test]
fn renderers_appear_in_the_capture_file() {
    let raw = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/captures/entry-existing/raw.bin"),
    )
    .unwrap();
    let contains = |want: &[u8]| raw.windows(want.len()).any(|w| w == want);
    assert!(contains(&render_menu(plain_player())));
    assert!(contains(&render_entry_banner()));
    assert!(contains(&render_login_greeting(SHORTVER)));
}

#[test]
fn snapshots() {
    let mut out = String::new();
    let mut add = |name: &str, bytes: Vec<u8>| {
        out.push_str(&format!("{name}:\n{}|\n", bytes.escape_ascii()));
    };
    add("menu_plain", render_menu(plain_player()));
    add(
        "menu_everything",
        render_menu(MenuInput {
            race_change: Some(RaceChange {
                class_too: true,
                minutes_left: 0,
            }),
            sysop: true,
            tournament: true,
            ..plain_player()
        }),
    );
    add("entry_banner", render_entry_banner());
    add("login_greeting", render_login_greeting(SHORTVER));
    insta::assert_snapshot!(out);
}
