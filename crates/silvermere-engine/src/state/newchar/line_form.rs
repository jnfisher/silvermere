//! The line by line character editor: the game's `EDTCHAN2` form on a terminal too small
//! for the full-screen one, or without ANSI, driven one key at a time as the host's form
//! library drives it.
//!
//! Each field is prompted with the form's text before it, then its stored answer, with the
//! cursor after it. The first key that is not a space replaces the answer; a space or a
//! backspace edits it instead. Enter checks the answer (the game's check, then the form's
//! own: the stats 10 to 250, the looks one of their choices) and goes on to the next field;
//! a refusal is shown on lines of its own, with a bell, and the field is asked again with
//! its stored answer. The looks and the exit field take their choices: space shows the
//! next, letters pick by the start of the name. The exit field leaves with SAVE or QUIT,
//! and EDIT goes back to the first field. Ctrl-U or the up arrow goes back a field, Ctrl-G
//! saves and Ctrl-O (or ESC ESC) quits.
//!
//! The rules are [`StatEditor`]'s ([`StatEditor::verify_field`]); this is the
//! presentation and the form's own checks.

use silvermere_text::Msg;
use silvermere_text::package;
use silvermere_types::names::{EYE_COLOURS, HAIR_COLOURS, HAIR_LENGTHS};

use crate::output::OutputBatch;
use crate::state::newchar::stat_editor::{Exit, Field, Stat, StatEditor, Verdict};

/// The stock package's `EDTCHAN2`, for when no package text is installed.
const STOCK_FORM: &str = "\x1b[0m\x1b[2J\r\n      M A J O R   M U D\r\n\r\n     Character Generator\r\n\r\n      Point Cost Chart\r\n\r\n +1-10 points costs 1 CP each\r\n+11-20 points costs 2 CP each\r\n+21-30 points costs 3 CP each\r\n+31-40 points costs 4 CP each\r\n\r\nGiven Name  ??????????\r\nFamily Name ??????????????????\r\nRace        ???????????????????????????????\r\nClass       ???????????????????????????????\r\n\r\nNot Used: Base Strength:   $$$$\r\nNot Used: Max Strength:   $$$$\r\n Strength:  $$$$\r\nNot Used: Base Intellect:  $$$$\r\nNot Used: Max Intellect:  $$$$\r\n Intellect: $$$$\r\nNot Used: Base Willpower:  $$$$\r\nNot Used: Max Willpower:  $$$$\r\n Willpower: $$$$\r\nNot Used: Base Agility:    $$$$\r\nNot Used: Max Agility:    $$$$\r\n Agility:   $$$$\r\nNot Used: Base Health:     $$$$\r\nNot Used: Max Health:     $$$$\r\n Health:    $$$$\r\nNot Used: Base Charm:      $$$$\r\nNot Used: Max Charm:      $$$$\r\n Charm:     $$$$\r\n\r\n             (hit space to toggle)\r\n Hair Length ???????????????\r\n\r\n             (hit space to toggle)\r\n Hair Colour ???????????????\r\n\r\n             (hit space to toggle)\r\n Eye Colour  ???????????????\r\n\r\n Exit: ????\r\n\r\n CP Available: $$$$\r\n";

/// The form's fields.
const FIELDS: usize = 26;

/// The choices of the looks and the exit field, as the form spells them.
const HAIR_LENGTH: &[&str] = &[
    "None",
    "Short",
    "Shoulder-Length",
    "Long",
    "Waist-length",
    "Ankle-Length",
];
const HAIR_COLOUR: &[&str] = &[
    "Black",
    "White",
    "Silver",
    "Red",
    "Brown",
    "Dark-Brown",
    "Blonde",
    "Green",
    "Blue",
    "Grey",
];
const EYE_COLOUR: &[&str] = &[
    "Black",
    "Crimson",
    "Yellow",
    "Pale-Blue",
    "Sea-Blue",
    "Dark-Blue",
    "Grey-Blue",
    "Slate-Grey",
    "Bright-Green",
    "Forest-Green",
    "Pale-Green",
    "Chesnut-Brown",
    "Dark-Brown",
    "Hazel",
    "Violet",
    "Lavender",
    "Golden",
];
const EXIT: &[&str] = &["SAVE", "EDIT", "QUIT"];

