//! Reading field specifications and templates, seen through a prepared form.

use silvermere_fsd::{Form, Mode};

const SPEC: &[u8] = b"NAME NUM(MIN=10, MAX=99) DONE(ALT=SAVE ALT=QUIT MULTICHOICE)";
const SCREEN: &[u8] = b"Name: ??????\rNum:  $$$$\r!!!!!!!!!!!!!!!!!!!!\rDone: ????";

fn background(template: &[u8], spec: &[u8], answers: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    Form::prepare(template, spec, Mode::FullScreen, answers).background(&mut out);
    out
}

#[test]
fn defaults_are_cut_to_the_field_width() {
    let f = Form::prepare(b"A: ???", b"A B", Mode::Linear, b"B=xyz\0A=abcdef\0\0");
    assert_eq!(f.field_count(), 2);
    assert_eq!(f.answer(0), b"abc");
    // a field the template lacks has no width, so no answer
    assert_eq!(f.answer(1), b"");
    assert_eq!(f.answers(), b"A=abc\0B=\0\0");
}

#[test]
fn full_screen_scan_tracks_the_bottom_row() {
    let maxy = |template: &[u8], mode| Form::prepare(template, SPEC, mode, b"\0").maxy();
    assert_eq!(maxy(SCREEN, Mode::FullScreen), 3);
    assert_eq!(maxy(b"\x1B[20;1HX: ??", Mode::FullScreen), 19);
    // line mode does not track the screen
    assert_eq!(maxy(SCREEN, Mode::Linear), 0);
}

#[test]
fn single_markers_and_other_markup_are_text() {
    let out = background(b"# $ ? ! Y/N ##: ?? $$", b"A", b"A=12\0\0");
    // only the "??" is a field; the "$$" is past the specification's fields
    assert!(out.ends_with(b"\x1B[0m# $ ? ! Y/N ##: 12 $$"));
}

#[test]
fn the_template_and_specification_stop_at_a_nul() {
    let out = background(b"A: ??\0B: ??", b"A\0B", b"A=1\0B=2\0\0");
    assert!(out.ends_with(b"\x1B[0mA: 1 "));
}
