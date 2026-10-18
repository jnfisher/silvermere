//! The prompt: the twelve standard forms seen in play, after `strip_ansi`, each matched
//! against the pattern registry's `STD_PROMPT` (`patterns.json` `prompt.std`); the colour
//! bytes against hand-built colour-table rows; the custom mode; and the original's bytes from the
//! T-005 captures.

use regex::Regex;
use serde::Deserialize;
use silvermere_text::ansi::{ANSI_CLRLINE, ANSI_CYAN, ANSI_GREY, ANSI_LTCYAN, ANSI_LTRED};
use silvermere_text::colour::Palette;
use silvermere_text::prompt::{DebugInfo, PromptInput, render_prompt};
use silvermere_text::statline::StatlineCtx;
use silvermere_text::strip::strip_ansi;
use silvermere_types::enums::{Magery, StatlineMode};

#[derive(Deserialize)]
struct Registry {
    prompt: PromptPatterns,
}

#[derive(Deserialize)]
struct PromptPatterns {
    std: String,
}

fn std_prompt() -> Regex {
    let registry: Registry =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json"))
            .expect("patterns.json parses");
    Regex::new(&format!("^(?:{})$", registry.prompt.std)).expect("STD_PROMPT compiles")
}

fn input(hp: i32, mana: i32, maxmana: i32) -> PromptInput<'static> {
    PromptInput {
        hp,
        maxhp: 48,
        mana,
        maxmana,
        magery: Some(Magery::Mage),
        resting: false,
        meditating: false,
        mode: StatlineMode::Full,
        template: None,
        palette: Palette::default(),
        sysop_debug: None,
    }
}

fn ctx() -> StatlineCtx<'static> {
    StatlineCtx {
        hp: 48,
        maxhp: 48,
        mana: 0,
        maxmana: 0,
        exp: 0,
        exp_needed: Some(0),
        wealth_copper: 0,
        first_name: "Aldric",
        evil_warn: false,
        resting: false,
        meditating: false,
        pronoun_possessive: "his",
        pronoun_object: "him",
        title: None,
        palette: Palette::default(),
    }
}

fn plain(input: &PromptInput) -> String {
    String::from_utf8(strip_ansi(&render_prompt(input, &ctx()))).unwrap()
}

// the twelve standard forms
#[test]
fn standard_forms_after_strip_ansi() {
    let resting = |mut i: PromptInput<'static>| {
        i.resting = true;
        i
    };
    let meditating = |mut i: PromptInput<'static>| {
        i.meditating = true;
        i
    };
    let kai = |mut i: PromptInput<'static>| {
        i.magery = Some(Magery::Kai);
        i
    };
    let mut off = input(48, 0, 0);
    off.mode = StatlineMode::Off;
    let mut debug = input(48, 0, 0);
    debug.sysop_debug = Some(DebugInfo {
        energy: 1234,
        sneaking: true,
        just_arrived: true,
        hidden: true,
    });
    let mut no_class = input(32, 22, 30);
    no_class.magery = None;
    let cases: [(PromptInput, &str); 12] = [
        // no mana
        (input(48, 0, 0), "[HP=48]:"),
        // the mark inside the brackets without mana
        (resting(input(29, 0, 0)), "[HP=29 (Resting) ]:"),
        (meditating(input(29, 0, 0)), "[HP=29 (Meditating) ]:"),
        // with mana
        (input(32, 22, 30), "[HP=32/MA=22]:"),
        // the mark after "]:" with mana
        (resting(input(7, 19, 30)), "[HP=7/MA=19]: (Resting) "),
        (meditating(input(7, 19, 30)), "[HP=7/MA=19]: (Meditating) "),
        // KAI by the class magery
        (kai(input(40, 5, 10)), "[HP=40/KAI=5]:"),
        // negative hit points while unconscious
        (input(-1, 19, 30), "[HP=-1/MA=19]:"),
        (input(-107, 0, 0), "[HP=-107]:"),
        // off
        (off, ">"),
        // sysop debug
        (debug, "[HP=48]:{Energy=1234 Snk JstEnt Hdn }:"),
        // a mana user without a class record: nothing after the colour
        (no_class, ""),
    ];
    let rx = std_prompt();
    for (input, want) in cases {
        let got = plain(&input);
        assert_eq!(got, want, "{input:?}");
        if want.starts_with('[') {
            assert!(rx.is_match(&got), "STD_PROMPT does not match {got:?}");
        }
    }
}

