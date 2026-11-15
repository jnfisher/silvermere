//! Line-by-line entry for terminals without ANSI.

use silvermere_fsd::key::{BS, CR, CRSRUP, CTRL_G};
use silvermere_fsd::{Form, Mode, Outcome, State, Verdict};

use crate::support::{begun, key, key_with, show, typed};

const SPEC: &[u8] = b"NAME AGE(MAX=50)";
const SCREEN: &[u8] = b"Name: ??????\rAge: $$\r";

#[test]
fn each_field_is_prompted_with_its_answer() {
    let (f, out) = begun(SCREEN, SPEC, Mode::Linear, b"NAME=Al\0\0");
    assert_eq!(show(&out), show(b"\r\nName: Al"));
    assert_eq!(f.state(), State::LinePrompt);
}

#[test]
fn typing_rubs_out_the_answer_and_return_prompts_the_next_field() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::Linear, b"NAME=Al\0\0");
    let (out, _) = key(&mut f, u16::from(b'B'));
    assert_eq!(show(&out), show(b"\x08 \x08\x08 \x08B"));
    assert_eq!(f.state(), State::LineEditing);
    assert_eq!(typed(&mut f, "ob"), b"ob");
    let (out, _) = key(&mut f, BS);
    assert_eq!(out, b"\x08 \x08");
    let (out, _) = key(&mut f, CR);
    assert_eq!(show(&out), show(b"\rAge: "));
    assert_eq!(f.answer(0), b"Bo");
    assert_eq!((f.state(), f.current_field()), (State::LinePrompt, 1));
}

#[test]
fn a_rejected_entry_is_explained_and_prompted_again() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::Linear, b"\0");
    key(&mut f, CR);
    typed(&mut f, "60");
    let (out, _) = key(&mut f, CR);
    assert_eq!(
        show(&out),
        show(b"\r\n\r\nEnter no higher than 50\x07\r\n\rAge: ")
    );
    assert_eq!(f.answer(1), b"");
}

#[test]
fn return_on_the_last_field_prompts_it_again_and_ctrl_g_saves() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::Linear, b"\0");
    key(&mut f, CR);
    typed(&mut f, "42");
    // (sic) in line mode the last field comes back to itself rather than ending the session
    let (out, outcome) = key(&mut f, CR);
    assert_eq!(show(&out), show(b"\rAge: 42"));
    assert_eq!(outcome, Outcome::Continue);
    let (out, outcome) = key(&mut f, CTRL_G);
    assert_eq!(out, b"\r\n");
    assert_eq!(outcome, Outcome::Save);
    assert_eq!(f.answers(), b"NAME=\0AGE=42\0\0");
}

#[test]
fn up_goes_back_without_checking_the_entry() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::Linear, b"NAME=Al\0AGE=7\0\0");
    key(&mut f, CR);
    typed(&mut f, "99");
    let (out, _) = key(&mut f, CRSRUP);
    // the typing is rubbed out, the old answer kept, and the first field prompted
    assert_eq!(show(&out), show(b"\x08 \x08\x08 \x08\r\nName: Al"));
    assert_eq!(f.answer(1), b"7");
    assert_eq!(f.current_field(), 0);
    // there is no field before the first one
    let (out, _) = key(&mut f, CRSRUP);
    assert!(out.is_empty());
}

#[test]
fn space_shows_the_next_choice() {
    let (mut f, _) = begun(
        b"Hair: ?????\r",
        b"HAIR(ALT=Short ALT=Long MULTICHOICE)",
        Mode::Linear,
        b"HAIR=Long\0\0",
    );
    let (out, _) = key(&mut f, u16::from(b' '));
    assert_eq!(
        show(&out),
        show(b"\x08 \x08\x08 \x08\x08 \x08\x08 \x08Short")
    );
    assert_eq!(f.state(), State::LineEditing);
}

#[test]
fn save_on_the_last_field_ends_with_a_new_line() {
    // FSD.C line 2033: vfyadn() ends the session through xitfsd(), which starts a new
    // line in line mode (line 1102)
    let (mut f, _) = begun(b"Name: ????\rDone? ????", b"NAME DONE", Mode::Linear, b"\0");
    key(&mut f, CR);
    typed(&mut f, "S");
    let (out, outcome) = key(&mut f, CR);
    assert_eq!(outcome, Outcome::Save);
    assert_eq!(out, b"\r\n");
}

#[test]
fn a_silent_reject_on_the_last_field_rubs_out_the_entry() {
    // FSD.C lines 1143 and 1169-1171: unentr() on the entered field, though the cursor is
    // past the last one
    let (mut f, out) = begun(b"A: ??\rB: ????\r", b"A B", Mode::Linear, b"B=xy\0\0");
    assert_eq!(out, b"\r\nA: ");
    let mut verify = |_: &mut Form, n: usize, _: &mut Vec<u8>, _: &mut Vec<u8>| {
        let verdict = if n == 1 {
            Verdict::Reject
        } else {
            Verdict::Accept
        };
        (verdict, Vec::new())
    };
    let (out, _) = key_with(&mut f, CR, &mut verify);
    assert_eq!(show(&out), show(b"\rB: xy"));
    key_with(&mut f, u16::from(b'a'), &mut verify);
    key_with(&mut f, u16::from(b'b'), &mut verify);
    let (out, outcome) = key_with(&mut f, CR, &mut verify);
    assert_eq!(show(&out), show(b"\x07\x08 \x08\x08 \x08\rB: xy"));
    assert_eq!(outcome, Outcome::Continue);
    assert_eq!((f.state(), f.current_field()), (State::LinePrompt, 1));
}
