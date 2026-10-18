//! `raw_to_lines` against every T-005 capture: each capture's plain text is its raw bytes
//! split into lines, plus one marker per send.

use silvermere_testkit::captures::{Capture, captures_dir, py_repr, text_to_lines};
use silvermere_testkit::raw_to_lines;

#[test]
fn every_capture_plain_txt_is_raw_to_lines_plus_markers() {
    let captures = Capture::all(captures_dir()).unwrap();
    if captures.is_empty() {
        eprintln!("skipped: no captures under tests/captures/");
        return;
    }
    assert_eq!(captures.len(), 12, "the twelve T-005 scenarios");
    for c in &captures {
        let (lines, found) = c.plain_without_markers();
        assert_eq!(lines, raw_to_lines(&c.raw), "{}", c.name);
        assert_eq!(found, c.sends().len(), "{}: a marker per send", c.name);
        assert!(found > 0, "{}", c.name);
    }
}

#[test]
fn line_splitting_rules() {
    let split = |s: &str| text_to_lines(s);
    assert_eq!(
        split("a\x1b[79D\x1b[Kb"),
        ["a", "b"],
        "CLRLINE breaks a line"
    );
    assert_eq!(split("a\x01b"), ["a", "b"], "^A breaks a line");
    assert_eq!(split("a\r\nb\rc\nd"), ["a", "b", "c", "d"]);
    assert_eq!(
        split("\x1b[1;31mred\x1b[0m"),
        ["red"],
        "other ANSI is dropped"
    );
    assert_eq!(
        split("nJ\x08orth"),
        ["north"],
        "backspace removes the letter before it"
    );
    assert_eq!(
        split("-\x08\\\x08|\x08ok"),
        ["ok"],
        "spinner frames fold away"
    );
    assert_eq!(split("ding\x07"), ["ding"], "bell dropped");
}

#[test]
fn python_repr_of_sent_text() {
    assert_eq!(py_repr("n"), "'n'");
    assert_eq!(py_repr(""), "''");
    assert_eq!(py_repr("\r"), "'\\r'");
    assert_eq!(py_repr("it's"), "\"it's\"");
    assert_eq!(py_repr("\x1b[A"), "'\\x1b[A'");
}

#[test]
fn script_masks_compile_and_blank_the_build_stamp() {
    use silvermere_testkit::captures::apply_script_masks;
    let captures = Capture::all(captures_dir()).unwrap();
    if captures.is_empty() {
        eprintln!("skipped: no captures under tests/captures/");
        return;
    }
    let mut stamps = 0;
    for c in &captures {
        let masks = c.script_masks();
        for m in &masks {
            assert!(
                silvermere_testkit::patterns::Rx::new(&m.regex).is_ok(),
                "{}: {:?} does not compile",
                c.name,
                m.regex
            );
        }
        let text = c.plain.join("\n");
        let masked = apply_script_masks(&text, &masks);
        assert!(!masked.contains("Dec 30 2005 14:20:05"), "{}", c.name);
        stamps += usize::from(text.contains("Dec 30 2005 14:20:05"));
    }
    assert!(stamps > 0, "at least one capture shows the build stamp");
}