// KAI comes from the class, not from the mana figure
#[test]
fn kai_follows_the_class_not_the_value() {
    let mut i = input(40, 0, 10);
    i.magery = Some(Magery::Kai);
    assert_eq!(plain(&i), "[HP=40/KAI=0]:");
    i.magery = Some(Magery::Priest);
    i.mana = 5;
    assert_eq!(plain(&i), "[HP=40/MA=5]:");
}

// the debug flags print only when set; the energy always
#[test]
fn debug_suffix_flags() {
    let mut i = input(48, 0, 0);
    i.sysop_debug = Some(DebugInfo {
        energy: -5,
        ..DebugInfo::default()
    });
    assert_eq!(plain(&i), "[HP=48]:{Energy=-5 }:");
    i.sysop_debug = Some(DebugInfo {
        energy: 900,
        hidden: true,
        ..DebugInfo::default()
    });
    assert_eq!(plain(&i), "[HP=48]:{Energy=900 Hdn }:");
    // the debug suffix belongs to the standard line: not after OFF
    i.mode = StatlineMode::Off;
    assert_eq!(plain(&i), ">");
    // nor after a custom line
    i.mode = StatlineMode::Custom;
    i.template = Some("%h:");
    assert_eq!(plain(&i), "48:");
}

// the colour bytes, palettes 0 and 2, HP above half, at half and at a quarter (rows
// COL_PROMPT 2, COL_PROMPTVAL 3, COL_HPHURT 4, COL_HPLOW 5 of the colour table)
#[test]
fn colour_bytes_by_palette_and_health() {
    fn bytes(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }
    let p0 = Palette::new(0).unwrap();
    let p2 = Palette::new(2).unwrap();
    let render = |hp: i32, maxhp: i32, mana: i32, maxmana: i32, palette: Palette| {
        let mut i = input(hp, mana, maxmana);
        i.maxhp = maxhp;
        i.palette = palette;
        render_prompt(&i, &ctx())
    };
    // palette 0, above half, no mana: COL_PROMPTVAL is NOCOLOR in palette 0
    assert_eq!(
        render(48, 48, 0, 0, p0),
        bytes(&[
            ANSI_CLRLINE,
            ANSI_GREY,
            b"[HP=",
            b"",
            b"48",
            ANSI_GREY,
            b"]:"
        ])
    );
    // palette 2, above half, with mana
    assert_eq!(
        render(32, 40, 22, 30, p2),
        bytes(&[
            ANSI_CLRLINE,
            ANSI_CYAN,
            b"[HP=",
            ANSI_LTCYAN,
            b"32",
            ANSI_CYAN,
            b"/MA=",
            ANSI_LTCYAN,
            b"22",
            ANSI_CYAN,
            b"]:",
        ])
    );
    // palette 0, exactly half: COL_HPHURT is white in palette 0
    assert_eq!(
        render(20, 40, 22, 30, p0),
        bytes(&[
            ANSI_CLRLINE,
            ANSI_GREY,
            b"[HP=",
            b"\x1B[1;37m",
            b"20",
            ANSI_GREY,
            b"/MA=",
            b"",
            b"22",
            ANSI_GREY,
            b"]:",
        ])
    );
    // palette 2, exactly a quarter: COL_HPLOW
    assert_eq!(
        render(10, 40, 0, 0, p2),
        bytes(&[
            ANSI_CLRLINE,
            ANSI_CYAN,
            b"[HP=",
            ANSI_LTRED,
            b"10",
            ANSI_CYAN,
            b"]:"
        ])
    );
    // palette 2, half: COL_HPHURT is grey in palette 2
    assert_eq!(
        render(20, 40, 0, 0, p2),
        bytes(&[
            ANSI_CLRLINE,
            ANSI_CYAN,
            b"[HP=",
            ANSI_GREY,
            b"20",
            ANSI_CYAN,
            b"]:"
        ])
    );
}

