//! The verify routine's verdicts as the session acts on them.

use silvermere_fsd::key::CR;
use silvermere_fsd::{Form, Mode, State, Verdict};

use crate::support::{begun, key_with, show};

const SPEC: &[u8] = b"A(MAX=50) B";
const SCREEN: &[u8] = b"A: $$\rB: ????\r!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!";

fn enter(f: &mut Form, text: &str, verdict: Verdict, msg: &str) -> Vec<u8> {
    let mut verify = |_: &mut Form, _: usize, _: &mut Vec<u8>, _: &mut Vec<u8>| {
        (verdict, msg.as_bytes().to_vec())
    };
    for b in text.bytes() {
        key_with(f, u16::from(b), &mut verify);
    }
    key_with(f, CR, &mut verify).0
}

#[test]
fn accept_stores_the_entry_unchecked() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    enter(&mut f, "77", Verdict::Accept, "");
    assert_eq!(f.answer(0), b"77");
    assert_eq!(f.current_field(), 1);
}

#[test]
fn check_applies_the_field_options() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    let out = enter(&mut f, "77", Verdict::Check, "");
    assert!(
        show(&out).contains("Enter no higher than 50\\x07"),
        "{}",
        show(&out)
    );
    assert_eq!(f.answer(0), b"12");
    assert_eq!((f.state(), f.current_field()), (State::MessageShown, 0));
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    enter(&mut f, "33", Verdict::Check, "");
    assert_eq!(f.answer(0), b"33");
}

#[test]
fn reject_keeps_the_old_answer_and_the_field() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    let out = enter(&mut f, "33", Verdict::Reject, "");
    // no message: just the alarm, then the field shown again under the cursor
    assert_eq!(
        show(&out),
        show(b"\x1B[2;4f\x07\x1B[1;4f\x1B[0;7m12\x1B[1;4f")
    );
    assert_eq!(f.answer(0), b"12");
    assert_eq!((f.state(), f.current_field()), (State::FieldSelected, 0));
    // with a message: shown in the help area, cut to its width
    let out = enter(
        &mut f,
        "33",
        Verdict::Reject,
        "No, not that, not ever, never ever",
    );
    assert!(
        show(&out).contains("\\x1b[3;1f\\x1b[0;37;40mNo, not that, not ever, never \\x07"),
        "{}",
        show(&out)
    );
    assert_eq!(f.state(), State::MessageShown);
}

#[test]
fn revert_keeps_the_old_answer_and_moves_on() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    enter(&mut f, "33", Verdict::Revert, "");
    assert_eq!(f.answer(0), b"12");
    assert_eq!(f.current_field(), 1);
}

#[test]
fn a_message_with_an_accepted_entry_is_shown_without_the_alarm() {
    let (mut f, _) = begun(SCREEN, SPEC, Mode::FullScreen, b"A=12\0\0");
    let out = enter(&mut f, "33", Verdict::Accept, "Noted");
    assert!(
        show(&out).contains("\\x1b[3;1f\\x1b[0;37;40mNoted\\x1b"),
        "{}",
        show(&out)
    );
    assert!(!out.contains(&0x07));
    assert_eq!(f.answer(0), b"33");
}

#[test]
fn the_routine_may_change_the_answer_and_write_output() {
    let (mut f, _) = begun(b"A: ??\rB: ????", b"A B", Mode::FullScreen, b"\0");
    let mut seen = Vec::new();
    let mut verify = |f: &mut Form, n: usize, a: &mut Vec<u8>, o: &mut Vec<u8>| {
        seen.push((n, a.clone()));
        a.make_ascii_uppercase();
        o.extend_from_slice(b"<v>");
        f.done_at_end(n, a, o)
    };
    key_with(&mut f, CR, &mut verify);
    for b in b"hi" {
        key_with(&mut f, u16::from(*b), &mut verify);
    }
    let (out, _) = key_with(&mut f, CR, &mut verify);
    assert!(out.starts_with(b"\x1B[1;4f<v>"), "{}", show(&out));
    assert_eq!(f.answer(1), b"HI");
    assert_eq!(f.answer_named("B"), b"HI");
    assert_eq!(seen, [(0, Vec::new()), (1, b"hi".to_vec())]);
}

#[test]
fn blanks_are_dropped_before_the_check() {
    let (mut f, _) = begun(b"A: $$$$ B: ????", b"A B", Mode::FullScreen, b"A= 4\0\0");
    let mut seen = Vec::new();
    let mut verify = |_: &mut Form, _: usize, a: &mut Vec<u8>, _: &mut Vec<u8>| {
        seen.push(a.clone());
        (Verdict::Accept, Vec::new())
    };
    // a number loses leading blanks too, text only trailing ones
    for k in [CR, u16::from(b' '), u16::from(b'x'), u16::from(b' '), CR] {
        key_with(&mut f, k, &mut verify);
    }
    assert_eq!(seen, [b"4".to_vec(), b" x".to_vec()]);
}

#[test]
fn bytes_above_ascii_are_kept() {
    // FSD.C keeps answers as C strings: any byte but NUL
    let (mut f, _) = begun(b"A: ????\rB: ??", b"A B", Mode::FullScreen, b"A=\x82t\0\0");
    assert_eq!(f.answer(0), b"\x82t");
    let mut seen = Vec::new();
    let mut verify = |f: &mut Form, n: usize, a: &mut Vec<u8>, o: &mut Vec<u8>| {
        seen.push(a.clone());
        f.done_at_end(n, a, o)
    };
    let (out, _) = key_with(&mut f, 0xE9, &mut verify);
    assert!(out.ends_with(b"\xE9"), "{}", show(&out));
    key_with(&mut f, u16::from(b'x'), &mut verify);
    key_with(&mut f, CR, &mut verify);
    assert_eq!(seen, [b"\xE9x".to_vec()]);
    assert_eq!(f.answer(0), b"\xE9x");
    assert_eq!(f.answers(), b"A=\xE9x\0B=\0\0");
}
