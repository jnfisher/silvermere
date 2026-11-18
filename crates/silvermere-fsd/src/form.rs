//! A form session and its public API.

use crate::answers::{join, lookup};
use crate::check::match_choices;
use crate::render::{Justify, answer_in, spaces};
use crate::spec::{FieldSpec, parse};
use crate::template::{Help, Place, scan};

/// How the session is entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Full screen, with ANSI cursor moves.
    FullScreen,
    /// Line by line, for terminals without ANSI.
    Linear,
}

/// Where a session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Full screen: the cursor is on a field (the original's `FSDAPT`).
    FieldSelected,
    /// Full screen: a field is being typed in (`FSDAEN`).
    Editing,
    /// Line mode: a field is prompted (`FSDNPT`).
    LinePrompt,
    /// Line mode: a field is being typed in (`FSDNEN`).
    LineEditing,
    /// An entry is finished and waits for [`Form::process`] (`FSDBUF`).
    Pending,
    /// Full screen: a message is in the help area; the next key wipes it (`FSDKHP`).
    MessageShown,
    /// Over: save the answers.
    Save,
    /// Over: toss the answers.
    Quit,
}

/// A verify routine's verdict on an entry (the library's `VFYxxx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Store the entry as it is (`VFYOK`).
    Accept,
    /// Store it if the field's type, `MIN=`, `MAX=` or choices allow (`VFYCHK`).
    Check,
    /// Refuse it: the field is entered again (`VFYREJ`).
    Reject,
    /// Keep the old answer and move on (`VFYDEF`).
    Revert,
}

/// What [`Form::process`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Keep going.
    Continue,
    /// The session is over; save the answers.
    Save,
    /// The session is over; toss the answers.
    Quit,
}

/// A form's verify routine, called with the form, the field number, the entry (which it
/// may change) and the output (which it may write to). It returns its verdict and a
/// message for the help area, empty for none. [`Form::done_at_end`] is the standard one.
pub type Verify<'a> =
    dyn FnMut(&mut Form, usize, &mut Vec<u8>, &mut Vec<u8>) -> (Verdict, Vec<u8>) + 'a;

/// One field: its specification and its place in the template.
#[derive(Clone, Debug, Default)]
pub(crate) struct Field {
    pub(crate) spec: FieldSpec,
    pub(crate) place: Place,
    /// Skipped by the cursor and shown blank.
    pub(crate) avoided: bool,
    /// Changed this session.
    pub(crate) changed: bool,
}

/// The field being typed in.
#[derive(Clone, Debug, Default)]
pub(crate) struct Entry {
    pub(crate) text: Vec<u8>,
    /// Cursor position in `text`.
    pub(crate) pos: usize,
    /// The choice shown, if the text is one.
    pub(crate) choice: Option<usize>,
    /// A prefix of several choices is being typed.
    pub(crate) typing_choice: bool,
}

/// One form entry session.
#[derive(Clone, Debug)]
pub struct Form {
    pub(crate) template: Vec<u8>,
    pub(crate) fields: Vec<Field>,
    /// Fields in the template: the first ones of `fields`.
    pub(crate) on_screen: usize,
    pub(crate) help: Option<Help>,
    pub(crate) bottom_row: u8,
    pub(crate) answers: Vec<Vec<u8>>,
    pub(crate) mode: Mode,
    pub(crate) state: State,
    /// The field the cursor is on; in line mode one past the last after it is left.
    pub(crate) cursor: usize,
    /// The field whose entry is pending.
    pub(crate) entered: usize,
    /// The field still shown under the cursor while a cursor move is held back.
    pub(crate) shown_at: usize,
    /// The key that ended the entry.
    pub(crate) exit_key: u16,
    /// Answers changed this session, up to 255.
    pub(crate) changes: u8,
    pub(crate) entry: Entry,
    /// Output has gone to the host and may not have been sent yet.
    pub(crate) output_pending: bool,
    /// A cursor move waits for the output to be sent.
    pub(crate) move_held: bool,
    /// The pending entry is not to be checked or stored.
    pub(crate) skip_check: bool,
}

