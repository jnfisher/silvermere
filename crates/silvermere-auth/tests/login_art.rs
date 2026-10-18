//! The login art (T-118): the MegaMud-safe rules for the default and for a sysop's
//! replacement file, the fallback to the default, and its place before the first prompt.

use std::path::PathBuf;

use silvermere_auth::art::{self, ArtError};
use silvermere_auth::login::Output;
use silvermere_auth::{AuthMsg, LoginConfig, LoginDialogue};

/// A fresh assets directory under the system temp directory.
fn assets(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("silvermere-art-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_default_is_megamud_safe() {
    assert_eq!(art::check(art::DEFAULT), Ok(()));
    assert!(art::DEFAULT.ends_with(b"\x1b[0m\r\n"));
    assert!(!art::DEFAULT.contains(&0x1A));
}

#[test]
fn the_default_is_the_chosen_candidate() {
    // John picked candidate 02 (PR #126); tools/art/make_logos.py regenerates it.
    let candidate = include_bytes!("../../../tools/art/candidates/02-underwld-mere.ans");
    assert_eq!(art::DEFAULT, candidate);
}

#[test]
fn check_rejects_each_broken_rule() {
    let ok = b"\x1b[0;1;37mSILVER\x1b[0m\r\n";
    assert_eq!(art::check(ok), Ok(()));
    assert_eq!(art::check(b"\x1b[0;1;37mSILVER\x1b[0m"), Ok(()));

    let wide = [b"\x1b[0m".as_slice(), &[b'x'; 80], b"\x1b[0m\r\n"].concat();
    assert_eq!(
        art::check(&wide),
        Err(ArtError::TooWide {
            line: 1,
            columns: 80
        })
    );
    let fits = [b"\x1b[0m".as_slice(), &[0xDB; 79], b"\x1b[0m\r\n"].concat();
    assert_eq!(art::check(&fits), Ok(()));

    let tall = [b"x\r\n".repeat(20).as_slice(), b"\x1b[0m\r\n"].concat();
    assert_eq!(art::check(&tall), Err(ArtError::TooManyLines(21)));
    let twenty = [b"x\r\n".repeat(19).as_slice(), b"\x1b[0m\r\n"].concat();
    assert_eq!(art::check(&twenty), Ok(()));

    assert_eq!(art::check(b"x\ny\x1b[0m"), Err(ArtError::BareLineEnd(1)));
    assert_eq!(
        art::check(b"x\x07\x1b[0m"),
        Err(ArtError::Control { line: 1, byte: 7 })
    );
    assert_eq!(art::check(b"\x1b[2Jx\x1b[0m"), Err(ArtError::Escape(1)));
    assert_eq!(art::check(b"\x1b[10;10Hx\x1b[0m"), Err(ArtError::Escape(1)));
    for code in ["38;5;12", "48;2;1;2;3", "90", "100", "7"] {
        let bad = format!("\x1b[{code}mx\x1b[0m");
        assert!(
            matches!(
                art::check(bad.as_bytes()),
                Err(ArtError::Sgr { line: 1, .. })
            ),
            "{code}"
        );
    }
    assert_eq!(art::check(b"\x1b[1;37mx\r\n"), Err(ArtError::NoReset));
}

#[test]
fn no_file_gives_the_default() {
    let dir = assets("none");
    let got = art::load(&dir);
    assert_eq!(got.bytes, art::DEFAULT);
    assert_eq!(got.warning, None);
}

#[test]
fn a_valid_replacement_is_used_without_its_sauce() {
    let dir = assets("valid");
    let mine = b"\x1b[0;36mMY BBS\x1b[0m\r\n";
    let sauce = [b"\x1a".as_slice(), b"SAUCE00", &[0; 121]].concat();
    std::fs::write(dir.join(art::FILE_NAME), [mine.as_slice(), &sauce].concat()).unwrap();
    let got = art::load(&dir);
    assert_eq!(got.bytes, mine);
    assert_eq!(got.warning, None);
}

#[test]
fn an_empty_replacement_shows_no_art() {
    let dir = assets("empty");
    std::fs::write(dir.join(art::FILE_NAME), b"").unwrap();
    let got = art::load(&dir);
    assert!(got.bytes.is_empty());
    assert_eq!(got.warning, None);
}

#[test]
fn a_broken_replacement_falls_back_to_the_default_with_a_warning() {
    let dir = assets("broken");
    // A 256-colour code, as a modern editor might save.
    std::fs::write(dir.join(art::FILE_NAME), b"\x1b[38;5;45mMY BBS\x1b[0m\r\n").unwrap();
    let got = art::load(&dir);
    assert_eq!(got.bytes, art::DEFAULT);
    let warning = got.warning.expect("a warning");
    assert!(warning.contains("line 1"), "{warning}");
}

#[test]
fn the_art_comes_first_and_the_prompts_are_unchanged() {
    let mut plain = LoginDialogue::new(LoginConfig::default());
    let before = plain.start();
    let mut with_art = LoginDialogue::new(LoginConfig {
        login_art: art::DEFAULT.to_vec(),
        ..LoginConfig::default()
    });
    let out = with_art.start();
    assert_eq!(out[0], Output::Raw(art::DEFAULT.to_vec()));
    assert_eq!(out[1..], before[..]);
    assert_eq!(out[1], Output::Text(AuthMsg::WelcomeEnd));
}
