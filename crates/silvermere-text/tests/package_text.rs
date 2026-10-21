//! The game's long text read from the package (T-138) is byte for byte the text the
//! repository held before: every help topic, text block, creation form and moved catalogue
//! screen is hashed here as the old files gave it (and, for the moved catalogue entries, as
//! the renderer sent it), and compared with what [`silvermere_text::package`] reads from the
//! stock package.

use sha2::{Digest, Sha256};
use silvermere_text::catalogue::TemplatePiece;
use silvermere_text::editor_screen::{CellKind, EditorScreen};
use silvermere_text::package::{self, MOVED_KEYS, NOT_INSTALLED, PackageText};
use silvermere_text::{Msg, Palette, render};

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every help topic and the overview: `overview NUL`, then per topic its key, text key,
/// words and text, each followed by NUL.
const HELP_TOPICS: usize = 131;
const HELP_SHA: &str = "5d624d3fb1dd93a2f358918ba6327928905ca8ab1f92166a7fc6685cd1090ce1";

/// The text blocks: file, key, SHA-256 of the template.
const TEXT_BLOCKS: &[(&str, &str, &str)] = &[
    (
        "wccmmud",
        "usrlogin",
        "92c29528fffa78a87f60cc90c23d0cc81be3f2714b4467d854a5ff52116b69af",
    ),
    (
        "wccmmud",
        "usrlogof",
        "14a0312fa23de5ca79b3f51cc63410216a32bbbc6987818a1ccb5ba8c17bca08",
    ),
    (
        "wccmmud",
        "usrfoled",
        "efe51c4015594e4e55ac3a99f9282f9728d7e069f4d56418e83146b1929f0d00",
    ),
    (
        "wccmmud",
        "usrexit",
        "d0976e9e825beea37384bc4c2df09091d1ec95f0c205e0d1bda3dc0de52789ee",
    ),
    (
        "wccmmud",
        "leavemnu",
        "4b3ad3a092aaa00fb3f57db445445d38b2085adf2fb98b73015b5a160fd31775",
    ),
    (
        "wccmmud",
        "inachup",
        "f473e9e19689fdd4f716a336c2abfcb68c91cceaad2d2c841b09da9219d6019d",
    ),
    (
        "wcctext",
        "goodtext",
        "3272007173fae82c83891136ec826005bd9af4bbef380e670400c0a17a62c2c5",
    ),
    (
        "wcctext",
        "goodpmpt",
        "02278c59e365d7efcd1a0ed33e668d6242e407455915ea63d87044eabc19bbd3",
    ),
    (
        "wcctext",
        "chsrace",
        "14c7810a910257f6e1281c2319c7d9ff8c13ccda9700ba3d10defe1cb7d18646",
    ),
    (
        "wcctext",
        "chsclass",
        "0a26a76ddcd0ceefc7ea38685df91b0841c6430748222d2a72844ba28bf1b5b9",
    ),
    (
        "wcctext",
        "menutext",
        "2b156b700b433bdb3cc0f9d567bac070352281b66bf055fa0bc29e62905cea23",
    ),
    (
        "wcctext",
        "menupmpt",
        "344c5df4df5cb247bdb28ad952256fa3c13196af5ed0a5b2f137cbee905d8545",
    ),
];

/// The creation forms: name, SHA-256 of the text.
const FORMS: &[(&str, &str)] = &[
    (
        "editcha1",
        "8d30b7d35e69aa8dc14b4d1496ec8ff6ecd445fb6d92888b4ebc31763bc6e4b8",
    ),
    (
        "edtchan2",
        "b49c566f52296c3e1146b5a1a1087c2a4efb5b211fcaa6720f2e3f2505c7d5b2",
    ),
];

