//! Reading answers back: by field, by name and as a choice.

use silvermere_fsd::{Form, Mode};

const SPEC: &[u8] =
    b"HAIR(ALT=Short ALT=Long ALT=Bald MULTICHOICE) EYES(ALT=Blue ALT=Brown MULTICHOICE) NAME";
const SCREEN: &[u8] = b"?????? ?????? ??????";

fn form(answers: &[u8]) -> Form {
    Form::prepare(SCREEN, SPEC, Mode::FullScreen, answers)
}

#[test]
fn ordinal_is_the_unique_matching_choice() {
    let mut f = form(b"HAIR=Long\0EYES=Br\0NAME=Al\0\0");
    assert_eq!(f.ordinal(0), Some(1));
    // a unique prefix counts, and the answer is completed to the choice
    assert_eq!(f.ordinal(1), Some(1));
    assert_eq!(f.answer(1), b"Brown");
    assert_eq!(f.answers(), b"HAIR=Long\0EYES=Brown\0NAME=Al\0\0");
    // no choices at all
    assert_eq!(f.ordinal(2), None);
}

#[test]
fn ordinal_ignores_case_and_blanks_but_not_ambiguity() {
    let mut f = form(b"HAIR=b a\0EYES=B\0\0");
    assert_eq!(f.ordinal(0), Some(2));
    assert_eq!(f.answer(0), b"Bald");
    assert_eq!(f.ordinal(1), None);
    assert_eq!(f.answer(1), b"B");
}

#[test]
fn answers_by_name() {
    let f = form(b"NAME=Al\0HAIR=Bald\0\0");
    assert_eq!(f.answer_named("NAME"), b"Al");
    assert_eq!(f.answer_named("name"), b"Al");
    assert_eq!(f.answer_named("EYES"), b"");
    assert_eq!(f.answer_named("NOPE"), b"");
    // the first of duplicate defaults wins
    let f = form(b"NAME=One\0NAME=Two\0\0");
    assert_eq!(f.answer(2), b"One");
}
