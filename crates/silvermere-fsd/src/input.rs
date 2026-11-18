//! Starting entry, keystrokes and moving between fields.

use crate::entry::Typed;
use crate::form::{Form, Mode, State};
use crate::key::{
    BAKTAB, BS, CR, CRSRDN, CRSRLF, CRSRRT, CRSRUP, CTRL_F, CTRL_G, CTRL_O, CTRL_U, CTRLEND,
    CTRLHOME, CTRLPGDN, CTRLPGUP, DEL, END, ESC, HOME, RUBOUT, TAB,
};
use crate::render::{CURSOR_ATTR, Justify};

/// Fields wider than this are not reshown while output is going out.
const WIDE_FIELD: usize = 40;

/// A key as a printable byte, if it is one.
fn printable(c: u16) -> Option<u8> {
    u8::try_from(c).ok().filter(|&b| b >= 0x20)
}

impl Form {
    /// Starts entry (as the library's `fsdego`): full screen puts the cursor on the first
    /// field that is not avoided; line mode prompts it with its answer.
    pub fn begin(&mut self, out: &mut Vec<u8>) {
        let first = self.open_field(0, 1).unwrap_or(0);
        match self.mode {
            Mode::FullScreen => {
                self.select(first, out);
                self.go_to(first, out);
                self.clear_entry();
                self.state = State::FieldSelected;
            }
            Mode::Linear => {
                self.output_pending = false;
                self.move_held = false;
                self.skip_check = false;
                self.cursor = first;
                self.prompt_with_answer(true, out);
                self.state = State::LinePrompt;
            }
        }
        self.changes = 0;
    }

    /// Takes one keystroke (as the library's `fsdinc`): a byte 0x20-0xFF, or a code from
    /// [`crate::key`]. Keys are ignored while an entry is pending or once the session is
    /// over.
    pub fn key(&mut self, c: u16, out: &mut Vec<u8>) {
        let wiped = self.state == State::MessageShown;
        if wiped {
            self.state = State::FieldSelected;
            self.wipe_help(out);
            self.go_to(self.cursor, out);
        }
        match self.state {
            State::FieldSelected => self.selected_key(c, out),
            State::Editing => self.editing_key(c, out),
            State::LinePrompt => match printable(c).filter(|&b| b != b' ') {
                Some(b) => {
                    self.state = State::LineEditing;
                    self.entry.pos = 0;
                    if self.classify(b, out) == Typed::Enter {
                        self.start_typing(out);
                        self.put_char(b, out);
                    }
                    self.entry.pos = self.entry.text.len();
                }
                None => self.line_key(c, out),
            },
            State::LineEditing => self.line_key(c, out),
            State::Pending | State::MessageShown | State::Save | State::Quit => {}
        }
        if wiped {
            self.output_pending = true;
        }
    }

    /// Tells the session the output so far has been sent (as the library's `fsdqoe`); it may
    /// then make a cursor move it held back. The host calls it after sending each key's
    /// output.
    pub fn output_sent(&mut self, out: &mut Vec<u8>) {
        self.output_pending = false;
        if !self.move_held {
            return;
        }
        self.move_held = false;
        if matches!(self.state, State::FieldSelected | State::MessageShown) {
            self.show_plain(self.shown_at, out);
            self.select(self.cursor, out);
            self.go_to(self.cursor, out);
        }
    }

