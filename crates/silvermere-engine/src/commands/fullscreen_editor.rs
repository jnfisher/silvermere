//! The full-screen character editor (T-063): the EDITCHA1 form over the stat fields on an
//! ANSI terminal of at least 80x23, driven one key at a time.
//!
//! A thin view over the line-mode editor's state: the values, the cursor and the rules
//! are [`StatEditor`]'s, every answer goes through [`StatEditor::verify_field`], and leaving runs
//! the same [`newchar::editor_done`]; only the presentation differs. The screen bytes are
//! [`EditorScreen`]'s.
//!
//! The full-screen key handling follows the T-005 capture of the original
//! (`tests/captures/creation-fsd`):
//!
//! | Key | Effect |
//! |---|---|
//! | a printable byte (digits only in a stat) | the first one blanks a field that holds something, then each is echoed |
//! | Backspace, DEL | deletes the last character typed |
//! | CR, down arrow | the field is verified; accepted: the CP display if asked for, the next field |
//! | up arrow | the same, then the previous field |
//! | Space in the looks or the exit field | the next `ALT=` value, in upper case |
//! | CR on the exit field | SAVE or QUIT leave the form, EDIT goes back to the first field |
//!
//! Each move clears the error line and redraws the field left and the field entered. A
//! refused answer shows the refusal on the error line and leaves the cursor on the field.
//! Other escape sequences are swallowed.

use silvermere_text::Msg;
use silvermere_text::editor_screen::{CellKind, EditorScreen};
use silvermere_types::ids::SessionId;
use silvermere_types::names::{EYE_COLOURS, HAIR_COLOURS, HAIR_LENGTHS};

use crate::output::OutputBatch;
use crate::player_session::Caps;
use crate::state::Outcome;
use crate::state::newchar::stat_editor::{Exit, Field, StatEditor, Verdict};
use crate::world::World;
use crate::{EngineOutput, state::newchar};

/// `ALT=SAVE ALT=EDIT ALT=QUIT`: the exit field's choices.
const EXIT_CHOICES: [&str; 3] = ["SAVE", "EDIT", "QUIT"];

const CR: u8 = b'\r';
const BS: u8 = 0x08;
const DEL: u8 = 0x7f;
const ESC: u8 = 0x1b;

/// Full screen on an ANSI terminal of at least 23 rows and 80 columns.
pub fn fits(caps: &Caps) -> bool {
    caps.ansi && caps.rows >= 23 && caps.columns >= 80
}

/// The form's own state, kept in [`StatEditor::fullscreen`]; the cursor is
/// [`StatEditor::field`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullScreen {
    /// What the focused name or stat field holds.
    buffer: String,
    /// A key has been typed into the focused field since it was entered.
    typed: bool,
    /// The exit field's choice, an index into [`EXIT_CHOICES`].
    exit: usize,
    /// The looks fields cycled with Space show their value in upper case.
    upper: [bool; 3],
    /// Where an escape sequence is.
    esc: EscState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum EscState {
    #[default]
    None,
    /// ESC seen.
    Esc,
    /// `ESC [` or `ESC O` seen: parameters until the final byte.
    Seq,
}

/// A decoded key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Byte(u8),
    Up,
    Down,
}

