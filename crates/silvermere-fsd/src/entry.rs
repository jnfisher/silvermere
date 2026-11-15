//! Typing in a field: characters, choices and the entry's cursor, and showing fields.

use std::mem::{replace, take};

use crate::check::{exact_choice, match_choices, prefix_choice};
use crate::form::{Field, Form, Mode, State};
use crate::render::{BEL, CURSOR_ATTR, Justify, answer_in, colour, cursor_move, rubout, spaces};
use crate::template::Kind;

/// What became of a typed character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Typed {
    /// To be put in the entry as typed.
    Enter,
    /// Nothing.
    Ignore,
    /// Already handled: a choice was put in.
    Done,
}

impl Form {
    /// The field the cursor is on.
    pub(crate) fn current(&self) -> &Field {
        &self.fields[self.cursor]
    }

    /// Decides what a printable character does in the current field (as `FSD.C`'s
    /// `hdlprt`): digits and a leading minus in a number, choice keys in a
    /// multiple-choice field, anything in text.
    pub(crate) fn classify(&mut self, c: u8, out: &mut Vec<u8>) -> Typed {
        let field = self.current();
        match field.place.kind {
            Kind::Text if field.spec.multichoice => self.choice_key(c, out),
            Kind::Text => Typed::Enter,
            Kind::Number => {
                let digit_ok = c.is_ascii_digit()
                    || (c == b'-' && self.entry.pos == 0 && !field.spec.no_negative);
                if digit_ok && (!self.entry.typing_choice || self.entry.text.is_empty()) {
                    if self.entry.typing_choice || self.entry.choice.is_some() {
                        self.start_typing(out);
                        self.entry.typing_choice = false;
                        self.entry.choice = None;
                    }
                    Typed::Enter
                } else {
                    self.choice_key(c, out)
                }
            }
        }
    }

    /// A key towards a choice (as `FSD.C`'s `hdlalt`): Space shows the next choice of a
    /// multiple-choice field; another key extends the typed prefix, putting in the choice
    /// when only one matches and typing on when several do.
    fn choice_key(&mut self, c: u8, out: &mut Vec<u8>) -> Typed {
        let field = self.current();
        let count = field.spec.choices.len();
        if count == 0 {
            return Typed::Ignore;
        }
        if c == b' ' {
            if !field.spec.multichoice || self.output_pending {
                return Typed::Ignore;
            }
            let next = match self.entry.choice {
                Some(shown) => Some(shown + 1).filter(|&n| n < count),
                None => prefix_choice(field, &self.entry.text),
            };
            let next = next.unwrap_or(0);
            if self.entry.choice == Some(next) {
                return Typed::Ignore;
            }
            let choice = field.spec.choices[next].clone();
            self.show_choice(next, choice, out);
            return Typed::Done;
        }
        let mut typed = if self.entry.typing_choice {
            self.entry.text.clone()
        } else {
            Vec::new()
        };
        typed.push(c);
        let found = match_choices(field, &typed, None);
        match found.count {
            0 => Typed::Ignore,
            1 => {
                let first = found.first.unwrap_or(0);
                self.show_choice(first, found.answer, out);
                Typed::Done
            }
            _ => {
                if !self.entry.typing_choice {
                    self.entry.typing_choice = true;
                    self.entry.choice = None;
                    if !self.entry.text.is_empty() {
                        self.start_typing(out);
                    }
                }
                Typed::Enter
            }
        }
    }

    /// Puts choice `index`, `text`, in the entry and shows it with the cursor after it (as
    /// `FSD.C`'s `altntr`).
    fn show_choice(&mut self, index: usize, text: Vec<u8>, out: &mut Vec<u8>) {
        let shown = replace(&mut self.entry.text, text).len();
        self.entry.choice = Some(index);
        self.restart(shown, out);
        let place = &self.fields[self.cursor].place;
        match self.mode {
            Mode::FullScreen => {
                answer_in(place, &self.entry.text, Justify::Left, out);
                let back = place.width.saturating_sub(self.entry.text.len());
                cursor_move(back, b'D', out);
            }
            Mode::Linear => out.extend_from_slice(&self.entry.text),
        }
        self.entry.typing_choice = false;
        self.output_pending = true;
    }

    /// Empties the entry and shows the field blank, to type it afresh.
    pub(crate) fn start_typing(&mut self, out: &mut Vec<u8>) {
        let shown = take(&mut self.entry.text).len();
        self.restart(shown, out);
    }

    /// Begins typing from the entry's text, `shown` bytes of the old being on screen:
    /// full screen blanks the field, line mode rubs the old out; the cursor goes to the end
    /// (as `FSD.C`'s `bgnter`).
    fn restart(&mut self, shown: usize, out: &mut Vec<u8>) {
        match self.mode {
            Mode::FullScreen => {
                let place = &self.fields[self.cursor].place;
                out.extend_from_slice(&place.goto);
                colour(CURSOR_ATTR, out);
                spaces(place.width, out);
                out.extend_from_slice(&place.goto);
            }
            Mode::Linear => rubout(shown, out),
        }
        self.entry.pos = self.entry.text.len();
    }

    /// Puts a typed character in the entry at the cursor and echoes it (as `FSD.C`'s
    /// `addprt`); nothing once the field is full.
    pub(crate) fn put_char(&mut self, c: u8, out: &mut Vec<u8>) {
        let entry = &mut self.entry;
        if entry.pos >= self.fields[self.cursor].place.width {
            return;
        }
        if entry.pos < entry.text.len() {
            entry.text[entry.pos] = c;
        } else {
            entry.text.push(c);
        }
        entry.pos += 1;
        out.push(c);
    }