/// The moved catalogue entries: key, SHA-256 of `Msg::text()`, SHA-256 of the rendered bytes
/// in the default palette.
const MOVED: &[(&str, &str, &str)] = &[
    (
        "menu_about_ansi",
        "93d157bb9a68fac6ac4bc6c980976626ee3351355182faeb2acf6cadb5438b05",
        "405aa0661206b965726e6a50cb476ae36560918278f93774b6008cf23c9d78ba",
    ),
    (
        "menu_about_plain",
        "78140a7f49ed2aea091cecb4e7d28f67f53149da568e790414bf361c63b85f46",
        "21d62133b8fc6271f3aa1735e2e87c44d1da925fbf0c7d771884f31630f128d0",
    ),
    (
        "menu_info_ansi",
        "9edfabd190b3e151b3906eee409b87cbad68527f7a5a30aafb52f29dec56ba18",
        "aff83942b22f353391c64382f7ad4d5a78375d8290219e8f0c31b232f803e30c",
    ),
    (
        "menu_info_plain",
        "fb82dec9967c83a683399182a83fdc6782102c74a50a9ba4e3fd9310bb33a1dc",
        "7c2590d6b9dd69852050fe6609b8c76328a8110a09ad53cf0debf3f1f2c53dc5",
    ),
    (
        "menu_counts_ansi",
        "456e3f21047093069cc74ec37288bc77b464c28a56d09cb4ebfc022bc285cffe",
        "54003a8d70fcce05082f188d288f3be76f163c2991573fbe6ac51e102415b0cb",
    ),
    (
        "menu_counts_plain",
        "30264010cb564bad26fdfe38b5edcb146442771a1e0c657f843de1198d1f8e20",
        "6d58b7584d9accd7b457e426fb13ea6561d4680bcace6cb933fa899d4518fec1",
    ),
    (
        "sysop_help",
        "7dcb0e8702099ea511fb22798f1f00a8c5b4eb260eb27469af21c7fe0fa4ac54",
        "37a99f63a09311d5aa746ac9142df31f00ecb12488caf279f02c3c25b5bf1efe",
    ),
    (
        "sysop_help_sysop",
        "38612f4812ac90388e6f7bdedcc2dd9718e9e9140e2ca3fea3329eac09810f66",
        "ced9c14e86320856fffbfbdb0ee4bf45990c7ef90c434d4a5cb872d776b6b4bc",
    ),
    (
        "sysop_menu",
        "e5f8eb4b680729ebf127c561bb49fd279ab76fb639d34122b8d2f5f06bb808f1",
        "7af71747e5a983ab807317dd52d397260a3d11db28b38a9d6f892f6dad838ec4",
    ),
    (
        "stat_editor_preamble",
        "95b2721dc1c3bb70ef3120b74693a79d0031bc8011c2d860c7936a3c8c3da23b",
        "95b2721dc1c3bb70ef3120b74693a79d0031bc8011c2d860c7936a3c8c3da23b",
    ),
    (
        "enter_stats_reset",
        "b0492c96d2563e2382d7fdc6cc839acd61812e900a6e2672bdb8f262b3d40ce6",
        "494471bc42b37d69d9ef440eaf1490ce4c0bfad71d113d5d91e1d39c425bc8f0",
    ),
    (
        "save_refused",
        "6279e62366d2290947574ea677cdc3c10fe4e47bfe6d833ad94cea76c0b5e2dd",
        "6eebb8ea768b736a7c42676177301d559183cb5d5ae0d243460ba801eb0168c3",
    ),
    (
        "suicide_pw_notice",
        "23a5672b68f4c739751ad063118eaca25da3113b5bb7aae140acfd1e8fab3596",
        "ee99e0b762ae8593a930e6adab30296df1639698c444ab6a5cd7571ef525ac3a",
    ),
    (
        "sysop_god_syntax",
        "7f348da62c25943a0a1a804c1b5f5f37f479508f180b4638c3a47337ec856156",
        "aa537c3c237a92640362b6c92298b34ba863649851ed94c402331b2e67deed74",
    ),
    (
        "sysop_god_valid_options",
        "753c558f5fa73624f634247747d84e7d7cb171f61f6f0d73ee8a1c6edb85abb3",
        "6831a40e9a44723c0727e8454859cb620909d22476ede55db146309823a01041",
    ),
    (
        "sysop_name_changed",
        "78971bb8a888207930f777c6bf82012fe67e447b7fd637e472482c9afefdfffe",
        "c371e6547bfbc1f800217ff443334d374c8927c54acd10c41772c7b6e855809c",
    ),
    (
        "sysop_valid_edits",
        "a926ac9575f5d15840cb167f16f58101ec79909b8ceb807eede70e8256a02a45",
        "67db6f37e12a0137defc4f0aa0a8e22234ff1e7a4b8a59203b7652f2d6279a90",
    ),
    (
        "sysop_xref_another",
        "ce5faedfde4e7b20c2998df4f61f6f829b896de5fd273f978943aa2406fcc65d",
        "851d76ce9e9d0b97b3f45cd06e6aa0ac83bf170509facc56902ada154cbaa011",
    ),
    (
        "race_change_done",
        "7854f0855a4224d67efb6e294fd433ad0660041cc8f4edca678006c2aac6c280",
        "82635c8c91cd1b46ac13b422ce00a78d216af2605ff1f458f841f4e6be01e904",
    ),
    (
        "class_change_done",
        "56d37172337482d5d21d41911a2a65f006000a637957d1908d9f9d322cd395fc",
        "cfcef083e38e5be2348d028f0bc8d10698c3851675e71fe75305ae91fee16d97",
    ),
    (
        "cast_silly",
        "c5f46aadd338d1f9030cedb123ec13f5a67d78add867d00c3ae2683fe2b9a623",
        "7d8dfae359cbae3c3fe3444ddb033f9563d9c47094b4c5f76bf40d1eefec8985",
    ),
    (
        "goodtext",
        "3272007173fae82c83891136ec826005bd9af4bbef380e670400c0a17a62c2c5",
        "6771bd95908934d713269f34b7d586b91045a9d0c001b7cceaa7d2e2b73e7462",
    ),
];