impl FullScreen {
    /// Bytes to keys: arrows are `ESC [ A`/`ESC [ B` (or `ESC O`); other sequences
    /// are dropped.
    fn decode(&mut self, byte: u8) -> Option<Key> {
        match self.esc {
            EscState::None if byte == ESC => {
                self.esc = EscState::Esc;
                None
            }
            EscState::None => Some(Key::Byte(byte)),
            EscState::Esc => {
                self.esc = if byte == b'[' || byte == b'O' {
                    EscState::Seq
                } else {
                    EscState::None
                };
                None
            }
            EscState::Seq => {
                if (0x40..=0x7e).contains(&byte) {
                    self.esc = EscState::None;
                    match byte {
                        b'A' => Some(Key::Up),
                        b'B' => Some(Key::Down),
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }
}

/// Opens the form over `editor` (already filled by [`StatEditor::open`] or
/// [`StatEditor::open_existing`]) and draws it: the background, the CP display, every
/// field with its initial answer, the empty error line and the first field focused.
pub fn open(editor: &mut StatEditor, race: &str, class: &str, out: &mut OutputBatch) {
    let screen = EditorScreen::editcha1();
    let fsd = FullScreen::default();
    let mut bytes = screen.background();
    bytes.extend(screen.cp_left(editor.unspent()));
    for n in 0..screen.field_count() {
        let value = match n {
            2 => race.to_string(),
            3 => class.to_string(),
            // the race minimum and maximum of each stat
            4..=21 if (n - 4) % 3 == 0 => editor.min[(n - 4) / 3].to_string(),
            4..=21 if (n - 4) % 3 == 1 => editor.max[(n - 4) / 3].to_string(),
            _ => value_of(editor, &fsd, field_of(n)),
        };
        bytes.extend(screen.draw(n, &value));
    }
    bytes.extend(screen.message(""));
    let first = editor.first_field();
    editor.field = first;
    bytes.extend(screen.focus(first.sf(), &value_of(editor, &fsd, first)));
    editor.fullscreen = Some(Box::new(fsd));
    out.push_raw(bytes);
}

/// One key typed while the form is open.
pub fn on_key(world: &mut World, id: SessionId, byte: u8) -> Outcome {
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let Some(editor) = s.editor.as_mut() else {
        return Outcome::Continue;
    };
    let Some(mut fsd) = editor.fullscreen.take() else {
        return Outcome::Continue;
    };
    let mut bytes = Vec::new();
    let left = match fsd.decode(byte) {
        Some(key) => press(editor, &mut fsd, key, &mut bytes),
        None => None,
    };
    editor.fullscreen = Some(fsd);
    s.output.push_raw(bytes);
    if let Some(exit) = left {
        world
            .outbox
            .push(EngineOutput::SetCharMode { id, on: false });
        newchar::editor_done(world, id, exit == Exit::Save);
    }
    Outcome::Continue
}

/// The field with `SF_*` number `n`, for the fields the cursor can visit (the others
/// map to the given name, which is never asked of them).
fn field_of(n: usize) -> Field {
    use crate::state::newchar::stat_editor::Stat;
    match n {
        1 => Field::FamilyName,
        6..=21 => Field::Stat(Stat::ALL[(n - 6) / 3]),
        22 => Field::HairLength,
        23 => Field::HairColour,
        24 => Field::EyeColour,
        25 => Field::Exit,
        _ => Field::GivenName,
    }
}

/// What a field shows when the cursor is not typing into it.
fn value_of(editor: &StatEditor, fsd: &FullScreen, field: Field) -> String {
    let looks = |table: &[&str], i: u8, upper: bool| {
        let v = table[usize::from(i)];
        if upper {
            v.to_ascii_uppercase()
        } else {
            v.to_string()
        }
    };
    match field {
        Field::HairLength => looks(HAIR_LENGTHS, editor.hairlen, fsd.upper[0]),
        Field::HairColour => looks(HAIR_COLOURS, editor.haircol, fsd.upper[1]),
        Field::EyeColour => looks(EYE_COLOURS, editor.eyecol, fsd.upper[2]),
        Field::Exit => EXIT_CHOICES[fsd.exit].to_string(),
        _ => editor.current(field),
    }
}

/// One decoded key. Returns how the form was left, if it was.
fn press(
    editor: &mut StatEditor,
    fsd: &mut FullScreen,
    key: Key,
    out: &mut Vec<u8>,
) -> Option<Exit> {
    let screen = EditorScreen::editcha1();
    let field = editor.field;
    let n = field.sf();
    let looks = matches!(
        field,
        Field::HairLength | Field::HairColour | Field::EyeColour
    );
    match key {
        Key::Byte(CR) if field == Field::Exit => {
            match fsd.exit {
                0 | 2 => {
                    let exit = if fsd.exit == 0 {
                        Exit::Save
                    } else {
                        Exit::Quit
                    };
                    out.extend(screen.message(""));
                    out.extend(screen.draw(n, EXIT_CHOICES[fsd.exit]));
                    out.extend(screen.leave());
                    return Some(exit);
                }
                // EDIT: back to the first field with the answers kept
                _ => {
                    fsd.exit = 0;
                    let first = editor.first_field();
                    move_to(editor, fsd, first, out);
                }
            }
            None
        }
        Key::Byte(CR) | Key::Down => {
            let next = if field == Field::Exit {
                field
            } else {
                field.next()
            };
            leave_field(editor, fsd, next, out);
            None
        }
        Key::Up => {
            let prev = field.prev();
            let prev = if prev == Field::GivenName && !editor.show_first_name {
                field
            } else {
                prev
            };
            leave_field(editor, fsd, prev, out);
            None
        }
        Key::Byte(b' ') if looks || field == Field::Exit => {
            let shown = match field {
                Field::HairLength => {
                    editor.hairlen = (editor.hairlen + 1) % HAIR_LENGTHS.len() as u8;
                    fsd.upper[0] = true;
                    value_of(editor, fsd, field)
                }
                Field::HairColour => {
                    editor.haircol = (editor.haircol + 1) % HAIR_COLOURS.len() as u8;
                    fsd.upper[1] = true;
                    value_of(editor, fsd, field)
                }
                Field::EyeColour => {
                    editor.eyecol = (editor.eyecol + 1) % EYE_COLOURS.len() as u8;
                    fsd.upper[2] = true;
                    value_of(editor, fsd, field)
                }
                _ => {
                    fsd.exit = (fsd.exit + 1) % EXIT_CHOICES.len();
                    EXIT_CHOICES[fsd.exit].to_string()
                }
            };
            out.extend(screen.focus(n, &shown));
            None
        }
        _ if looks || field == Field::Exit => None,
        Key::Byte(BS | DEL) => {
            if !fsd.typed {
                start_typing(fsd, n, out);
            } else if fsd.buffer.pop().is_some() {
                out.extend_from_slice(b"\x08 \x08");
            }
            None
        }
        Key::Byte(b) => {
            let cell = screen.field(n);
            let ok = match cell.kind {
                CellKind::Numeric => b.is_ascii_digit(),
                _ => (0x20..0x7f).contains(&b),
            };
            if !ok {
                return None;
            }
            if !fsd.typed {
                start_typing(fsd, n, out);
            }
            if fsd.buffer.len() < usize::from(cell.width) {
                fsd.buffer.push(char::from(b));
                out.push(b);
            }
            None
        }
    }
}

/// The first key typed into a field replaces what it held: a field with something in
/// it is blanked first (the capture: Strength `40` is cleared before `5`, the empty
/// family name is not redrawn before `Stonebrook`).
fn start_typing(fsd: &mut FullScreen, n: usize, out: &mut Vec<u8>) {
    fsd.typed = true;
    if !fsd.buffer.is_empty() {
        fsd.buffer.clear();
        out.extend(EditorScreen::editcha1().focus(n, ""));
    }
}

/// The cursor leaves the focused field towards `to`: the answer is verified, then the move, or the
/// refusal on the error line.
fn leave_field(editor: &mut StatEditor, fsd: &mut FullScreen, to: Field, out: &mut Vec<u8>) {
    let screen = EditorScreen::editcha1();
    let field = editor.field;
    let answer = if fsd.typed {
        fsd.buffer.clone()
    } else {
        editor.current(field)
    };
    match editor.verify_field(field, &answer) {
        Verdict::Reject(msg) => {
            // the refusal on the error line; the field keeps the answer, the cursor stays
            out.extend(screen.message(&message_text(&msg)));
            fsd.buffer = answer;
            fsd.typed = false;
            out.extend(screen.focus(field.sf(), &fsd.buffer));
        }
        Verdict::ShowCp => {
            // the CP left to distribute
            out.extend(screen.cp_left(editor.unspent()));
            move_to(editor, fsd, to, out);
        }
        Verdict::Accept | Verdict::Again => move_to(editor, fsd, to, out),
    }
}

/// Clears the error line, redraws the field left (unfocused, with its stored value) and
/// focuses `to`.
fn move_to(editor: &mut StatEditor, fsd: &mut FullScreen, to: Field, out: &mut Vec<u8>) {
    let screen = EditorScreen::editcha1();
    let field = editor.field;
    out.extend(screen.message(""));
    out.extend(screen.draw(field.sf(), &value_of(editor, fsd, field)));
    editor.field = to;
    fsd.typed = false;
    fsd.buffer = value_of(editor, fsd, to);
    out.extend(screen.focus(to.sf(), &fsd.buffer));
}

/// The plain text of a refusal, for the error line.
fn message_text(msg: &Msg) -> String {
    let bytes = silvermere_text::render(msg, Default::default()).bytes;
    let text = silvermere_text::cp437::decode(&silvermere_text::strip_ansi(&bytes));
    text.trim_end_matches(['\r', '\n']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_session::clear_player;
    use crate::state::newchar::stat_editor::Stat;
    use crate::world::blank::clear_race;
    use proptest::prelude::*;
    use silvermere_types::ids::RaceNo;

    fn editor(min: [u16; 6], span: [u16; 6], cp: u16) -> StatEditor {
        let mut rac = clear_race(RaceNo(1));
        let to_i = |n: u16| n as i16;
        (rac.minstr, rac.minint, rac.minwil) = (to_i(min[0]), to_i(min[1]), to_i(min[2]));
        (rac.minagi, rac.minhea, rac.mincha) = (to_i(min[3]), to_i(min[4]), to_i(min[5]));
        let max: Vec<i16> = (0..6).map(|i| to_i(min[i] + span[i])).collect();
        (rac.maxstr, rac.maxint, rac.maxwil) = (max[0], max[1], max[2]);
        (rac.maxagi, rac.maxhea, rac.maxcha) = (max[3], max[4], max[5]);
        let mut plr = clear_player();
        plr.firstname = "Kasimir".into();
        (plr.str, plr.intel, plr.wil) = (min[0], min[1], min[2]);
        (plr.agi, plr.hea, plr.cha) = (min[3], min[4], min[5]);
        plr.totalcp = cp;
        StatEditor::open(&plr, &rac, false)
    }

    proptest! {
        // the line-mode editor and the form accept exactly the same answers, stat by stat, and end
        // with the same totals
        #[test]
        fn both_editors_accept_the_same_stat_vectors(
            min in proptest::array::uniform6(10u16..60),
            span in proptest::array::uniform6(0u16..60),
            cp in 0u16..300,
            v in proptest::array::uniform6(0u16..200),
        ) {
            let mut line = editor(min, span, cp);
            let mut form = editor(min, span, cp);
            let mut out = OutputBatch::default();
            open(&mut form, "Human", "Warrior", &mut out);
            let mut fsd = form.fullscreen.take().unwrap();
            for (i, stat) in Stat::ALL.into_iter().enumerate() {
                line.field = Field::Stat(stat);
                line.on_line(&v[i].to_string(), &mut out);
                let line_ok = line.field != Field::Stat(stat);
                form.field = Field::Stat(stat);
                fsd.typed = false;
                fsd.buffer = form.current(form.field);
                let mut bytes = Vec::new();
                for b in format!("{}\r", v[i]).bytes() {
                    press(&mut form, &mut fsd, Key::Byte(b), &mut bytes);
                }
                let form_ok = form.field != Field::Stat(stat);
                prop_assert_eq!(line_ok, form_ok, "stat {}", i);
            }
            prop_assert_eq!(line.values, form.values);
            prop_assert_eq!(line.unspent(), form.unspent());
        }
    }

    #[test]
    fn arrows_decode_and_other_sequences_drop() {
        let mut fsd = FullScreen::default();
        let mut keys = Vec::new();
        for &b in b"\x1b[A\x1b[B\x1bOA\x1b[1;5C\x1b[2~\x1bxq" {
            if let Some(k) = fsd.decode(b) {
                keys.push(k);
            }
        }
        assert_eq!(keys, [Key::Up, Key::Down, Key::Up, Key::Byte(b'q')]);
    }

    #[test]
    fn fits_80x23_ansi() {
        let caps = |columns, rows, ansi| Caps {
            columns,
            rows,
            ansi,
            utf8: false,
        };
        assert!(fits(&caps(80, 23, true)));
        assert!(fits(&Caps::default()));
        assert!(!fits(&caps(79, 25, true)));
        assert!(!fits(&caps(80, 22, true)));
        assert!(!fits(&caps(132, 50, false)));
    }
}