    /// Puts the current field's answer in the entry (as `FSD.C`'s `defntr`).
    pub(crate) fn load_answer(&mut self) {
        self.entry.text = self.answers[self.cursor].clone();
        self.entry.choice = exact_choice(self.current(), &self.entry.text);
        self.entry.typing_choice = false;
    }

    /// Empties the entry (as `FSD.C`'s `entprp`).
    pub(crate) fn clear_entry(&mut self) {
        self.entry = Default::default();
    }

    /// May the entry be edited? Not a choice shown in a multiple-choice or number field;
    /// a choice shown in a text field becomes plain text (as `FSD.C`'s `bstalt`).
    pub(crate) fn may_edit(&mut self) -> bool {
        if self.entry.choice.is_some() {
            let field = self.current();
            if field.spec.multichoice || field.place.kind != Kind::Text {
                return false;
            }
            self.entry.choice = None;
        }
        true
    }

    /// Shows the entry from the cursor on and moves back (as `FSD.C`'s `dprest`).
    pub(crate) fn redraw_rest(&self, out: &mut Vec<u8>) {
        let rest = self.entry.text.get(self.entry.pos..).unwrap_or_default();
        out.extend_from_slice(rest);
        cursor_move(rest.len(), b'D', out);
    }

    /// Line mode: rubs out what was typed (as `FSD.C`'s `unentr`).
    pub(crate) fn rub_out_entry(&self, out: &mut Vec<u8>) {
        rubout(self.entry.text.len(), out);
    }

    /// Echoes the character at the cursor, moving past it.
    pub(crate) fn step_over(&mut self, out: &mut Vec<u8>) {
        if let Some(&c) = self.entry.text.get(self.entry.pos) {
            out.push(c);
        }
        self.entry.pos += 1;
    }

    /// Moves the cursor to the end of the entry.
    pub(crate) fn step_to_end(&mut self, out: &mut Vec<u8>) {
        let len = self.entry.text.len();
        cursor_move(len.saturating_sub(self.entry.pos), b'C', out);
        self.entry.pos = len;
    }

    /// Outputs field `field`'s go-to command.
    pub(crate) fn go_to(&self, field: usize, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.fields[field].place.goto);
    }

    /// Shows field `field`'s answer in `attr` (as `FSD.C`'s `shofld`).
    pub(crate) fn show_field(&self, field: usize, attr: u8, justify: Justify, out: &mut Vec<u8>) {
        let place = &self.fields[field].place;
        out.extend_from_slice(&place.goto);
        colour(attr, out);
        answer_in(place, &self.answers[field], justify, out);
    }

    /// Shows field `field` in its own colour.
    pub(crate) fn show_plain(&self, field: usize, out: &mut Vec<u8>) {
        let attr = self.fields[field].place.attr;
        self.show_field(field, attr, Justify::Field, out);
    }

    /// Puts the cursor on field `field`, shown in the cursor colour (as `FSD.C`'s
    /// `cursat`).
    pub(crate) fn select(&mut self, field: usize, out: &mut Vec<u8>) {
        self.cursor = field;
        self.shown_at = field;
        self.show_field(field, CURSOR_ATTR, Justify::Field, out);
    }

    /// Shows the entry in the field, left-justified, and starts editing at its start.
    pub(crate) fn reshow_and_edit(&mut self, out: &mut Vec<u8>) {
        let place = &self.fields[self.cursor].place;
        out.extend_from_slice(&place.goto);
        colour(CURSOR_ATTR, out);
        answer_in(place, &self.entry.text, Justify::Left, out);
        out.extend_from_slice(&place.goto);
        self.state = State::Editing;
    }

    /// Shows a message: full screen in the help area, cut to its width, until the next
    /// key; line mode on lines of its own. With `beep` the bell too (as `FSD.C`'s
    /// `announce`).
    pub(crate) fn announce(&mut self, msg: &[u8], beep: bool, out: &mut Vec<u8>) {
        match (self.mode, &self.help) {
            (Mode::FullScreen, Some(help)) => {
                out.extend_from_slice(&help.goto);
                colour(help.attr, out);
                out.extend_from_slice(&msg[..msg.len().min(help.width)]);
                if beep {
                    out.push(BEL);
                }
                self.state = State::MessageShown;
            }
            (Mode::FullScreen, None) => {
                if beep {
                    out.push(BEL);
                }
            }
            (Mode::Linear, _) => {
                out.extend_from_slice(b"\r\n\r\n");
                out.extend_from_slice(msg);
                if beep {
                    out.push(BEL);
                }
                out.extend_from_slice(b"\r\n");
            }
        }
    }

    /// Wipes the message from the help area.
    pub(crate) fn wipe_help(&mut self, out: &mut Vec<u8>) {
        if let Some(help) = &self.help {
            out.extend_from_slice(&help.goto);
            colour(help.attr, out);
            spaces(help.width, out);
        }
    }

    /// Line mode: outputs the template text between the previous field and the current
    /// one; before the first field a new line (as `FSD.C`'s `pmtfld`).
    pub(crate) fn prompt(&self, out: &mut Vec<u8>) {
        let start = match self.cursor.checked_sub(1) {
            Some(prev) => {
                let place = &self.fields[prev].place;
                place.offset + place.width
            }
            None => {
                out.extend_from_slice(b"\r\n");
                0
            }
        };
        let end = self.current().place.offset;
        if start < end {
            out.extend_from_slice(&self.template[start..end]);
        }
    }

    /// Line mode: prompts the current field with its answer.
    pub(crate) fn prompt_with_answer(&mut self, prompt: bool, out: &mut Vec<u8>) {
        if prompt {
            self.prompt(out);
        }
        self.load_answer();
        self.entry.pos = self.entry.text.len();
        out.extend_from_slice(&self.entry.text);
    }
}