impl Form {
    /// Prepares a session (as the library's `fsdroom` and `fsdapr`): reads `template` (CP437
    /// message text as the host's `getasc` hands it over, CR LF line ends), the field specification `spec` and the default `answers` (an answer
    /// string), each default cut to its field's width. Both stop at a NUL.
    pub fn prepare(template: &[u8], spec: &[u8], mode: Mode, answers: &[u8]) -> Self {
        let end = template.iter().position(|&b| b == 0);
        let template = template[..end.unwrap_or(template.len())].to_vec();
        let specs = parse(spec);
        let layout = scan(&template, specs.len(), mode == Mode::FullScreen);
        let on_screen = layout.places.len();
        let mut places = layout.places.into_iter();
        let fields: Vec<Field> = specs
            .into_iter()
            .map(|spec| Field {
                spec,
                place: places.next().unwrap_or_default(),
                ..Field::default()
            })
            .collect();
        let answers = fields
            .iter()
            .map(|f| {
                let default = lookup(answers, &f.spec.name);
                default[..default.len().min(f.place.width)].to_vec()
            })
            .collect();
        Self {
            template,
            fields,
            on_screen,
            help: layout.help,
            bottom_row: layout.bottom_row,
            answers,
            mode,
            state: match mode {
                Mode::FullScreen => State::FieldSelected,
                Mode::Linear => State::LinePrompt,
            },
            cursor: 0,
            entered: 0,
            shown_at: 0,
            exit_key: 0,
            changes: 0,
            entry: Entry::default(),
            output_pending: false,
            move_held: false,
            skip_check: false,
        }
    }

    /// Draws the full-screen background: clears the screen and shows the template with the
    /// answers filled in, the help area blank and avoided fields as blanks (as the library's
    /// `fsdbkg`).
    pub fn background(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1B[0m\x1B[2J\x1B[0m");
        let mut text = self.template.clone();
        if let Some(help) = &self.help {
            let end = (help.offset + help.width).min(text.len());
            text[help.offset..end].fill(b' ');
        }
        let mut from = 0;
        for (field, answer) in self.fields[..self.on_screen].iter().zip(&self.answers) {
            let place = &field.place;
            out.extend_from_slice(&text[from..place.offset]);
            if field.avoided {
                spaces(place.width, out);
            } else {
                answer_in(place, answer, Justify::Field, out);
            }
            from = place.offset + place.width;
        }
        out.extend_from_slice(&text[from..]);
    }

    /// The answer of field `field`. Panics if there is no such field.
    pub fn answer(&self, field: usize) -> &[u8] {
        &self.answers[field]
    }

    /// The answer of the field called `name`, any case; empty if there is none.
    pub fn answer_named(&self, name: &str) -> &[u8] {
        self.fields
            .iter()
            .position(|f| f.spec.name.eq_ignore_ascii_case(name.as_bytes()))
            .map_or(&[], |i| &self.answers[i])
    }

    /// All the answers as an answer string, `NAME=value\0...\0\0`.
    pub fn answers(&self) -> Vec<u8> {
        let names = self.fields.iter().map(|f| f.spec.name.as_slice());
        join(names.zip(self.answers.iter().map(Vec::as_slice)))
    }

    /// The zero-based number of field `field`'s answer among its `ALT=` choices, when it
    /// is a prefix of exactly one (any case, blanks ignored); the answer is then completed
    /// to that choice (as the library's `fsdord`). Panics if there is no such field.
    pub fn ordinal(&mut self, field: usize) -> Option<usize> {
        let found = match_choices(&self.fields[field], &self.answers[field], None);
        if found.count != 1 {
            return None;
        }
        self.answers[field] = found.answer;
        found.first
    }

    /// Fields in the specification.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Has the cursor skip field `field` and shows it blank. The game avoids fields after
    /// drawing the background, so they show there. Panics if there is no such field.
    pub fn avoid(&mut self, field: usize) {
        self.fields[field].avoided = true;
    }

    /// Whether field `field` is avoided. Panics if there is no such field.
    pub fn is_avoided(&self, field: usize) -> bool {
        self.fields[field].avoided
    }

    /// Whether field `field`'s answer changed this session. Panics if there is no such
    /// field.
    pub fn is_changed(&self, field: usize) -> bool {
        self.fields[field].changed
    }

    /// How the session is entered.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The session state.
    pub fn state(&self) -> State {
        self.state
    }

    /// The field the cursor is on.
    pub fn current_field(&self) -> usize {
        self.cursor
    }

    /// The bottom screen row (0-based) the template reaches, full screen only; the host
    /// puts the cursor below it when the session ends.
    pub fn maxy(&self) -> u8 {
        self.bottom_row
    }

    /// Stores `answer` as field `field`'s; whether it differs from the old one.
    pub(crate) fn store(&mut self, field: usize, answer: Vec<u8>) -> bool {
        let changed = self.answers[field] != answer;
        self.answers[field] = answer;
        changed
    }
}
