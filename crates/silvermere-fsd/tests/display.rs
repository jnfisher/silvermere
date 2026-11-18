//! Drawing the background.

use silvermere_fsd::{Form, Mode};

use crate::support::show;

const CLEAR: &[u8] = b"\x1B[0m\x1B[2J\x1B[0m";

fn background(f: &Form) -> Vec<u8> {
    let mut out = Vec::new();
    f.background(&mut out);
    assert!(out.starts_with(CLEAR));
    out.split_off(CLEAR.len())
}

#[test]
fn fields_are_justified_by_type_and_the_help_area_blanked() {
    let f = Form::prepare(
        b"Name: ??????\r\nNum:  $$$$\r\n!!!!!!!!!!!!!!!!!!!!\r\nDone: ????",
        b"NAME NUM DONE",
        Mode::FullScreen,
        b"NAME=Bob\0NUM=42\0DONE=SAVE\0\0",
    );
    assert_eq!(
        show(&background(&f)),
        show(b"Name: Bob   \r\nNum:    42\r\n                    \r\nDone: SAVE")
    );
}

#[test]
fn avoided_fields_show_as_blanks() {
    let mut f = Form::prepare(
        b"A: ??? B: ???.",
        b"A B",
        Mode::FullScreen,
        b"A=abc\0B=xyz\0\0",
    );
    f.avoid(0);
    assert!(f.is_avoided(0) && !f.is_avoided(1));
    assert_eq!(show(&background(&f)), show(b"A:     B: xyz."));
}

#[test]
fn template_bytes_pass_through() {
    let f = Form::prepare(
        b"\xC4\xB3 ??\x1B[1;31m\xAE",
        b"A",
        Mode::FullScreen,
        b"A=\xE9x\0\0",
    );
    assert_eq!(background(&f), b"\xC4\xB3 \xE9x\x1B[1;31m\xAE");
}
