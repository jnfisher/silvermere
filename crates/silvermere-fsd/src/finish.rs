//! Finishing an entry: the verify routine, the field's checks, storing and moving on.

use crate::MAX_HELP;
use crate::check::{match_choices, max_ok, min_ok, type_ok};
use crate::form::{Form, Mode, Outcome, State, Verdict, Verify};
use crate::key::{BAKTAB, CRSRDN, CRSRUP, CTRL_G, CTRL_O, CTRL_U, ESC, TAB};
use crate::render::BEL;
use crate::template::Kind;

/// What [`Form::done_at_end`] says when ^O or Escape would toss changes.
const ARE_YOU_SURE: &[u8] = b"Are you sure?  Enter QUIT or ^O to quit now.";

impl Form {
    /// Does the work a key left (as `FSD.C`'s `fsdprc`): a finished entry is checked by
    /// `verify` and the field's options, then stored, or refused with a message, and the
    /// cursor moves on. The host calls it after each [`Form::key`]; it reports when the
    /// session is over.
    pub fn process(&mut self, verify: &mut Verify, out: &mut Vec<u8>) -> Outcome {
        if self.state == State::Pending {
            self.finish_entry(verify, out);
            if self.state == State::Pending {
                self.state = match self.mode {
                    Mode::FullScreen => State::FieldSelected,
                    Mode::Linear => State::LinePrompt,
                };
            }
        }
        match self.state {
            State::Save => Outcome::Save,
            State::Quit => Outcome::Quit,
            _ => Outcome::Continue,
        }
    }

    fn finish_entry(&mut self, verify: &mut Verify, out: &mut Vec<u8>) {
        let entered = self.entered;
        let mut answer = self.entry.text.clone();
        let keep = answer
            .iter()
            .rposition(|&b| !b.is_ascii_whitespace() && b != 0x1A)
            .map_or(0, |i| i + 1);
        answer.truncate(keep);
        if self.fields[entered].place.kind == Kind::Number {
            let lead = answer
                .iter()
                .take_while(|&&b| b.is_ascii_whitespace() || b == 0x0B)
                .count();
            answer.drain(..lead);
        }
        let (mut verdict, mut message) = if self.skip_check {
            self.skip_check = false;
            (Verdict::Revert, Vec::new())
        } else {
            verify(self, entered, &mut answer, out)
        };
        let mut next = self.cursor;
        let mut over = self.state != State::Pending;
        let mut reprompt = next != entered;
        let mut changed = false;
        match verdict {
            Verdict::Accept => changed = self.store(entered, answer),
            Verdict::Check => match self.checked(entered, answer) {
                Ok(answer) => changed = self.store(entered, answer),
                Err(why) => {
                    if !why.is_empty() {
                        message = why;
                    }
                    verdict = Verdict::Reject;
                }
            },
            Verdict::Reject | Verdict::Revert => {}
        }
        if verdict == Verdict::Reject {
            next = entered;
            if !message.is_empty() {
                self.announce(&message, true, out);
                reprompt = true;
            } else {
                out.push(BEL);
                if self.mode == Mode::Linear {
                    self.rub_out_entry(out);
                }
            }
        }
        if changed {
            self.fields[entered].changed = true;
            if !over {
                self.changes = self.changes.saturating_add(1);
            }
        }
        if verdict != Verdict::Reject {
            if !message.is_empty() {
                self.announce(&message, false, out);
                reprompt = true;
            }
            match self.exit_key {
                CTRL_O | ESC => {
                    self.end_session(State::Quit, out);
                    over = true;
                }
                CTRL_G => {
                    self.end_session(State::Save, out);
                    over = true;
                }
                _ => {}
            }
        }
        let step = if next > 0 && next < entered { -1 } else { 1 };
        let next = self.open_field(next as isize, step).unwrap_or(entered);
        match self.mode {
            Mode::FullScreen => {
                if next != entered || over {
                    self.show_plain(entered, out);
                }
                if !over {
                    self.select(next, out);
                    self.go_to(next, out);
                    self.clear_entry();
                }
            }
            // (sic) Return on the last field comes back to it, as `next` is past the end
            Mode::Linear if !over => {
                self.cursor = next;
                self.prompt_with_answer(reprompt, out);
            }
            Mode::Linear => {}
        }
    }

    /// The entry if the field's type, `MIN=` and `MAX=` allow it, or if it stands for one
    /// choice (completed to it); else the message, empty for none.
    fn checked(&self, field: usize, answer: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let field = &self.fields[field];
        let why = if !type_ok(field, &answer) {
            Vec::new()
        } else {
            match min_ok(field, &answer).and_then(|()| max_ok(field, &answer)) {
                Ok(()) => return Ok(answer),
                Err(why) => why,
            }
        };
        let room = match (self.mode, &self.help) {
            (Mode::FullScreen, help) => help.as_ref().map_or(0, |h| h.width),
            (Mode::Linear, _) => MAX_HELP,
        };
        let found = match_choices(field, &answer, Some(room));
        if found.count == 1 {
            Ok(found.answer)
        } else if found.report.is_empty() {
            Err(why)
        } else {
            Err(found.report)
        }
    }

    /// The standard verify routine, for forms ending in a "done?" field (as `FSD.C`'s
    /// `vfyadn`).
    ///
    /// On the last field an answer starting S, Y or D saves and Q, X or A quits
    /// ([`Verdict::Accept`]); E or N goes back to the first field, and leaving it by
    /// cursor, tab, ^G, ^O or Escape keeps the old answer ([`Verdict::Revert`]). On another
    /// field ^O or Escape after changes asks "Are you sure?" and puts the cursor on the
    /// last field. Otherwise [`Verdict::Check`]. Ending the session in line mode starts a
    /// new line in `out`.
    pub fn done_at_end(
        &mut self,
        field: usize,
        answer: &[u8],
        out: &mut Vec<u8>,
    ) -> (Verdict, Vec<u8>) {
        if field + 1 == self.on_screen {
            match self.exit_key {
                TAB | CRSRDN if self.mode == Mode::FullScreen => {
                    return (Verdict::Revert, Vec::new());
                }
                CRSRUP | BAKTAB | CTRL_U | CTRL_G | CTRL_O | ESC => {
                    return (Verdict::Revert, Vec::new());
                }
                _ => {}
            }
            match answer.first().map(u8::to_ascii_uppercase) {
                Some(b'S' | b'Y' | b'D') => {
                    self.end_session(State::Save, out);
                    return (Verdict::Accept, Vec::new());
                }
                Some(b'Q' | b'X' | b'A') => {
                    self.end_session(State::Quit, out);
                    return (Verdict::Accept, Vec::new());
                }
                Some(b'E' | b'N') => {
                    self.cursor = 0;
                    return (Verdict::Revert, Vec::new());
                }
                _ => {}
            }
        } else if matches!(self.exit_key, CTRL_O | ESC)
            && (self.changes > 0 || answer != self.answers[field])
        {
            self.cursor = self.on_screen.saturating_sub(1);
            self.exit_key = 0;
            return (Verdict::Check, ARE_YOU_SURE.to_vec());
        }
        (Verdict::Check, Vec::new())
    }
}
