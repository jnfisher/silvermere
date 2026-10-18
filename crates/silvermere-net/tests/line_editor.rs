//! Line discipline, pure: no sockets.

use proptest::prelude::*;
use silvermere_net::line_editor::{LINE_LIMIT, PENDING_CAP};
use silvermere_net::{EchoMode, EditorEvent, LineEditor, TelnetCodec};

/// Feeds raw (post-telnet) bytes; returns the events and the echo.
fn edit(editor: &mut LineEditor, bytes: &[u8]) -> (Vec<EditorEvent>, Vec<u8>) {
    let mut echo = Vec::new();
    editor.feed_all(bytes, &mut echo);
    (std::iter::from_fn(|| editor.next_event()).collect(), echo)
}

fn lines(bytes: &[u8]) -> Vec<EditorEvent> {
    edit(&mut LineEditor::new(), bytes).0
}

fn line(s: &[u8]) -> EditorEvent {
    EditorEvent::Line(s.to_vec())
}

/// Telnet-decodes then edits, as the session task does.
fn wire(bytes: &[u8]) -> Vec<EditorEvent> {
    let mut codec = TelnetCodec::new();
    let (mut data, mut reply) = (Vec::new(), Vec::new());
    codec.decode(bytes, &mut data, &mut reply);
    lines(&data)
}

#[test]
fn cr_lf_ends_a_line() {
    assert_eq!(lines(b"n\r\n"), vec![line(b"n")]);
}

#[test]
fn cr_nul_ends_a_line() {
    assert_eq!(lines(b"n\r\0"), vec![line(b"n")]);
}

#[test]
fn bare_lf_ends_a_line() {
    assert_eq!(lines(b"n\n"), vec![line(b"n")]);
}

#[test]
fn bare_cr_ends_a_line_and_next_line_is_intact() {
    assert_eq!(lines(b"a\rb\r"), vec![line(b"a"), line(b"b")]);
}

#[test]
fn backspace_erases_and_echoes_rubout() {
    let (events, echo) = edit(&mut LineEditor::new(), b"ab\x08c\r");
    assert_eq!(events, vec![line(b"ac")]);
    assert_eq!(echo, b"ab\x08 \x08c\r\n");
}

#[test]
fn del_erases_like_backspace() {
    assert_eq!(lines(b"ab\x7fc\r"), vec![line(b"ac")]);
}

#[test]
fn backspace_on_empty_line_does_nothing() {
    let (events, echo) = edit(&mut LineEditor::new(), b"\x08\x08x\r");
    assert_eq!(events, vec![line(b"x")]);
    assert_eq!(echo, b"x\r\n");
}

// The password prompt echoes `*` for up to 8 bytes.
#[test]
fn secure_echo_prints_stars() {
    let mut ed = LineEditor::new();
    ed.set_echo(EchoMode::Secure { ch: b'*', width: 8 });
    let (events, echo) = edit(&mut ed, b"secret\r");
    assert_eq!(events, vec![line(b"secret")]);
    assert_eq!(echo, b"******\r\n");
}

// Secure echo: nothing is echoed past the width.
#[test]
fn secure_echo_stops_at_width() {
    let mut ed = LineEditor::new();
    ed.set_echo(EchoMode::Secure { ch: b'*', width: 8 });
    let (events, echo) = edit(&mut ed, b"abcdefghijkl\r");
    assert_eq!(events, vec![line(b"abcdefghijkl")]);
    assert_eq!(echo, b"********\r\n");
}

#[test]
fn secure_echo_lasts_one_line() {
    let mut ed = LineEditor::new();
    ed.set_echo(EchoMode::Secure { ch: b'*', width: 8 });
    edit(&mut ed, b"pw\r");
    assert_eq!(ed.echo(), EchoMode::On);
    assert_eq!(edit(&mut ed, b"x").1, b"x");
}

#[test]
fn echo_off_prints_nothing() {
    let mut ed = LineEditor::new();
    ed.set_echo(EchoMode::Off);
    let (events, echo) = edit(&mut ed, b"ab\x08\r");
    assert_eq!(events, vec![line(b"a")]);
    assert!(echo.is_empty());
}

// The game's line limit is 254 bytes.
#[test]
fn line_of_254_bytes_is_accepted() {
    let mut input = vec![b'x'; LINE_LIMIT];
    input.push(b'\r');
    assert_eq!(lines(&input), vec![EditorEvent::Line(vec![b'x'; 254])]);
}

// The 255th byte overflows the line limit and the host discards the line.
#[test]
fn byte_255_discards_the_line() {
    let mut ed = LineEditor::new();
    let (events, _) = edit(&mut ed, &[b'x'; 255]);
    assert_eq!(events, vec![EditorEvent::LineTooLong]);
    assert!(ed.buffer().is_empty());
}

#[test]
fn control_bytes_are_dropped() {
    assert_eq!(
        lines(b"a\x01\x02\x03\x04\x05\x06\x07b\r"),
        vec![line(b"ab")]
    );
}

#[test]
fn xoff_is_dropped() {
    assert_eq!(lines(b"a\x13b\r"), vec![line(b"ab")]);
}

#[test]
fn doubled_iac_is_kept_as_ff() {
    assert_eq!(wire(b"a\xff\xffb\r\n"), vec![line(b"a\xffb")]);
}

#[test]
fn high_bit_byte_is_kept() {
    assert_eq!(lines(b"\xc9\r"), vec![line(b"\xc9")]);
}

#[test]
fn char_mode_delivers_keys_without_echo() {
    let mut ed = LineEditor::new();
    ed.set_char_mode(true);
    let (events, echo) = edit(&mut ed, b"q\x1b\r\n");
    assert_eq!(
        events,
        vec![
            EditorEvent::Key(b'q'),
            EditorEvent::Key(0x1b),
            EditorEvent::Key(b'\r')
        ]
    );
    assert!(echo.is_empty());
}

#[test]
fn queue_keeps_oldest_and_drops_newest_past_the_cap() {
    let mut ed = LineEditor::new();
    let input: Vec<u8> = (0..PENDING_CAP + 5)
        .flat_map(|i| format!("{i}\r").into_bytes())
        .collect();
    let (events, _) = edit(&mut ed, &input);
    assert_eq!(events.len(), PENDING_CAP);
    assert_eq!(events[0], line(b"0"));
    assert_eq!(events[PENDING_CAP - 1], line(b"63"));
}

proptest! {
    #[test]
    fn random_streams_never_panic_and_lines_are_clean(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048)
    ) {
        let mut codec = TelnetCodec::new();
        let (mut data, mut reply) = (Vec::new(), Vec::new());
        codec.decode(&bytes, &mut data, &mut reply);
        let has_doubled_iac = bytes.windows(2).any(|w| w == [0xFF, 0xFF]);
        for event in lines(&data) {
            if let EditorEvent::Line(l) = event {
                prop_assert!(l.len() <= LINE_LIMIT);
                prop_assert!(!l.iter().any(|&b| b < 0x20 || b == 0x7F));
                // 0xFF reaches a line only as an escaped IAC IAC from the client.
                prop_assert!(has_doubled_iac || !l.contains(&0xFF));
            }
        }
    }
}