#[test]
fn every_help_topic_is_byte_identical() {
    let help = crate::package().help();
    assert_eq!(help.topics.len(), HELP_TOPICS);
    let mut all = format!("{}\0", help.overview);
    for t in &help.topics {
        all.push_str(&format!(
            "{}\0{}\0{}\0{}\0",
            t.key, t.body_key, t.words, t.text
        ));
    }
    assert_eq!(sha(all.as_bytes()), HELP_SHA);
}

#[test]
fn every_text_block_is_byte_identical() {
    let package = crate::package();
    for (file, key, want) in TEXT_BLOCKS {
        let cat = match *file {
            "wccmmud" => package.wccmmud(),
            _ => package.wcctext(),
        };
        assert_eq!(
            sha(cat.messages[*key].text.as_bytes()),
            *want,
            "{file} {key}"
        );
    }
    assert_eq!(
        package.wccmmud().messages.len() + package.wcctext().messages.len(),
        TEXT_BLOCKS.len()
    );
}

#[test]
fn both_forms_are_byte_identical() {
    let package = crate::package();
    for (name, want) in FORMS {
        let text = match *name {
            "editcha1" => package.editcha1(),
            _ => package.edtchan2(),
        };
        assert_eq!(sha(text.as_bytes()), *want, "{name}");
    }
}

#[test]
fn every_moved_screen_renders_byte_identically() {
    crate::package();
    let keys: Vec<&str> = MOVED.iter().map(|(k, _, _)| *k).collect();
    assert_eq!(keys, MOVED_KEYS);
    for (key, want_text, want_bytes) in MOVED {
        let msg = Msg::examples()
            .into_iter()
            .find(|m| m.key() == *key)
            .unwrap_or_else(|| panic!("no Msg {key}"));
        assert_eq!(sha(msg.text().as_bytes()), *want_text, "{key} text");
        assert_eq!(
            sha(&render(&msg, Palette::default()).bytes),
            *want_bytes,
            "{key} bytes"
        );
    }
}

// The catalogue keeps each moved entry's key, and its text is the notice shown until the
// package text is installed; no other entry has that text.
#[test]
fn the_catalogue_holds_only_the_notice_for_moved_entries() {
    let catalogue =
        silvermere_text::catalogue::parse(silvermere_text::msg::CATALOGUE_TOML).unwrap();
    let notices: Vec<&str> = catalogue
        .messages
        .iter()
        .filter(|(_, e)| e.text == NOT_INSTALLED)
        .map(|(k, _)| k.as_str())
        .collect();
    let mut moved = MOVED_KEYS.to_vec();
    moved.sort_unstable();
    assert_eq!(notices, moved);
    for key in MOVED_KEYS {
        let msg = Msg::examples()
            .into_iter()
            .find(|m| m.key() == *key)
            .unwrap();
        let text: String = msg
            .pieces()
            .iter()
            .map(|p| match p {
                silvermere_text::Piece::Text(t) => *t,
                other => panic!("{key}: {other:?}"),
            })
            .collect();
        assert_eq!(text, NOT_INSTALLED, "{key}");
    }
}

