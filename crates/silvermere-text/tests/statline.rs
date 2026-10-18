//! Statline expansion: the forum custom prompts, every code in
//! isolation, the quirks (`%%`, `%b1`, codes in the wrong state), the 60-character
//! storage cut and the `SUBSTMAX` stop.

use silvermere_text::ansi::{ANSI_CYAN, ANSI_LTRED};
use silvermere_text::colour::Palette;
use silvermere_text::statline::{STATLINELEN, SUBSTMAX, StatlineCtx, cut_template, substitute};

fn ctx() -> StatlineCtx<'static> {
    StatlineCtx {
        hp: 395,
        maxhp: 395,
        mana: 392,
        maxmana: 433,
        exp: 123_456,
        exp_needed: Some(200_000),
        wealth_copper: 180_750,
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

fn ascii(s: &[u8]) -> &str {
    std::str::from_utf8(s).unwrap()
}

// the custom prompts of the forum captures
#[test]
fn forum_templates() {
    assert_eq!(
        substitute("[HP=%h/%H,MA=%m/%M]:", &ctx()),
        "[HP=395/395,MA=392/433]:"
    );
    let c = StatlineCtx {
        hp: 1,
        maxhp: 2,
        mana: 8,
        maxmana: 42,
        ..ctx()
    };
    assert_eq!(
        substitute("[hp=%h/%H,MA=%m/%M,CASH=%c,WARNS=%w]:", &c),
        "[hp=1/2,MA=8/42,CASH=180750,WARNS=Off]:"
    );
}

// each code alone (the twenty letter codes, %%, a foreground, a
// background and the (sic) %b1)
#[test]
fn every_code_in_isolation() {
    let c = ctx();
    let cases: [(&str, &str); 24] = [
        ("%h", "395"),
        ("%H", "395"),
        ("%m", "392"),
        ("%M", "433"),
        ("%x", "123456"),
        ("%X", "76544"),
        ("%c", "180750"),
        ("%i", "Aldric"),
        ("%t", "nothing"),
        ("%p", "his"),
        ("%P", "him"),
        ("%r", ""),
        ("%w", "Off"),
        ("%n", "\r"),
        ("%d", "\x1B[0;37m"),
        ("%B", "\x1B[1m"),
        ("%N", "\x1B[0m"),
        ("%U", "\x1B[4m"),
        ("%L", "\x1B[5m"),
        ("%R", "\x1B[7m"),
        ("%%", "%%"),
        ("%f3", "\x1B[33m"),
        ("%b4", "\x1B[44m"),
        ("%b1", "\x1B[42m"), // as the original shows it: green, not red
    ];
    for (template, want) in cases {
        assert_eq!(substitute(template, &c), want, "{template}");
    }
}

// every foreground and background digit
#[test]
fn colour_digits() {
    for n in 0..8u8 {
        let f = format!("%f{n}");
        let b = format!("%b{n}");
        assert_eq!(substitute(&f, &ctx()), format!("\x1B[3{n}m"));
        let bg = if n == 1 { 2 } else { n };
        assert_eq!(substitute(&b, &ctx()), format!("\x1B[4{bg}m"));
    }
}

// the variants of %r, %w and %t
#[test]
fn state_dependent_codes() {
    let resting = StatlineCtx {
        resting: true,
        meditating: true,
        ..ctx()
    };
    assert_eq!(substitute("[%r]", &resting), "[ (Resting) ]");
    let meditating = StatlineCtx {
        meditating: true,
        ..ctx()
    };
    assert_eq!(substitute("[%r]", &meditating), "[ (Meditating) ]");
    assert_eq!(substitute("[%r]", &ctx()), "[]");
    let warn = StatlineCtx {
        evil_warn: true,
        ..ctx()
    };
    assert_eq!(substitute("%w", &warn), "On");
    let title = StatlineCtx {
        title: Some("the sword"),
        ..ctx()
    };
    assert_eq!(substitute("<%t>", &title), "<the sword>");
}

// the quirks
#[test]
fn quirks() {
    let c = ctx();
    // %% prints two percent signs; so do %f% and %b%
    assert_eq!(substitute("100%%", &c), "100%%");
    assert_eq!(substitute("%f%", &c), "%%");
    assert_eq!(substitute("%b%", &c), "%%");
    // no code letter: the pending part comes back with the character
    assert_eq!(substitute("%z", &c), "%z");
    assert_eq!(substitute("%f9", &c), "%f9");
    assert_eq!(substitute("%b8", &c), "%b8");
    assert_eq!(substitute("%8", &c), "%8");
    // a code letter in the wrong state: the character alone
    assert_eq!(substitute("%5", &c), "5");
    assert_eq!(substitute("%fh", &c), "h");
    assert_eq!(substitute("%bB", &c), "B");
    // a template ending in a pending code loses it
    assert_eq!(substitute("ab%", &c), "ab");
    assert_eq!(substitute("ab%f", &c), "ab");
    // plain letters that are codes elsewhere are just text
    assert_eq!(substitute("hHf7", &c), "hHf7");
}

// needed minus exp, floored at 0; without a race or class record the X is copied
// alone
#[test]
fn exp_left_floors_at_zero() {
    let enough = StatlineCtx {
        exp: 250_000,
        ..ctx()
    };
    assert_eq!(substitute("%X", &enough), "0");
    let exact = StatlineCtx {
        exp: 200_000,
        ..ctx()
    };
    assert_eq!(substitute("%X", &exact), "0");
    let no_record = StatlineCtx {
        exp_needed: None,
        ..ctx()
    };
    assert_eq!(substitute("[%X]", &no_record), "[X]");
}

// %d makes %h coloured; a colour digit clears it
#[test]
fn hp_colour_after_d() {
    let c = StatlineCtx {
        hp: 10,
        maxhp: 40,
        palette: Palette::new(2).unwrap(),
        ..ctx()
    };
    let cyan = ascii(ANSI_CYAN);
    let low = ascii(ANSI_LTRED);
    assert_eq!(substitute("%h", &c), "10");
    assert_eq!(substitute("%d%h", &c), format!("{cyan}{low}10{cyan}"));
    assert_eq!(substitute("%d%f2%h", &c), format!("{cyan}\x1B[32m10"));
    // an attribute code does not clear it
    assert_eq!(
        substitute("%d%B%h", &c),
        format!("{cyan}\x1B[1m{low}10{cyan}")
    );
    // %d stays in force for later %h
    assert_eq!(
        substitute("%d%h/%h", &c),
        format!("{cyan}{low}10{cyan}/{low}10{cyan}")
    );
}

// SET STATLINE stores at most 60 characters
#[test]
fn template_cut_to_sixty() {
    let template = format!("{}{}", "0123456789".repeat(6), "abcdefghij");
    assert_eq!(template.len(), 70);
    let cut = cut_template(&template);
    assert_eq!(cut.len(), STATLINELEN);
    assert_eq!(substitute(cut, &ctx()), "0123456789".repeat(6));
}

// expansion stops once SUBSTMAX characters are written; the piece
// being copied finishes
#[test]
fn expansion_stops_at_substmax() {
    // plain text: exactly 196
    let long = "x".repeat(300);
    assert_eq!(substitute(&long, &ctx()).len(), SUBSTMAX);
    // 4 x 49 = 196: the fourth title reaches the bound, nothing after it
    let title = "t".repeat(49);
    let c = StatlineCtx {
        title: Some(&title),
        ..ctx()
    };
    assert_eq!(substitute("%t%t%t%t%tZ", &c), title.repeat(4));
    // 4 x 50 = 200: the fourth piece runs past 196 and still finishes
    let title = "t".repeat(50);
    let c = StatlineCtx {
        title: Some(&title),
        ..ctx()
    };
    let out = substitute("%t%t%t%t%tZ", &c);
    assert_eq!(out.len(), 200);
    // 195 plain characters, then a code: it is expanded in full
    let template = format!("{}%c", "y".repeat(195));
    assert_eq!(
        substitute(&template, &ctx()),
        format!("{}180750", "y".repeat(195))
    );
    // the pending "%" of a non-code is written but not counted
    let template = format!("{}%z%zQ", "y".repeat(194));
    assert_eq!(
        substitute(&template, &ctx()),
        format!("{}%z%z", "y".repeat(194))
    );
}

#[test]
fn snapshots() {
    let c = StatlineCtx {
        hp: 10,
        maxhp: 40,
        resting: true,
        palette: Palette::new(2).unwrap(),
        ..ctx()
    };
    let mut out = String::new();
    for template in [
        "[HP=%h/%H,MA=%m/%M]:",
        "[hp=%h/%H,MA=%m/%M,CASH=%c,WARNS=%w]:",
        "%d[%h/%H]%r%n%B%f1%b1Exp:%x Left:%X%N>",
        "%i loves %p sword; give %P %t. 100%% %z %5 %f9",
    ] {
        out.push_str(&format!(
            "{template}\n  -> {}|\n",
            substitute(template, &c).as_bytes().escape_ascii()
        ));
    }
    insta::assert_snapshot!(out);
}