// OFF is the two prefixes and ">"
#[test]
fn off_bytes() {
    let mut i = input(48, 0, 0);
    i.mode = StatlineMode::Off;
    assert_eq!(render_prompt(&i, &ctx()), b"\x1B[79D\x1B[K\x1B[0;37m>");
}

// modes above SL_FULL print the template; %n arrives as \r\n; the template is cut to
// 60 characters
#[test]
fn custom_modes() {
    for mode in [
        StatlineMode::Custom,
        StatlineMode::FullCustom,
        StatlineMode::Unknown(9),
    ] {
        let mut i = input(48, 0, 0);
        i.mode = mode;
        i.template = Some("[HP=%h/%H]:%nok");
        assert_eq!(
            render_prompt(&i, &ctx()),
            b"\x1B[79D\x1B[K\x1B[0;37m[HP=48/48]:\r\nok",
            "{mode:?}"
        );
    }
    let long = format!("{}{}", "a".repeat(60), "%h");
    let mut i = input(48, 0, 0);
    i.mode = StatlineMode::Custom;
    i.template = Some(&long);
    assert_eq!(plain(&i), "a".repeat(60));
    // no template stored: nothing after the colour
    i.template = None;
    assert_eq!(render_prompt(&i, &ctx()), b"\x1B[79D\x1B[K\x1B[0;37m");
}

// T-005 captures of the original: entry-existing (a level 1 warrior, full HP) and
// death-rebirth (negative HP with mana), palette 0
#[test]
fn bytes_from_the_captures() {
    let mut i = input(35, 0, 0);
    i.maxhp = 35;
    assert_eq!(
        render_prompt(&i, &ctx()),
        b"\x1B[79D\x1B[K\x1B[0;37m[HP=35\x1B[0;37m]:"
    );
    let mut i = input(-56, 22, 22);
    i.maxhp = 30;
    assert_eq!(
        render_prompt(&i, &ctx()),
        b"\x1B[79D\x1B[K\x1B[0;37m[HP=\x1B[1;31m-56\x1B[0;37m/MA=22\x1B[0;37m]:"
    );
}

/// The prompts above are found in the committed capture files themselves.
#[test]
fn prompts_appear_in_the_capture_files() {
    let captures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/captures");
    let mut i = input(35, 0, 0);
    i.maxhp = 35;
    let raw = std::fs::read(captures.join("entry-existing/raw.bin")).unwrap();
    let want = render_prompt(&i, &ctx());
    assert!(raw.windows(want.len()).any(|w| w == want));
    let mut i = input(-56, 22, 22);
    i.maxhp = 30;
    let raw = std::fs::read(captures.join("death-rebirth/raw.bin")).unwrap();
    let want = render_prompt(&i, &ctx());
    assert!(raw.windows(want.len()).any(|w| w == want));
}

#[test]
fn snapshots() {
    let mut out = String::new();
    let mut add = |name: &str, i: &PromptInput| {
        let bytes = render_prompt(i, &ctx());
        out.push_str(&format!("{name}: {}|\n", bytes.escape_ascii()));
    };
    add("no_mana", &input(48, 0, 0));
    let mut i = input(7, 19, 30);
    i.resting = true;
    add("mana_resting", &i);
    let mut i = input(10, 5, 10);
    i.magery = Some(Magery::Kai);
    i.palette = Palette::new(2).unwrap();
    add("kai_low_palette2", &i);
    let mut i = input(48, 0, 0);
    i.mode = StatlineMode::Off;
    add("off", &i);
    let mut i = input(48, 0, 0);
    i.sysop_debug = Some(DebugInfo {
        energy: 1234,
        sneaking: true,
        just_arrived: true,
        hidden: true,
    });
    add("sysop_debug", &i);
    let mut i = input(20, 0, 0);
    i.mode = StatlineMode::FullCustom;
    i.template = Some("%d[%h/%H]%f2%r%n%B>");
    add("custom", &i);
    insta::assert_snapshot!(out);
}