#[test]
fn moved_templates_take_no_argument() {
    let package = crate::package();
    for key in MOVED_KEYS {
        let pieces = package.catalogue_pieces(key).unwrap();
        assert!(
            !pieces.iter().any(|p| matches!(p, TemplatePiece::Arg(_))),
            "{key}"
        );
    }
}

// A directory without the files, or with a damaged one, is an error naming the file.
#[test]
fn missing_or_damaged_files_are_errors() {
    let dir = std::env::temp_dir().join(format!("package-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let err = PackageText::load(&dir).unwrap_err();
    assert_eq!(err.file, package::HELP_FILE);
    for name in [
        package::HELP_FILE,
        package::WCCMMUD_FILE,
        package::WCCTEXT_FILE,
        package::DLL_TEXT_FILE,
    ] {
        // fresh files of our own: the stock tree is read-only, and a copy keeps that
        let bytes = std::fs::read(crate::stock::assets_dir().join(name)).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    assert!(PackageText::load(&dir).is_ok());
    let dll = std::fs::read(dir.join(package::DLL_TEXT_FILE)).unwrap();
    std::fs::write(dir.join(package::DLL_TEXT_FILE), &dll[..dll.len() / 2]).unwrap();
    let err = PackageText::load(&dir).unwrap_err();
    assert_eq!(err.file, package::DLL_TEXT_FILE);
    let _ = std::fs::remove_dir_all(&dir);
}
// ---- the EDITCHA1 form --------------------------------------------------------------

fn editcha1() -> &'static EditorScreen {
    crate::package();
    EditorScreen::editcha1().expect("installed")
}

// 26 fields, then the error line
#[test]
fn editcha1_has_the_26_fields_and_the_error_line() {
    let s = editcha1();
    assert_eq!(s.field_count(), 26);
    let m = s.message_field();
    assert_eq!((m.row, m.col, m.width), (23, 1, 61));
    assert_eq!(m.attr, b"\x1b[1;31m");
    let kinds: Vec<CellKind> = (0..26).map(|n| s.field(n).kind).collect();
    assert!(kinds[..4].iter().all(|k| *k == CellKind::Text));
    assert!(kinds[4..22].iter().all(|k| *k == CellKind::Numeric));
    assert!(kinds[22..].iter().all(|k| *k == CellKind::Text));
}

// the observed cursor positions of character creation; the capture
// (tests/captures/creation-fsd/script.json until_cursor) agrees
#[test]
fn positions_match_the_observed_cursor() {
    let s = editcha1();
    assert_eq!(s.field_pos(0), (4, 20));
    assert_eq!(s.field_pos(1), (5, 20));
    for (i, row) in (9..=14).enumerate() {
        assert_eq!(s.field_pos(6 + 3 * i), (row, 34), "stat {i}");
        assert_eq!(s.field_pos(4 + 3 * i), (row, 19));
        assert_eq!(s.field_pos(5 + 3 * i), (row, 27));
    }
    assert_eq!(s.field_pos(22), (16, 22));
    assert_eq!(s.field_pos(23), (17, 22));
    assert_eq!(s.field_pos(24), (18, 22));
    assert_eq!(s.field_pos(25), (20, 14));
    assert_eq!(s.field(0).width, 18);
    assert_eq!(s.field(6).width, 4);
    assert_eq!(s.field(22).width, 15);
    assert_eq!(s.field(25).width, 4);
}

#[test]
fn field_bytes_match_the_capture() {
    let s = editcha1();
    assert_eq!(
        s.draw(6, "50"),
        b"\x1b[0;1m\x1b[9;34f\x1b[1m    \x1b[9;34f50".to_vec()
    );
    assert_eq!(
        s.draw(5, "100"),
        b"\x1b[0;1m\x1b[9;27f    \x1b[9;27f100".to_vec()
    );
    assert_eq!(
        s.focus(22, "SHORT"),
        b"\x1b[16;22f\x1b[0;1;7m               \x1b[16;22fSHORT\x1b[16;22f".to_vec()
    );
    let mut clear = b"\x1b[0;1m\x1b[23;1f\x1b[1;31m".to_vec();
    clear.extend([b' '; 61]);
    assert_eq!(s.message(""), clear);
    assert_eq!(s.cp_left(90), b"\x1b[20;33f\x1b[0m  90".to_vec());
}
