//! The host's full-screen form library (the original's `FSD.C` and `FSDBBS.C`), as far as
//! the game's character editor uses it.
//!
//! A form is three pieces of bytes:
//!
//! - the *template*, the screen as message text (CR line ends, ANSI colour and cursor
//!   sequences allowed) with field markers in it: a run of `?` is a text field, a run of
//!   `$` a right-justified number, a run of `!` the help area where messages appear;
//! - the *field specification*, the fields in template order, each a name with options:
//!   `NAME AGE(MIN=10, MAX=99) DONE(ALT=SAVE ALT=QUIT MULTICHOICE)`. `MIN=` and `MAX=`
//!   bound an entry, `ALT=` lists choices and `MULTICHOICE` makes them the only answers;
//! - the default answers, an answer string `NAME=value\0...\0\0`.
//!
//! [`Form::prepare`] reads all three, [`Form::background`] draws the screen and
//! [`Form::begin`] starts entry, full screen with ANSI cursor moves or line by line. The
//! host then passes each keystroke to [`Form::key`], runs [`Form::process`], which checks a
//! finished entry through the form's [`Verify`] routine, and calls [`Form::output_sent`]
//! once the output has gone out. Answers are read by field ([`Form::answer`]), by name
//! ([`Form::answer_named`]) or as a choice ([`Form::ordinal`]).
//!
//! ```
//! use silvermere_fsd::{Form, Mode, Outcome, key};
//!
//! let template = b"Name: ??????\rDone? ????";
//! let spec = b"NAME DONE(ALT=SAVE ALT=QUIT MULTICHOICE)";
//! let mut form = Form::prepare(template, spec, Mode::FullScreen, b"DONE=SAVE\0\0");
//! let mut out = Vec::new();
//! form.background(&mut out);
//! form.begin(&mut out);
//! let mut verify =
//!     |f: &mut Form, n: usize, a: &mut Vec<u8>, o: &mut Vec<u8>| f.done_at_end(n, a, o);
//! let mut outcome = Outcome::Continue;
//! for k in [u16::from(b'A'), u16::from(b'l'), key::CR, key::CR] {
//!     form.key(k, &mut out);
//!     outcome = form.process(&mut verify, &mut out);
//!     form.output_sent(&mut out);
//! }
//! assert_eq!(outcome, Outcome::Save);
//! assert_eq!(form.answer(0), b"Al");
//! ```
//!
//! Every output byte follows the original. Its other markup (`#` digit fields, `Y/N`,
//! fields with embedded punctuation) and options (`SECRET`, `NOSPACES`) are not supported:
//! `#` and `/` are plain text, and a `?` run directly after another `?` run is a field of
//! its own.

#![forbid(unsafe_code)]

mod answers;
mod check;
mod entry;
mod finish;
mod form;
mod input;
pub mod key;
mod render;
mod screen;
mod spec;
mod template;

pub use form::{Form, Mode, Outcome, State, Verdict, Verify};

/// Longest answer, and widest field.
const MAX_ANSWER: usize = 80;
/// Widest help area.
const MAX_HELP: usize = 80;