    /// Full screen, the cursor on a field.
    fn selected_key(&mut self, c: u16, out: &mut Vec<u8>) {
        let at = self.cursor as isize;
        match c {
            CRSRUP | BAKTAB | CTRL_U => self.hop(at - 1, -1, out),
            CRSRDN | TAB => self.hop(at + 1, 1, out),
            CTRLPGUP | CTRLHOME => self.hop(0, 1, out),
            CTRLPGDN | CTRLEND => self.hop(self.on_screen as isize - 1, -1, out),
            CR => {
                self.load_answer();
                self.exit_key = c;
                self.leave_field(1, out);
            }
            CTRL_O | CTRL_G | ESC => {
                self.load_answer();
                self.exit_key = c;
                self.leave_field(0, out);
            }
            CRSRRT | CRSRLF | HOME | END | BS => {
                if !self.can_edit() {
                    return;
                }
                self.load_answer();
                if !self.may_edit() {
                    return;
                }
                self.select_for_editing(out);
                self.go_to(self.cursor, out);
                if !self.entry.text.is_empty() {
                    if c == CRSRRT {
                        self.step_over(out);
                    } else if c == END {
                        self.step_to_end(out);
                    }
                }
                self.state = State::Editing;
            }
            CTRL_F => {
                let full = self.answers[self.cursor].len() == self.current().place.width;
                if !self.can_edit() || full {
                    return;
                }
                self.load_answer();
                if !self.may_edit() {
                    return;
                }
                self.entry.text.insert(0, b' ');
                self.reshow_and_edit(out);
            }
            RUBOUT | DEL => {
                if !self.can_edit() || self.answers[self.cursor].is_empty() {
                    return;
                }
                self.load_answer();
                if !self.may_edit() {
                    return;
                }
                if !self.entry.text.is_empty() {
                    self.entry.text.remove(0);
                }
                self.reshow_and_edit(out);
            }
            _ => {
                let Some(b) = printable(c) else {
                    return;
                };
                if self.output_pending {
                    return;
                }
                self.load_answer();
                self.entry.pos = 0;
                match self.classify(b, out) {
                    Typed::Enter => {
                        self.start_typing(out);
                        self.put_char(b, out);
                        self.state = State::Editing;
                    }
                    Typed::Done => self.state = State::Editing,
                    Typed::Ignore => {}
                }
            }
        }
    }

    /// Whether keys may edit the current field: no output going out, not multiple choice.
    fn can_edit(&self) -> bool {
        !self.output_pending && !self.current().spec.multichoice
    }

    /// Shows the current field left-justified in the cursor colour, as editing starts at
    /// its first character.
    fn select_for_editing(&self, out: &mut Vec<u8>) {
        self.show_field(self.cursor, CURSOR_ATTR, Justify::Left, out);
    }

    /// Full screen, typing in a field.
    fn editing_key(&mut self, c: u16, out: &mut Vec<u8>) {
        let multichoice = self.current().spec.multichoice;
        let len = self.entry.text.len();
        let pos = self.entry.pos;
        match c {
            CRSRUP | BAKTAB | CTRL_U => {
                self.exit_key = c;
                self.leave_field(-1, out);
            }
            CR | TAB | CRSRDN => {
                self.exit_key = c;
                self.leave_field(1, out);
            }
            CTRL_O | CTRL_G | ESC => {
                self.exit_key = c;
                self.leave_field(0, out);
            }
            CRSRRT => {
                if pos < len && self.may_edit() && !multichoice {
                    self.entry.pos += 1;
                    out.extend_from_slice(b"\x1B[C");
                }
            }
            CRSRLF => {
                if pos > 0 && self.may_edit() && !multichoice {
                    self.entry.pos -= 1;
                    out.extend_from_slice(b"\x1B[D");
                }
            }
            HOME => {
                if !multichoice && self.may_edit() {
                    self.entry.pos = 0;
                    self.go_to(self.cursor, out);
                }
            }
            END => {
                if !multichoice && self.may_edit() {
                    self.step_to_end(out);
                }
            }
            CTRL_F => {
                if len >= self.current().place.width || !self.may_edit() {
                    return;
                }
                self.entry.text.insert(pos, b' ');
                self.redraw_rest(out);
            }
            RUBOUT | DEL => {
                if pos == len || !self.may_edit() {
                    return;
                }
                if pos == len - 1 {
                    out.extend_from_slice(b" \x08");
                } else {
                    self.entry.text.remove(pos);
                    self.entry.text.push(b' ');
                    self.redraw_rest(out);
                }
                self.entry.text.truncate(len - 1);
            }
            BS => {
                if pos == 0 || !self.may_edit() {
                    return;
                }
                if pos == len {
                    self.entry.pos -= 1;
                    out.extend_from_slice(b"\x08 \x08");
                } else {
                    self.entry.text.remove(pos - 1);
                    self.entry.text.push(b' ');
                    self.entry.pos -= 1;
                    out.push(0x08);
                    self.redraw_rest(out);
                }
                self.entry.text.truncate(len - 1);
            }
            _ => {
                if let Some(b) = printable(c)
                    && self.classify(b, out) == Typed::Enter
                {
                    self.put_char(b, out);
                }
            }
        }
    }

