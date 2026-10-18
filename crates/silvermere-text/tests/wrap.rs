//! 79-column word wrap, as the host's word wrap (`btutsw`) did it for the displays that
//! asked (the room display and `You notice`, for example).

use silvermere_text::colour::Palette;
use silvermere_text::msg::Msg;
use silvermere_text::render::render;
use silvermere_text::strip::strip_ansi;
use silvermere_text::wrap::{WRAPCOLS, wrap};

/// The `example` of a static pattern in `tests/fixtures/patterns.json` (T-004's export).
fn registry_example(name: &str) -> String {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json"))
            .expect("patterns.json parses");
    let pattern = json["static"]
        .as_array()
        .expect("static is a list")
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("pattern {name} in the registry"));
    pattern["example"]
        .as_str()
        .expect("example is text")
        .to_string()
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8(strip_ansi(bytes))
        .unwrap()
        .split("\r\n")
        .map(str::to_string)
        .collect()
}

// a short exits line is not touched
#[test]
fn registry_exits_example_fits_on_one_line() {
    let example = registry_example("room_exits");
    let exits = example
        .strip_prefix("Obvious exits: ")
        .expect("the registry example")
        .to_string();
    let r = render(&Msg::RoomExits { exits }, Palette::default());
    assert!(r.wraps);
    assert_eq!(lines(&r.bytes), [example, String::new()]);
}

// an exits line that wraps ends in "," or in a word of the next exit
// ("door"/"gate"/"passage") and continues on the next line
#[test]
fn long_exits_line_wraps_at_79_at_the_last_space() {
    let exits =
        "closed door north, closed door south, closed door east, closed door west, up, down";
    let r = render(
        &Msg::RoomExits {
            exits: exits.into(),
        },
        Palette::default(),
    );
    let l = lines(&r.bytes);
    assert_eq!(
        l,
        [
            "Obvious exits: closed door north, closed door south, closed door east, closed",
            "door west, up, down",
            "",
        ]
    );
    assert!(l[0].len() <= usize::from(WRAPCOLS));
    // the colour prefix is still in front and does not count toward the column
    assert!(r.bytes.starts_with(b"\x1B[0;32mObvious exits: "));
}

// MBBSEmu.Tests SessionBase_Tests.cs:82: a line never fills all 80 columns
#[test]
fn a_line_without_spaces_breaks_at_79() {
    let text: String = (0..200)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let wrapped = wrap(format!("{text}\r\n").as_bytes(), WRAPCOLS);
    let l: Vec<&str> = std::str::from_utf8(&wrapped)
        .unwrap()
        .split("\r\n")
        .collect();
    assert_eq!(l.len(), 4);
    assert_eq!(l[0].len(), 79);
    assert_eq!(l[1].len(), 79);
    assert_eq!(l[2].len(), 42);
    assert_eq!(l[3], "");
    assert_eq!(l.concat(), text);
}

// MBBSEmu LineBreaker: ANSI sequences are not printable characters
#[test]
fn ansi_sequences_do_not_count_toward_the_column() {
    let word = "x".repeat(70);
    let plain = format!("{word} yyyyyyyyyy\r\n");
    let coloured = format!("\x1B[79D\x1B[K\x1B[1;31m{word}\x1B[0m \x1B[0;32myyyyyyyyyy\x1B[0m\r\n");
    let plain_lines = lines(&wrap(plain.as_bytes(), WRAPCOLS));
    let coloured_lines = lines(&wrap(coloured.as_bytes(), WRAPCOLS));
    assert_eq!(plain_lines, [word.as_str(), "yyyyyyyyyy", ""]);
    assert_eq!(coloured_lines, plain_lines);
    // and the sequences themselves survive, with the break after the colour reset
    let raw = wrap(coloured.as_bytes(), WRAPCOLS);
    assert!(
        raw.windows(9).any(|w| w == b"\x1B[0m\r\n\x1B[0"),
        "{:?}",
        String::from_utf8_lossy(&raw)
    );
}

// "You notice ... here." wraps too
#[test]
fn room_items_wrap() {
    let items = "9 copper farthings, torch, torch, torch, torch, torch, torch, torch, torch, torch";
    let r = render(
        &Msg::RoomItems {
            items: items.into(),
        },
        Palette::default(),
    );
    let l = lines(&r.bytes);
    assert_eq!(l.len(), 3);
    assert!(l[0].starts_with("You notice 9 copper farthings, torch,"));
    assert!(l[0].len() <= 79);
    assert!(l[1].ends_with(" here."));
}

// a message that does not ask for wrapping is never wrapped
#[test]
fn messages_without_wrap_are_left_alone() {
    let who = "A".repeat(120);
    let r = render(&Msg::PlayerEntered { who: who.clone() }, Palette::default());
    assert!(!r.wraps);
    assert_eq!(
        lines(&r.bytes),
        [format!("{who} just entered the Realm."), String::new()]
    );
}