/// The longest a list of choices may be (the help line, less three).
const CHOICES_MAX: usize = 77;

const CR: u8 = b'\r';
const TAB: u8 = 0x09;
const BS: u8 = 0x08;
const DEL: u8 = 0x7f;
const ESC: u8 = 0x1b;
const CTRL_G: u8 = 0x07;
const CTRL_O: u8 = 0x0f;
const CTRL_U: u8 = 0x15;

/// A key, escape sequences decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Byte(u8),
    Up,
    Down,
    /// ESC ESC.
    Escape,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum EscState {
    #[default]
    None,
    Esc,
    /// `ESC [`.
    Bracket,
    /// `ESC O`: one byte more.
    O,
}

/// The form's state, kept in [`StatEditor::line`]; the field is [`StatEditor::field`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineForm {
    /// The template's field positions: start and width.
    cells: Vec<(usize, usize)>,
    /// The template.
    form: String,
    /// Each field's stored answer.
    answers: Vec<String>,
    /// What the field being answered shows.
    buf: String,
    /// Nothing typed since the field was prompted.
    fresh: bool,
    /// The start of a choice typed, more than one choice beginning with it.
    partial: bool,
    /// The choice the field shows, when it shows a whole one.
    alt: Option<usize>,
    /// A choice was just shown: a space is not taken until the output has gone.
    quiet: bool,
    /// Keys typed or answers changed since the form opened.
    changes: u32,
    esc: EscState,
}

/// The field with `SF_*` number `n`.
fn field_of(n: usize) -> Field {
    match n {
        0 => Field::GivenName,
        1 => Field::FamilyName,
        6..=21 => Field::Stat(Stat::ALL[(n - 6) / 3]),
        22 => Field::HairLength,
        23 => Field::HairColour,
        24 => Field::EyeColour,
        _ => Field::Exit,
    }
}

/// The choices of field `n`, if it has any.
fn choices(n: usize) -> Option<&'static [&'static str]> {
    match n {
        22 => Some(HAIR_LENGTH),
        23 => Some(HAIR_COLOUR),
        24 => Some(EYE_COLOUR),
        25 => Some(EXIT),
        _ => None,
    }
}

/// A stat field (`$$$$`).
fn numeric(n: usize) -> bool {
    (4..=21).contains(&n)
}