    /// Line mode, typing in a field (and control keys at its prompt).
    fn line_key(&mut self, c: u16, out: &mut Vec<u8>) {
        match c {
            CR | TAB | CRSRDN => {
                self.exit_key = c;
                if self.cursor + 1 < self.on_screen {
                    self.leave_field(1, out);
                } else {
                    self.leave_field(0, out);
                    self.cursor += 1;
                }
            }
            CRSRUP | BAKTAB | CTRL_U => {
                if self.open_field(self.cursor as isize - 1, -1).is_none() {
                    return;
                }
                if self.state == State::LineEditing {
                    self.rub_out_entry(out);
                }
                self.exit_key = c;
                self.leave_field(-1, out);
                self.skip_check = true;
            }
            BS | RUBOUT | DEL => {
                self.state = State::LineEditing;
                if self.entry.pos > 0 && self.may_edit() {
                    self.entry.pos -= 1;
                    self.entry.text.pop();
                    out.extend_from_slice(b"\x08 \x08");
                }
            }
            CTRL_O | CTRL_G | ESC => {
                self.exit_key = c;
                self.leave_field(0, out);
            }
            _ => {
                if let Some(b) = printable(c) {
                    self.state = State::LineEditing;
                    if self.classify(b, out) == Typed::Enter {
                        self.put_char(b, out);
                    }
                }
            }
        }
    }

    /// Field `n` of the template, wrapped round the ends in full screen; `None` past them
    /// in line mode (as the library's `vldfld`).
    fn wrap(&self, n: isize) -> Option<usize> {
        let last = self.on_screen.checked_sub(1)?;
        let wrap = self.mode == Mode::FullScreen;
        if n < 0 {
            wrap.then_some(last)
        } else if n as usize > last {
            wrap.then_some(0)
        } else {
            Some(n as usize)
        }
    }

    /// The first field from `from` on, stepping by `step`, that is not avoided (as
    /// the library's `movfld`); `None` if it comes back round or runs off the end.
    pub(crate) fn open_field(&self, from: isize, step: isize) -> Option<usize> {
        let mut at = self.wrap(from)?;
        let mut last = at;
        // (the original spins forever when every field is avoided and `from` is outside)
        for _ in 0..=self.on_screen {
            if !self.fields[at].avoided {
                return Some(at);
            }
            at = self.wrap(at as isize + step)?;
            if at as isize == from || at == last {
                return None;
            }
            last = at;
        }
        None
    }

    /// Moves the cursor to field `to` (or on by `step`). While output is going out only
    /// the cursor moves, and the fields are reshown when it has been sent (as the library's
    /// `hopfld`).
    fn hop(&mut self, to: isize, step: isize, out: &mut Vec<u8>) {
        let Some(to) = self.open_field(to, step) else {
            return;
        };
        if self.output_pending {
            let wide = |f: usize| self.fields[f].place.width > WIDE_FIELD;
            if wide(self.cursor) || wide(to) {
                return;
            }
            self.cursor = to;
            self.go_to(to, out);
            self.move_held = true;
        } else {
            self.show_plain(self.cursor, out);
            self.select(to, out);
            self.go_to(to, out);
            self.output_pending = true;
        }
    }

    /// Ends the entry and moves the cursor on by `step` (+1, -1 or 0), leaving the entry
    /// for [`Form::process`] (as the library's `xitfld`).
    fn leave_field(&mut self, step: isize, out: &mut Vec<u8>) {
        self.entered = self.cursor;
        if let Some(next) = self.open_field(self.cursor as isize + step, step) {
            self.cursor = next;
            if self.mode == Mode::FullScreen {
                self.go_to(next, out);
            }
        }
        self.state = State::Pending;
    }

    /// Ends the session with `state`, [`State::Save`] or [`State::Quit`]; line mode starts
    /// a new line (as the library's `xitfsd`).
    pub(crate) fn end_session(&mut self, state: State, out: &mut Vec<u8>) {
        if self.mode == Mode::Linear {
            out.extend_from_slice(b"\r\n");
        }
        self.state = state;
    }
}
