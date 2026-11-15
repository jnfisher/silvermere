//! Properties over random synthetic full-screen forms and keystrokes.

use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::select;
use silvermere_fsd::key::{
    BAKTAB, BS, CR, CRSRDN, CRSRLF, CRSRRT, CRSRUP, CTRL_F, CTRL_U, DEL, END, HOME, RUBOUT, TAB,
};
use silvermere_fsd::{Form, Mode, Outcome};

use crate::support::key;

/// One field: its options and its template markers.
fn field() -> impl Strategy<Value = (String, String)> {
    let opts = prop_oneof![
        Just(String::new()),
        Just("()".to_string()),
        Just("(MIN=10, MAX=500)".to_string()),
        Just("(MIN=3)".to_string()),
        Just("(ALT=RED ALT=ROSE ALT=BLUE MULTICHOICE)".to_string()),
        Just("(ALT=SAVE ALT=EDIT ALT=QUIT MULTICHOICE)".to_string()),
    ];
    let markers = prop_oneof!["[?]{2,8}", "[$]{2,6}"];
    (opts, markers)
}

fn keys() -> impl Strategy<Value = Vec<u16>> {
    let k = prop_oneof![
        select(b"aRrOoBbSsEeQq\xE90123456789- ".to_vec()).prop_map(u16::from),
        select(vec![
            BS, CR, TAB, BAKTAB, CTRL_U, CTRL_F, RUBOUT, DEL, CRSRUP, CRSRDN, CRSRLF, CRSRRT, HOME,
            END,
        ]),
    ];
    vec(k, 0..80)
}

proptest! {
    #[test]
    fn answers_stay_well_formed(
        fields in vec(field(), 1..6),
        keys in keys(),
        full_screen in any::<bool>(),
    ) {
        let mode = if full_screen { Mode::FullScreen } else { Mode::Linear };
        let mut fldspc = String::new();
        let mut template = String::from("\x1B[2J");
        for (i, (opts, markers)) in fields.iter().enumerate() {
            fldspc.push_str(&format!("F{i}{opts} "));
            template.push_str(&format!("\rField {i}: {markers}"));
        }
        template.push_str("\r!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
        let mut f = Form::prepare(template.as_bytes(), fldspc.as_bytes(), mode, b"\0");
        let mut out = Vec::new();
        f.background(&mut out);
        f.begin(&mut out);
        for k in keys {
            let (_, outcome) = key(&mut f, k);
            let answers = f.answers();
            prop_assert!(answers.ends_with(b"\0\0"));
            let entries: Vec<&[u8]> = answers[..answers.len() - 1].split(|&b| b == 0)
                .filter(|e| !e.is_empty()).collect();
            prop_assert_eq!(entries.len(), fields.len());
            for (i, entry) in entries.iter().enumerate() {
                let name = format!("F{i}=");
                prop_assert!(entry.starts_with(name.as_bytes()));
                prop_assert_eq!(f.answer(i), f.answer_named(&format!("F{i}")));
                prop_assert_eq!(&entry[name.len()..], f.answer(i));
            }
            if outcome != Outcome::Continue {
                break;
            }
            prop_assert!(f.current_field() < fields.len());
        }
    }
}
