//! Full-screen entry: keystrokes with the cursor on a field and while typing in one,
//! checked by the standard verify routine.

use silvermere_fsd::key::{BS, CR, CRSRDN, CRSRLF, CRSRRT, CRSRUP, CTRL_G, END, ESC, HOME, TAB};
use silvermere_fsd::{Mode, Outcome, State};

use crate::support::{begun, key, show, typed};

const SPEC: &[u8] = b"NAME NUM(MIN=10, MAX=99) DONE(ALT=SAVE ALT=QUIT MULTICHOICE)";
const SCREEN: &[u8] = b"Name: ??????\rNum:  $$$$\r!!!!!!!!!!!!!!!!!!!!\rDone: ????";
const ANSWERS: &[u8] = b"NAME=Bob\0NUM=42\0DONE=SAVE\0\0";

/// Cursor-attribute and normal-attribute colour sequences.
const REV: &str = "\x1B[0;7m";
const NORM: &str = "\x1B[0;37;40m";

fn expect(out: &[u8], parts: &[&str]) {
    assert_eq!(show(out), show(parts.concat().as_bytes()));
}

#[test]
fn begin_puts_the_cursor_on_the_first_field() {
    let (f, out) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    expect(&out, &["\x1B[1;7f", REV, "Bob   ", "\x1B[1;7f"]);
    assert_eq!(f.state(), State::FieldSelected);
}

#[test]
fn typing_replaces_the_answer_and_return_moves_on() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    let (out, _) = key(&mut f, u16::from(b'J'));
    // the field is blanked in the cursor attribute, then the key echoed
    expect(&out, &["\x1B[1;7f", REV, "      ", "\x1B[1;7f", "J"]);
    assert_eq!(f.state(), State::Editing);
    assert_eq!(typed(&mut f, "oe"), b"oe");
    let (out, _) = key(&mut f, BS);
    expect(&out, &["\x08", " \x08"]);
    let (out, outcome) = key(&mut f, CR);
    expect(
        &out,
        &[
            "\x1B[2;7f",
            // the old field in its own attribute, the new one in the cursor's
            "\x1B[1;7f",
            NORM,
            "Jo    ",
            "\x1B[2;7f",
            REV,
            "  42",
            "\x1B[2;7f",
        ],
    );
    assert_eq!(outcome, Outcome::Continue);
    assert_eq!(f.answer(0), b"Jo");
    assert!(f.is_changed(0) && !f.is_changed(1));
    assert_eq!((f.state(), f.current_field()), (State::FieldSelected, 1));
}

#[test]
fn a_failed_minimum_is_announced_in_the_help_area_until_the_next_key() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    key(&mut f, TAB);
    typed(&mut f, "5");
    let (out, _) = key(&mut f, CR);
    expect(
        &out,
        &[
            "\x1B[4;7f",
            "\x1B[3;1f",
            NORM,
            "Enter at least 10\x07",
            "\x1B[2;7f",
            REV,
            "  42",
            "\x1B[2;7f",
        ],
    );
    assert_eq!((f.state(), f.current_field()), (State::MessageShown, 1));
    assert_eq!(f.answer(1), b"42");
    // the next key wipes the message first
    let (out, _) = key(&mut f, u16::from(b'7'));
    expect(
        &out,
        &[
            "\x1B[3;1f",
            NORM,
            &" ".repeat(20),
            "\x1B[2;7f",
            "\x1B[2;7f",
            REV,
            "    ",
            "\x1B[2;7f",
            "7",
        ],
    );
    typed(&mut f, "7");
    let (out, _) = key(&mut f, CR);
    expect(
        &out,
        &[
            "\x1B[4;7f",
            "\x1B[2;7f",
            NORM,
            "  77",
            "\x1B[4;7f",
            REV,
            "SAVE",
            "\x1B[4;7f",
        ],
    );
    assert_eq!(f.answer(1), b"77");
}

#[test]
fn space_cycles_a_multiple_choice_field_and_return_quits_on_quit() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    key(&mut f, CRSRUP);
    assert_eq!(f.current_field(), 2);
    let (out, _) = key(&mut f, u16::from(b' '));
    expect(&out, &["\x1B[4;7f", REV, "    ", "\x1B[4;7f", "QUIT"]);
    // and round again
    let (out, _) = key(&mut f, u16::from(b' '));
    expect(&out, &["\x1B[4;7f", REV, "    ", "\x1B[4;7f", "SAVE"]);
    key(&mut f, u16::from(b' '));
    let (out, outcome) = key(&mut f, CR);
    expect(&out, &["\x1B[1;7f", "\x1B[4;7f", NORM, "QUIT"]);
    assert_eq!(outcome, Outcome::Quit);
    assert_eq!(f.state(), State::Quit);
    assert_eq!(f.answers(), b"NAME=Bob\0NUM=42\0DONE=QUIT\0\0");
}

#[test]
fn save_on_the_last_field_saves() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    key(&mut f, CRSRUP);
    let (_, outcome) = key(&mut f, CR);
    assert_eq!(outcome, Outcome::Save);
}