/// The choices that begin with `start` (case does not matter).
fn starting(table: &[&'static str], start: &str) -> Vec<usize> {
    (0..table.len())
        .filter(|&i| {
            table[i].len() >= start.len()
                && table[i].as_bytes()[..start.len()].eq_ignore_ascii_case(start.as_bytes())
        })
        .collect()
}

/// The choice `answer` is exactly, if any.
fn exactly(table: &[&str], answer: &str) -> Option<usize> {
    if answer.is_empty() {
        return None;
    }
    table.iter().position(|c| c.eq_ignore_ascii_case(answer))
}

/// `n` times backspace, space, backspace.
fn rub_out(n: usize, out: &mut Vec<u8>) {
    for _ in 0..n {
        out.extend_from_slice(b"\x08 \x08");
    }
}

/// The plain text of a refusal.
fn message_text(msg: &Msg) -> String {
    let bytes = silvermere_text::render(msg, Default::default()).bytes;
    let text = silvermere_text::cp437::decode(&silvermere_text::strip_ansi(&bytes));
    text.trim_end_matches(['\r', '\n']).to_string()
}

/// How a checked answer came out.
enum Checked {
    /// Stored as this.
    Stored(String),
    /// Refused, with this message.
    Refused(String),
    /// The exit field: SAVE or QUIT.
    Left(Exit),
    /// The exit field: EDIT, back to the first field.
    Edit,
}

impl LineForm {
    /// Opens the form over `editor` and prompts its first field.
    pub fn open(editor: &mut StatEditor, out: &mut OutputBatch) -> Self {
        // each line end as CR LF, as the form library reads the text
        let form = package::installed()
            .map_or(STOCK_FORM, |p| p.edtchan2())
            .replace("\r\n", "\r")
            .replace('\r', "\r\n");
        let cells = cells(&form);
        let mut answers = vec![String::new(); FIELDS];
        answers[0] = editor.given_name.clone();
        answers[1] = editor.family_name.clone();
        for stat in Stat::ALL {
            answers[Field::Stat(stat).sf()] = editor.values[stat.index()].to_string();
        }
        let looks = |t: &[&str], i: u8| t.get(usize::from(i)).copied().unwrap_or("").to_string();
        answers[22] = looks(HAIR_LENGTHS, editor.hairlen);
        answers[23] = looks(HAIR_COLOURS, editor.haircol);
        answers[24] = looks(EYE_COLOURS, editor.eyecol);
        answers[25] = "SAVE".into();
        let mut f = Self {
            cells,
            form,
            answers,
            buf: String::new(),
            fresh: true,
            partial: false,
            alt: None,
            quiet: false,
            changes: 0,
            esc: EscState::None,
        };
        let first = editor.first_field();
        let mut bytes = Vec::new();
        f.prompt(editor, first.sf(), &mut bytes);
        out.push_raw(bytes);
        f
    }

    /// The output has gone: a space shows the next choice again.
    pub fn drained(&mut self) {
        self.quiet = false;
    }

    /// The fields the cursor visits.
    fn visits(editor: &StatEditor, n: usize) -> bool {
        match n {
            0 => editor.show_first_name,
            1 | 22..=25 => true,
            6..=21 => (n - 6).is_multiple_of(3),
            _ => false,
        }
    }

    /// The form's text before field `n`: from the end of the field before it (the whole
    /// head, after a line end, for the first).
    fn before(&self, n: usize) -> &str {
        let start = self.cells.get(n).map_or(self.form.len(), |c| c.0);
        if n == 0 {
            &self.form[..start]
        } else {
            let (s, w) = self.cells.get(n - 1).copied().unwrap_or((start, 0));
            &self.form[(s + w).min(start)..start]
        }
    }

    /// Field `n` asked: its text, then its stored answer, with the cursor after it.
    fn prompt(&mut self, editor: &mut StatEditor, n: usize, out: &mut Vec<u8>) {
        if n == 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend(silvermere_text::cp437::encode(self.before(n)));
        editor.field = field_of(n);
        self.buf = self.answers[n].clone();
        out.extend(silvermere_text::cp437::encode(&self.buf));
        self.fresh = true;
        self.partial = false;
        self.alt = choices(n).and_then(|t| exactly(t, &self.buf));
    }

    /// One byte typed. Returns how the form was left, if it was.
    pub fn key(
        &mut self,
        editor: &mut StatEditor,
        byte: u8,
        out: &mut OutputBatch,
    ) -> Option<Exit> {
        let byte = byte & 0x7f;
        let key = match (self.esc, byte) {
            (_, 0) => return None,
            (EscState::None, ESC) => {
                self.esc = EscState::Esc;
                return None;
            }
            (EscState::None, b) => Key::Byte(b),
            (EscState::Esc, b) => {
                self.esc = match b {
                    b'[' => EscState::Bracket,
                    b'O' => EscState::O,
                    _ => EscState::None,
                };
                match b {
                    b'[' | b'O' => return None,
                    ESC => Key::Escape,
                    b => Key::Byte(b),
                }
            }
            (EscState::Bracket, b) => {
                self.esc = EscState::None;
                match b {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    _ => return None,
                }
            }
            (EscState::O, _) => {
                self.esc = EscState::None;
                return None;
            }
        };
        let mut bytes = Vec::new();
        let left = self.press(editor, key, &mut bytes, out);
        out.push_raw(bytes);
        left
    }

    fn press(
        &mut self,
        editor: &mut StatEditor,
        key: Key,
        bytes: &mut Vec<u8>,
        out: &mut OutputBatch,
    ) -> Option<Exit> {
        let n = editor.field.sf();
        match key {
            Key::Byte(CR | TAB) | Key::Down => self.enter(editor, n, bytes, out),
            Key::Up | Key::Byte(CTRL_U) => {
                if let Some(prev) = (0..n).rev().find(|&p| Self::visits(editor, p)) {
                    if !self.fresh {
                        rub_out(self.buf.len(), bytes);
                    }
                    self.prompt(editor, prev, bytes);
                }
                None
            }
            Key::Byte(BS | DEL) => {
                self.fresh = false;
                // a whole choice is not rubbed out
                let whole = choices(n).is_some() && self.alt.is_some();
                if !whole && self.buf.pop().is_some() {
                    self.changes += 1;
                    bytes.extend_from_slice(b"\x08 \x08");
                }
                None
            }
            Key::Byte(CTRL_G) => self.leave(editor, n, Exit::Save, bytes, out),
            Key::Byte(CTRL_O) | Key::Escape => self.leave(editor, n, Exit::Quit, bytes, out),
            Key::Byte(b) if b >= 0x20 => {
                if let Some(table) = choices(n) {
                    self.fresh = false;
                    self.choice(table, b, bytes);
                    return None;
                }
                let takes = if numeric(n) { b.is_ascii_digit() } else { true };
                let width = self.cells.get(n).map_or(0, |c| c.1);
                if self.fresh && b != b' ' && takes {
                    rub_out(self.buf.len(), bytes);
                    self.buf.clear();
                }
                self.fresh = false;
                if takes && self.buf.len() < width {
                    self.buf.push(char::from(b));
                    self.changes += 1;
                    bytes.push(b);
                }
                None
            }
            Key::Byte(_) => None,
        }
    }

    /// A key on a field of choices: space shows the next, a letter picks by the start.
    fn choice(&mut self, table: &'static [&'static str], b: u8, bytes: &mut Vec<u8>) {
        let show = |f: &mut Self, i: usize, bytes: &mut Vec<u8>| {
            rub_out(f.buf.len(), bytes);
            f.buf = table[i].to_string();
            bytes.extend_from_slice(f.buf.as_bytes());
            f.alt = Some(i);
            f.partial = false;
            f.quiet = true;
            f.changes += 1;
        };
        if b == b' ' {
            if self.quiet {
                return;
            }
            let next = match self.alt {
                Some(i) => Some((i + 1) % table.len()),
                None => starting(table, &self.buf).first().copied(),
            };
            if let Some(i) = next {
                show(self, i, bytes);
            }
            return;
        }
        let mut start = if self.partial {
            self.buf.clone()
        } else {
            String::new()
        };
        start.push(char::from(b));
        let found = starting(table, &start);
        match found.len() {
            0 => {}
            1 => show(self, found[0], bytes),
            _ => {
                if self.partial {
                    self.buf.push(char::from(b));
                } else {
                    rub_out(self.buf.len(), bytes);
                    self.buf = char::from(b).to_string();
                }
                bytes.push(b);
                self.partial = true;
                self.alt = None;
                self.changes += 1;
            }
        }
    }

    /// The answer of field `n` checked, as Enter checks it: the game's check (which may
    /// show the CP left), then the form's.
    fn check(&mut self, editor: &mut StatEditor, n: usize, out: &mut OutputBatch) -> Checked {
        let mut answer = self.buf.trim_end_matches(' ').to_string();
        if numeric(n) {
            answer = answer.trim_start_matches(' ').to_string();
        }
        let field = field_of(n);
        if let Some(table) = choices(n) {
            if field == Field::Exit {
                return match answer.bytes().next().map(|c| c.to_ascii_uppercase()) {
                    Some(b'S' | b'Y' | b'D') => Checked::Left(Exit::Save),
                    Some(b'Q' | b'X' | b'A') => Checked::Left(Exit::Quit),
                    Some(b'E' | b'N') => Checked::Edit,
                    _ => self.pick(table, &answer),
                };
            }
            return self.pick(table, &answer);
        }
        let before = editor.values;
        match editor.verify_field(field, &answer) {
            Verdict::Reject(msg) => return Checked::Refused(message_text(&msg)),
            Verdict::ShowCp => out.push(Msg::CpToDistribute {
                cp: editor.unspent(),
            }),
            Verdict::Accept | Verdict::Again => {}
        }
        match field {
            Field::GivenName => Checked::Stored(editor.given_name.clone()),
            Field::FamilyName => Checked::Stored(editor.family_name.clone()),
            _ => {
                // digits only, at least 10, no higher than 250
                let value: i64 = answer.parse().unwrap_or(0);
                let refused = if answer.is_empty() || value < 10 {
                    Some("Enter at least 10")
                } else if value > 250 {
                    Some("Enter no higher than 250")
                } else {
                    None
                };
                match refused {
                    Some(m) => {
                        editor.values = before;
                        Checked::Refused(m.into())
                    }
                    None => Checked::Stored(answer),
                }
            }
        }
    }

    /// A choice field's answer: the one choice it begins (blanks taken out), or the list.
    fn pick(&self, table: &[&'static str], answer: &str) -> Checked {
        let answer: String = answer
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let first = answer.bytes().next().map(|c| c.to_ascii_uppercase());
        let found = starting(table, &answer);
        let same: Vec<usize> = found
            .iter()
            .copied()
            .filter(|&i| Some(table[i].as_bytes()[0].to_ascii_uppercase()) == first)
            .collect();
        if same.len() == 1 {
            return Checked::Stored(table[same[0]].to_string());
        }
        let mut list = String::from("Choices:  ");
        for i in found {
            if list.len() + table[i].len() + 2 <= CHOICES_MAX {
                list.push_str(table[i]);
                list.push_str("  ");
            } else {
                list.push_str("...");
                break;
            }
        }
        Checked::Refused(list)
    }

    /// An answer kept: into the form and the editor.
    fn store(&mut self, editor: &mut StatEditor, n: usize, stored: String) {
        if self.answers[n] != stored {
            self.changes += 1;
        }
        if let Some(table) = choices(n)
            && let Some(i) = exactly(table, &stored)
        {
            let i = i as u8;
            match n {
                22 => editor.hairlen = i,
                23 => editor.haircol = i,
                24 => editor.eyecol = i,
                _ => {}
            }
        }
        self.answers[n] = stored;
    }

    /// A refusal on lines of its own, with the bell, then the field asked again.
    fn refuse(&mut self, editor: &mut StatEditor, n: usize, msg: &str, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(b"\r\n\r\n");
        bytes.extend(silvermere_text::cp437::encode(msg));
        bytes.extend_from_slice(b"\x07\r\n");
        self.prompt(editor, n, bytes);
    }

    /// Enter on field `n`.
    fn enter(
        &mut self,
        editor: &mut StatEditor,
        n: usize,
        bytes: &mut Vec<u8>,
        out: &mut OutputBatch,
    ) -> Option<Exit> {
        // what came before goes out first: the check may add the CP line
        out.push_raw(std::mem::take(bytes));
        match self.check(editor, n, out) {
            Checked::Refused(msg) => self.refuse(editor, n, &msg, bytes),
            Checked::Left(exit) => {
                bytes.extend_from_slice(b"\r\n\x1b[0;1;32m");
                return Some(exit);
            }
            Checked::Edit => {
                let first = (0..FIELDS).find(|&p| Self::visits(editor, p)).unwrap_or(1);
                self.prompt(editor, first, bytes);
            }
            Checked::Stored(stored) => {
                self.store(editor, n, stored);
                let next = (n + 1..FIELDS)
                    .find(|&p| Self::visits(editor, p))
                    .unwrap_or(25);
                self.prompt(editor, next, bytes);
            }
        }
        None
    }

    /// Ctrl-G (save) or Ctrl-O (quit): the answer checked and kept first. Quitting with
    /// changes made asks again at the exit field.
    fn leave(
        &mut self,
        editor: &mut StatEditor,
        n: usize,
        exit: Exit,
        bytes: &mut Vec<u8>,
        out: &mut OutputBatch,
    ) -> Option<Exit> {
        out.push_raw(std::mem::take(bytes));
        match self.check(editor, n, out) {
            Checked::Refused(msg) => {
                self.refuse(editor, n, &msg, bytes);
                None
            }
            Checked::Stored(stored) => {
                self.store(editor, n, stored);
                if exit == Exit::Quit && n != 25 && self.changes > 0 {
                    bytes.extend_from_slice(
                        b"\r\n\r\nAre you sure?  Enter QUIT or ^O to quit now.\r\n",
                    );
                    self.prompt(editor, 25, bytes);
                    return None;
                }
                bytes.extend_from_slice(b"\r\n\x1b[0;1;32m");
                Some(exit)
            }
            Checked::Left(_) | Checked::Edit => {
                bytes.extend_from_slice(b"\r\n\x1b[0;1;32m");
                Some(exit)
            }
        }
    }
}

/// The fields of a form: every run of two or more `?` or `$`, the first [`FIELDS`].
fn cells(form: &str) -> Vec<(usize, usize)> {
    let b = form.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < FIELDS {
        let c = b[i];
        if c == b'?' || c == b'$' {
            let mut j = i;
            while j < b.len() && b[j] == c {
                j += 1;
            }
            if j - i >= 2 {
                out.push((i, j - i));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_session::clear_player;
    use crate::world::blank::clear_race;
    use silvermere_types::ids::RaceNo;

    fn editor() -> StatEditor {
        let mut rac = clear_race(RaceNo(1));
        (rac.minstr, rac.minint, rac.minwil) = (30, 30, 30);
        (rac.minagi, rac.minhea, rac.mincha) = (30, 30, 30);
        (rac.maxstr, rac.maxint, rac.maxwil) = (100, 100, 100);
        (rac.maxagi, rac.maxhea, rac.mincha) = (100, 100, 30);
        rac.maxcha = 100;
        let mut plr = clear_player();
        plr.firstname = "Kasimir".into();
        plr.lastname = "Smith".into();
        (plr.str, plr.intel, plr.wil) = (30, 30, 30);
        (plr.agi, plr.hea, plr.cha) = (30, 30, 30);
        plr.totalcp = 100;
        plr.hairlen = 1;
        StatEditor::open_existing(&plr, &rac, false)
    }

    fn typed(f: &mut LineForm, e: &mut StatEditor, keys: &[u8]) -> (String, Option<Exit>) {
        let mut out = OutputBatch::default();
        let mut left = None;
        for &k in keys {
            left = left.or(f.key(e, k, &mut out));
        }
        f.drained();
        let text = out.render(Default::default(), b"");
        (String::from_utf8_lossy(&text).into_owned(), left)
    }

    #[test]
    fn cells_of_the_stock_form() {
        let c = cells(STOCK_FORM);
        assert_eq!(c.len(), FIELDS);
        let widths: Vec<usize> = c.iter().map(|c| c.1).collect();
        assert_eq!(&widths[..4], &[10, 18, 31, 31]);
        assert!(widths[4..22].iter().all(|&w| w == 4));
        assert_eq!(&widths[22..], &[15, 15, 15, 4]);
    }

    #[test]
    fn an_existing_character_starts_at_the_family_name() {
        let mut e = editor();
        let mut out = OutputBatch::default();
        let f = LineForm::open(&mut e, &mut out);
        assert_eq!(f.buf, "Smith");
        assert_eq!(
            out.render(Default::default(), b""),
            b"\r\nFamily Name Smith"
        );
    }

    #[test]
    fn typing_replaces_and_enter_moves_on_with_the_cp() {
        let mut e = editor();
        let mut f = LineForm::open(&mut e, &mut OutputBatch::default());
        let (text, _) = typed(&mut f, &mut e, b"jones\r");
        assert_eq!(
            text,
            "\x08 \x08\x08 \x08\x08 \x08\x08 \x08\x08 \x08jones\r\nYou have 100 CP to distribute.\r\n\r\n Strength:  30"
        );
        assert_eq!(e.family_name, "Jones");
        let (text, _) = typed(&mut f, &mut e, b"5\r");
        assert!(
            text.ends_with("\r\n\r\nStrength may not be lower than 30.\x07\r\n\r\n Strength:  30"),
            "{text:?}"
        );
    }

    #[test]
    fn choices_by_space_and_by_letters() {
        let mut e = editor();
        let mut f = LineForm::open(&mut e, &mut OutputBatch::default());
        for _ in 0..7 {
            typed(&mut f, &mut e, b"\r");
        }
        assert_eq!(e.field, Field::HairLength);
        let (text, _) = typed(&mut f, &mut e, b"  ");
        assert_eq!(
            text,
            "\x08 \x08\x08 \x08\x08 \x08\x08 \x08\x08 \x08Shoulder-Length"
        );
        let (text, _) = typed(&mut f, &mut e, b"sh");
        assert!(text.ends_with("sh"), "{text:?}");
        let (text, _) = typed(&mut f, &mut e, b"\r");
        assert!(
            text.starts_with("\r\n\r\nChoices:  Short  Shoulder-Length  \x07"),
            "{text:?}"
        );
        let (_, _) = typed(&mut f, &mut e, b"l\r");
        assert_eq!(e.hairlen, 3);
        typed(&mut f, &mut e, b"\r\r");
        assert_eq!(e.field, Field::Exit);
        let (text, left) = typed(&mut f, &mut e, b"\r");
        assert_eq!(left, Some(Exit::Save));
        assert_eq!(text, "\r\n\x1b[0;1;32m");
    }
}
