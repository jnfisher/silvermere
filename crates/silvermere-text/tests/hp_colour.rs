//! `get_hp_colour`: the hit point colour thresholds, integer
//! division as written.

use silvermere_text::colour::{CLRTBL, ColRow, Palette, get_hp_colour, hp_colour_row};

// hp > maxhp/2 healthy, hp > maxhp/4 hurt, else low
#[test]
fn thresholds_at_maxhp_56() {
    let cases: [(i32, i32, ColRow); 10] = [
        (56, 56, ColRow::PromptVal),
        (29, 56, ColRow::PromptVal), // just above half
        (28, 56, ColRow::HpHurt),    // exactly half: hurt
        (15, 56, ColRow::HpHurt),    // just above a quarter
        (14, 56, ColRow::HpLow),     // exactly a quarter: low
        (1, 56, ColRow::HpLow),
        (0, 56, ColRow::HpLow),
        (-1, 56, ColRow::HpLow),     // unconscious
        (-107, 56, ColRow::HpLow),   // deep negative, still low
        (29, 57, ColRow::PromptVal), // 57/2 = 28 (integer division), so 29 is healthy
    ];
    for (hp, maxhp, row) in cases {
        assert_eq!(hp_colour_row(hp, maxhp), row, "hp {hp} of {maxhp}");
    }
}

// odd maxima round down before the compare
#[test]
fn integer_division_rounds_the_boundaries_down() {
    // 7/2 = 3: 4 is healthy, 3 is hurt; 7/4 = 1: 2 is hurt, 1 is low
    assert_eq!(hp_colour_row(4, 7), ColRow::PromptVal);
    assert_eq!(hp_colour_row(3, 7), ColRow::HpHurt);
    assert_eq!(hp_colour_row(2, 7), ColRow::HpHurt);
    assert_eq!(hp_colour_row(1, 7), ColRow::HpLow);
    // a maximum of 0 (never in play): everything at or below 0 is low
    assert_eq!(hp_colour_row(0, 0), ColRow::HpLow);
    assert_eq!(hp_colour_row(1, 0), ColRow::PromptVal);
}

// the colours are the colour-table rows COL_PROMPTVAL, COL_HPHURT, COL_HPLOW
#[test]
fn colour_comes_from_the_table_in_the_players_palette() {
    for p in 0..Palette::COUNT {
        let palette = Palette::new(p).unwrap();
        assert_eq!(
            get_hp_colour(29, 56, palette),
            CLRTBL[3][usize::from(p)],
            "palette {p} healthy"
        );
        assert_eq!(
            get_hp_colour(28, 56, palette),
            CLRTBL[4][usize::from(p)],
            "palette {p} hurt"
        );
        assert_eq!(
            get_hp_colour(14, 56, palette),
            CLRTBL[5][usize::from(p)],
            "palette {p} low"
        );
    }
    // palettes 0 and 1 show healthy hit points with no colour change (NOCOLOR)
    assert_eq!(get_hp_colour(56, 56, Palette::new(0).unwrap()), b"");
    assert_eq!(
        get_hp_colour(56, 56, Palette::new(2).unwrap()),
        b"\x1B[1;36m"
    );
    assert_eq!(
        get_hp_colour(1, 56, Palette::new(3).unwrap()),
        b"\x1B[1;31m"
    );
}