#[test]
fn letters_pick_a_choice_and_letters_after_a_completion_start_again() {
    let spec = b"COLOR(ALT=RED ALT=ROSE ALT=BLUE MULTICHOICE)";
    let (mut f, _) = begun(b"Colour: ?????", spec, Mode::FullScreen, b"COLOR=RED\0\0");
    let goto = "\x1B[1;9f";
    // "R" could be RED or ROSE: taken as typed
    let (out, _) = key(&mut f, u16::from(b'R'));
    expect(
        &out,
        &[goto, REV, "     ", goto, goto, REV, "     ", goto, "R"],
    );
    // "RO" is only ROSE: completed, cursor back over the padding
    let (out, _) = key(&mut f, u16::from(b'O'));
    expect(&out, &[goto, REV, "     ", goto, "ROSE ", "\x1B[D"]);
    // after a completion a letter starts a new search: "B" is BLUE
    let (out, _) = key(&mut f, u16::from(b'B'));
    expect(&out, &[goto, REV, "     ", goto, "BLUE ", "\x1B[D"]);
    // no choice starts with "E": ignored
    let (out, _) = key(&mut f, u16::from(b'E'));
    assert!(out.is_empty());
    key(&mut f, CR);
    assert_eq!(f.answer(0), b"BLUE");
    assert_eq!(f.ordinal(0), Some(2));
}

#[test]
fn an_ambiguous_prefix_is_refused() {
    let spec = b"COLOR(ALT=RED ALT=ROSE ALT=BLUE MULTICHOICE)";
    let (mut f, _) = begun(b"Colour: ?????", spec, Mode::FullScreen, b"\0");
    typed(&mut f, "R");
    // "R" matches two choices: rejected with the choices listed (no help area)
    let (out, _) = key(&mut f, CR);
    assert!(out.contains(&0x07));
    assert_eq!(f.answer(0), b"");
}

#[test]
fn arrows_move_within_the_answer_and_home_end_jump() {
    let (mut f, _) = begun(b"N: ??????", b"N", Mode::FullScreen, b"N=abcd\0\0");
    // right arrow enters the field showing the answer, cursor after its first character
    let (out, _) = key(&mut f, CRSRRT);
    expect(&out, &["\x1B[1;4f", REV, "abcd  ", "\x1B[1;4f", "a"]);
    assert_eq!(f.state(), State::Editing);
    let (out, _) = key(&mut f, CRSRRT);
    assert_eq!(out, b"\x1B[C");
    let (out, _) = key(&mut f, CRSRLF);
    assert_eq!(out, b"\x1B[D");
    let (out, _) = key(&mut f, END);
    assert_eq!(out, b"\x1B[3C");
    let (out, _) = key(&mut f, HOME);
    assert_eq!(out, b"\x1B[1;4f");
    // typing overwrites at the cursor
    typed(&mut f, "X");
    key(&mut f, CR);
    assert_eq!(f.answer(0), b"Xbcd");
}

#[test]
fn escape_after_changes_asks_first() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    typed(&mut f, "Al");
    key(&mut f, CR);
    let (out, outcome) = key(&mut f, ESC);
    assert_eq!(outcome, Outcome::Continue);
    let msg = "Are you sure?  Enter";
    assert!(show(&out).contains(msg), "{}", show(&out));
    assert_eq!(f.current_field(), 2);
    let (_, outcome) = key(&mut f, ESC);
    assert_eq!(outcome, Outcome::Quit);
}

#[test]
fn escape_without_changes_quits_at_once() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    let (out, outcome) = key(&mut f, ESC);
    assert_eq!(outcome, Outcome::Quit);
    // the cursor stays put, and the field is turned off
    expect(&out, &["\x1B[1;7f", "\x1B[1;7f", NORM, "Bob   "]);
}

#[test]
fn ctrl_g_saves_from_any_field() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    typed(&mut f, "Zed");
    let (_, outcome) = key(&mut f, CTRL_G);
    assert_eq!(outcome, Outcome::Save);
    assert_eq!(f.answer(0), b"Zed");
}

#[test]
fn cursor_keys_wrap_and_skip_avoided_fields() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    f.avoid(1);
    key(&mut f, CRSRDN);
    assert_eq!(f.current_field(), 2);
    key(&mut f, CRSRDN);
    assert_eq!(f.current_field(), 0);
    key(&mut f, CRSRUP);
    assert_eq!(f.current_field(), 2);
}

#[test]
fn a_second_hop_before_the_output_drains_is_shuffled_later() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, ANSWERS);
    let mut out = Vec::new();
    f.key(CRSRDN, &mut out);
    expect(
        &out,
        &[
            "\x1B[1;7f",
            NORM,
            "Bob   ",
            "\x1B[2;7f",
            REV,
            "  42",
            "\x1B[2;7f",
        ],
    );
    // output not yet sent: only the cursor moves
    let mut out = Vec::new();
    f.key(CRSRDN, &mut out);
    assert_eq!(out, b"\x1B[4;7f");
    let mut out = Vec::new();
    f.output_sent(&mut out);
    expect(
        &out,
        &[
            "\x1B[2;7f",
            NORM,
            "  42",
            "\x1B[4;7f",
            REV,
            "SAVE",
            "\x1B[4;7f",
        ],
    );
}

#[test]
fn editing_keys_show_a_number_left_justified() {
    // FSD.C line 1682: shofld(fsdscb->crsatr,-1), so the entry starts where it is shown
    let (mut f, _) = begun(
        b"N: $$$$",
        b"N(MIN=10, MAX=250)",
        Mode::FullScreen,
        b"N=42\0\0",
    );
    let (out, _) = key(&mut f, CRSRRT);
    expect(&out, &["\x1B[1;4f", REV, "42  ", "\x1B[1;4f", "4"]);
    let (out, _) = key(&mut f, u16::from(b'7'));
    assert_eq!(out, b"7");
    let (out, _) = key(&mut f, CR);
    // the one field again, right-justified
    expect(&out, &["\x1B[1;4f", "\x1B[1;4f", REV, "  47", "\x1B[1;4f"]);
    assert_eq!(f.answer(0), b"47");
    let (out, _) = key(&mut f, BS);
    expect(&out, &["\x1B[1;4f", REV, "47  ", "\x1B[1;4f"]);
    assert_eq!(f.state(), State::Editing);
}
