//! The character editor on the host's form library ([`silvermere_fsd`]), as the game's
//! `edit_character_stats` runs it: the `EDITCHA1` form full screen on an ANSI terminal of
//! at least 80x23, else `EDTCHAN2` line by line.
//!
//! | Step | The game | Here |
//! |---|---|---|
//! | open | `fsdroom`, `fsdapr` with `statflds` and `statanss` | [`StatForm::prepare`] |
//! | full screen | `fsdbkg`: the template with its answers filled in | [`StatForm::background`] |
//! | line mode | the CP chart, the race ranges, the CP line | `edit_character_stats` |
//! | then | the display-only fields avoided, `fsdego`, the CP display (full screen) | [`StatForm::begin`] |
//! | a key | `ainchr` (ANSI keys to key codes), `fsdinc`, `fsdprc` | [`on_key`], [`on_line`] |
//! | a check | `ljnvfy`, then `vfyadn` | [`ljnvfy`] |
//! | the end | `goback`, then `ljndun` | [`newchar::editor_done`] |
//!
//! The keyboard is locked while the background goes out (`fsdbkg`'s `btulok`, lifted at the
//! host's next pass by the output-empty status): keys typed before then are dropped.
//!
//! Not carried: the sub-BBS client's screen start and end codes (`send_screen_start_code`).

use silvermere_fsd::key::{
    CRSRDN, CRSRLF, CRSRRT, CRSRUP, END, F1, F2, F3, F4, F5, F6, F7, F8, HOME,
};
use silvermere_fsd::{Form, Mode, Outcome as FormOutcome, Verdict as FormVerdict};
use silvermere_text::ansi::resolve_if_ansi;
use silvermere_text::{Msg, Palette, cp437, package, render, strip_ansi};
use silvermere_types::ids::SessionId;
use silvermere_types::names::{EYE_COLOURS, HAIR_COLOURS, HAIR_LENGTHS};
use silvermere_types::records::{Character, Class, Race};

use crate::EngineOutput;
use crate::player_session::Caps;
use crate::state::Outcome;
use crate::state::newchar::stat_editor::{Field, Stat, StatEditor, Verdict};
use crate::state::newchar::{self, leading_int};
use crate::world::World;

/// `statflds`: the form's fields.
pub const STATFLDS: &[u8] = b"LOCALNAME() LASTNAME() RACE CLASS \
BASE_STR MAX_STR TOT_STR(MIN=10, MAX=250) \
BASE_INT MAX_INT TOT_INT(MIN=10, MAX=250) \
BASE_WISDOM MAX_WISDOM TOT_WISDOM(MIN=10, MAX=250) \
BASE_AGIL MAX_AGIL TOT_AGIL(MIN=10, MAX=250) \
BASE_HEALTH MAX_HEALTH TOT_HEALTH(MIN=10, MAX=250) \
BASE_CHAR MAX_CHAR TOT_CHAR(MIN=10, MAX=250) \
HAIR_LEN(ALT=None ALT=Short ALT=Shoulder-Length ALT=Long ALT=Waist-length ALT=Ankle-Length MULTICHOICE) \
HAIR_COL(ALT=Black ALT=White ALT=Silver ALT=Red ALT=Brown ALT=Dark-Brown ALT=Blonde ALT=Green ALT=Blue ALT=Grey MULTICHOICE) \
EYE_COL(ALT=Black ALT=Crimson ALT=Yellow ALT=Pale-Blue ALT=Sea-Blue ALT=Dark-Blue ALT=Grey-Blue ALT=Slate-Grey ALT=Bright-Green ALT=Forest-Green ALT=Pale-Green ALT=Chesnut-Brown ALT=Dark-Brown ALT=Hazel ALT=Violet ALT=Lavender ALT=Golden MULTICHOICE) \
SAVE(ALT=SAVE ALT=EDIT ALT=QUIT MULTICHOICE)";

/// The stock package's `EDTCHAN2` as the host hands it over, for when no package text is
/// installed.
const STOCK_EDTCHAN2: &[u8] = b"\x1b[0m\x1b[2J\r\n      M A J O R   M U D\r\n\r\n     Character Generator\r\n\r\n      Point Cost Chart\r\n\r\n +1-10 points costs 1 CP each\r\n+11-20 points costs 2 CP each\r\n+21-30 points costs 3 CP each\r\n+31-40 points costs 4 CP each\r\n\r\nGiven Name  ??????????\r\nFamily Name ??????????????????\r\nRace        ???????????????????????????????\r\nClass       ???????????????????????????????\r\n\r\nNot Used: Base Strength:   $$$$\r\nNot Used: Max Strength:   $$$$\r\n Strength:  $$$$\r\nNot Used: Base Intellect:  $$$$\r\nNot Used: Max Intellect:  $$$$\r\n Intellect: $$$$\r\nNot Used: Base Willpower:  $$$$\r\nNot Used: Max Willpower:  $$$$\r\n Willpower: $$$$\r\nNot Used: Base Agility:    $$$$\r\nNot Used: Max Agility:    $$$$\r\n Agility:   $$$$\r\nNot Used: Base Health:     $$$$\r\nNot Used: Max Health:     $$$$\r\n Health:    $$$$\r\nNot Used: Base Charm:      $$$$\r\nNot Used: Max Charm:      $$$$\r\n Charm:     $$$$\r\n\r\n             (hit space to toggle)\r\n Hair Length ???????????????\r\n\r\n             (hit space to toggle)\r\n Hair Colour ???????????????\r\n\r\n             (hit space to toggle)\r\n Eye Colour  ???????????????\r\n\r\n Exit: ????\r\n\r\n CP Available: $$$$\r\n";

/// `SF_FIRSTNAME`, `SF_LASTNAME`.
const SF_FIRSTNAME: usize = 0;
const SF_LASTNAME: usize = 1;
/// `SF_HAIRLEN`, `SF_HAIRCOL`, `SF_EYECOL`.
const SF_HAIRLEN: usize = 22;
const SF_HAIRCOL: usize = 23;
const SF_EYECOL: usize = 24;
/// The race, the class and the race limits: shown, never entered.
const DISPLAY_ONLY: [usize; 14] = [2, 3, 4, 5, 7, 8, 10, 11, 13, 14, 16, 17, 19, 20];
/// The six totals' names, in [`Stat`] order.
const TOTALS: [&str; 6] = [
    "TOT_STR",
    "TOT_INT",
    "TOT_WISDOM",
    "TOT_AGIL",
    "TOT_HEALTH",
    "TOT_CHAR",
];
/// `eurmsk`: the host's default, U.S.A. characters only.
const EURMSK: u16 = 0x7F;
/// `ANSILN`: the host's screen rows.
const ANSILN: u16 = 25;

/// Full screen on an ANSI terminal of at least 23 rows and 80 columns.
pub fn fits(caps: &Caps) -> bool {
    caps.ansi && caps.rows >= 23 && caps.columns >= 80
}

/// The form while it is open, kept in [`StatEditor::form`].
#[derive(Debug, Clone)]
pub struct StatForm {
    form: Form,
    ain: Ain,
    /// The keyboard locked while the background goes out.
    locked: bool,
}

impl StatForm {
    /// `fsdroom` and `fsdapr`: `EDITCHA1` full screen, `EDTCHAN2` line by line, filled
    /// from `statanss`. The looks are already inside their tables.
    pub fn prepare(plr: &Character, rac: &Race, cls: &Class, full_screen: bool) -> Self {
        let text = package::installed();
        let (template, mode) = match text {
            Some(p) if full_screen => (getasc(&p.editcha1_bytes()), Mode::FullScreen),
            Some(p) => (getasc(&p.edtchan2_bytes()), Mode::Linear),
            None => (STOCK_EDTCHAN2.to_vec(), Mode::Linear),
        };
        let form = Form::prepare(&template, STATFLDS, mode, &statanss(plr, rac, cls));
        Self {
            form,
            ain: Ain::default(),
            locked: false,
        }
    }

    /// `btulok`: the keyboard locked until the background has gone out.
    pub fn lock_keyboard(&mut self) {
        self.locked = true;
    }

    /// Whether the form runs full screen.
    pub fn is_full_screen(&self) -> bool {
        self.form.mode() == Mode::FullScreen
    }

    /// `fsdbkg`: the screen cleared, the template with every answer in it.
    pub fn background(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.form.background(&mut out);
        out
    }

    /// The display-only fields avoided (and the first name, for an existing character),
    /// then `fsdego` and, full screen, the CP display; `cp` is the CP still to spend.
    pub fn begin(&mut self, newchar: bool, cp: i32, out: &mut Vec<u8>) {
        for field in DISPLAY_ONLY {
            self.form.avoid(field);
        }
        if !newchar {
            self.form.avoid(SF_FIRSTNAME);
        }
        self.form.begin(out);
        if self.is_full_screen() {
            display_cp(true, cp, out);
        }
    }

    /// The answers `ljndun` reads, into `editor`: the six totals, the names and the looks.
    fn read_answers(&mut self, editor: &mut StatEditor) {
        editor.values = totals(&self.form);
        editor.given_name = cp437::decode(self.form.answer(SF_FIRSTNAME));
        editor.family_name = cp437::decode(self.form.answer(SF_LASTNAME));
        for (field, look) in [
            (SF_HAIRCOL, &mut editor.haircol),
            (SF_HAIRLEN, &mut editor.hairlen),
            (SF_EYECOL, &mut editor.eyecol),
        ] {
            if let Some(i) = self.form.ordinal(field) {
                *look = u8::try_from(i).unwrap_or(*look);
            }
        }
    }
}

/// `getasc`: the message with each CR as CR LF, as the host hands a template over.
fn getasc(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 64);
    for &b in text {
        match b {
            b'\r' | b'\n' => out.extend_from_slice(b"\r\n"),
            b => out.push(b),
        }
    }
    out
}

/// `statanss` filled as `edit_character_stats` fills it.
fn statanss(plr: &Character, rac: &Race, cls: &Class) -> Vec<u8> {
    let look = |t: &[&str], i: u8| t.get(usize::from(i)).copied().unwrap_or("").to_string();
    let mut answers: Vec<(&str, String)> = vec![
        ("LOCALNAME", plr.firstname.clone()),
        ("LASTNAME", plr.lastname.clone()),
        ("RACE", rac.name.clone()),
        ("CLASS", cls.name.clone()),
    ];
    let stats = [
        ("STR", rac.minstr, rac.maxstr, plr.str),
        ("INT", rac.minint, rac.maxint, plr.intel),
        ("WISDOM", rac.minwil, rac.maxwil, plr.wil),
        ("AGIL", rac.minagi, rac.maxagi, plr.agi),
        ("HEALTH", rac.minhea, rac.maxhea, plr.hea),
        ("CHAR", rac.mincha, rac.maxcha, plr.cha),
    ];
    let mut names = Vec::new();
    for (stat, min, max, cur) in stats {
        names.push((format!("BASE_{stat}"), min.to_string()));
        names.push((format!("MAX_{stat}"), max.to_string()));
        names.push((format!("TOT_{stat}"), cur.to_string()));
    }
    answers.extend(names.iter().map(|(n, v)| (n.as_str(), v.clone())));
    answers.push(("HAIR_LEN", look(HAIR_LENGTHS, plr.hairlen)));
    answers.push(("HAIR_COL", look(HAIR_COLOURS, plr.haircol)));
    answers.push(("EYE_COL", look(EYE_COLOURS, plr.eyecol)));
    answers.push(("SAVE", "SAVE".into()));
    let mut out = Vec::new();
    for (name, value) in answers {
        out.extend_from_slice(name.as_bytes());
        out.push(b'=');
        out.extend(cp437::encode(&value));
        out.push(0);
    }
    out.push(0);
    out
}

/// `atol` of each `TOT_*` answer, into the game's unsigned.
fn totals(form: &Form) -> [u16; 6] {
    TOTALS.map(|name| {
        let answer = cp437::decode(form.answer_named(name));
        leading_int(answer.trim_start()) as u16
    })
}

/// `display_cp_to_distribute`: on the form at row 20, column 33, or as a line.
fn display_cp(full_screen: bool, cp: i32, out: &mut Vec<u8>) {
    if full_screen {
        out.extend_from_slice(b"\x1B[20;33f");
        out.extend(resolve_if_ansi(b"\x1B[[\x1B[0m| \x08 \x08 \x08]"));
        out.extend(format!("{cp:4}").into_bytes());
    } else {
        out.extend(render(&Msg::CpToDistribute { cp }, Palette::default()).bytes);
    }
}

/// `ljnvfy`: the game's check of fields 0, 1 and the six totals (redrawing the CP
/// display when one is accepted), then the standard `vfyadn`.
fn ljnvfy(
    editor: &mut StatEditor,
    form: &mut Form,
    field: usize,
    answer: &mut Vec<u8>,
    out: &mut Vec<u8>,
) -> (FormVerdict, Vec<u8>) {
    let checked = match field {
        0 => Field::GivenName,
        1 => Field::FamilyName,
        6 | 9 | 12 | 15 | 18 | 21 => Field::Stat(Stat::ALL[(field - 6) / 3]),
        _ => return form.done_at_end(field, answer, out),
    };
    // the six totals as they stand on the form now
    editor.values = totals(form);
    match editor.verify_field(checked, &cp437::decode(answer)) {
        Verdict::Reject(msg) => {
            let text = strip_ansi(&render(&msg, Palette::default()).bytes);
            return (FormVerdict::Reject, text);
        }
        Verdict::ShowCp => display_cp(form.mode() == Mode::FullScreen, editor.unspent(), out),
        Verdict::Accept => {}
    }
    // a name as the check rewrote it
    match checked {
        Field::GivenName => *answer = cp437::encode(&editor.given_name),
        Field::FamilyName => *answer = cp437::encode(&editor.family_name),
        Field::Stat(_) => {}
    }
    form.done_at_end(field, answer, out)
}

/// The host's ANSI key translator (`ainchr`): the cursor keys, Home and End
/// from `ESC [`, F1 to F8 from `ESC O`; `ESC` and another byte is that byte, so `ESC ESC`
/// is Escape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Ain {
    #[default]
    WaitEsc,
    WaitBracket,
    WaitAlpha,
    WaitO,
}

impl Ain {
    /// One byte in; the key code out, 0 for none yet.
    fn chr(&mut self, c: u8) -> u16 {
        let c16 = u16::from(c);
        match *self {
            Ain::WaitEsc => {
                if c != 0x1B {
                    return c16;
                }
                *self = Ain::WaitBracket;
                0
            }
            Ain::WaitBracket => match c {
                b'[' => {
                    *self = Ain::WaitAlpha;
                    0
                }
                b'O' => {
                    *self = Ain::WaitO;
                    0
                }
                _ => {
                    *self = Ain::WaitEsc;
                    c16
                }
            },
            Ain::WaitAlpha => {
                if c.is_ascii_digit() || c == b';' {
                    return 0;
                }
                *self = Ain::WaitEsc;
                match c {
                    b'A' => CRSRUP,
                    b'B' => CRSRDN,
                    b'C' => CRSRRT,
                    b'D' => CRSRLF,
                    b'H' | b'f' => HOME,
                    b'K' => END,
                    _ => 0,
                }
            }
            Ain::WaitO => {
                *self = Ain::WaitEsc;
                match c {
                    b'P' => F1,
                    b'Q' => F2,
                    b'W' => F3,
                    b'x' => F4,
                    b't' => F5,
                    b'u' => F6,
                    b'q' => F7,
                    b'r' => F8,
                    _ => 0,
                }
            }
        }
    }
}

/// The output-empty status of the host's next pass: the background has gone out and the
/// keyboard is unlocked, for every form but `except`'s (whose keys come first).
pub fn unlock_keyboards(world: &mut World, except: Option<SessionId>) {
    for (&id, s) in &mut world.sessions {
        if Some(id) != except
            && let Some(sf) = s.editor.as_mut().and_then(|e| e.form.as_mut())
        {
            sf.locked = false;
        }
    }
}

/// One keystroke in character mode: the key, the work it left, then the output counted
/// as sent.
pub fn on_key(world: &mut World, id: SessionId, byte: u8) -> Outcome {
    type_keys(world, id, &[byte], true);
    Outcome::Continue
}

/// A whole line: its keys, then Return, all arriving together; what the previous input
/// sent has gone.
pub fn on_line(world: &mut World, id: SessionId, line: &str) -> Outcome {
    let mut keys = line.as_bytes().to_vec();
    keys.push(b'\r');
    type_keys(world, id, &keys, false);
    Outcome::Continue
}

/// `fsdchi` for each byte, `fsdprc` after each key; at the end of the session `goback`
/// and the editor's end.
fn type_keys(world: &mut World, id: SessionId, bytes: &[u8], each_sent: bool) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(editor) = s.editor.as_mut() else {
        return;
    };
    // keys typed while the background goes out are dropped; the host's pass after them
    // lifts the lock
    if let Some(sf) = editor.form.as_mut()
        && sf.locked
    {
        sf.locked = false;
        return;
    }
    let Some(mut sf) = editor.form.take() else {
        return;
    };
    let mut out = Vec::new();
    if !each_sent {
        sf.form.output_sent(&mut out);
    }
    // the keys all go in before the work they leave is done (the host's cycle), so the
    // keys after one that finishes an entry are dropped (`fsdpch`)
    let mut keyed = false;
    for (n, &b) in bytes.iter().enumerate() {
        let c = sf.ain.chr(b);
        if c != 0 {
            sf.form
                .key(if c < 0x100 { c & EURMSK } else { c }, &mut out);
            keyed = true;
        }
        if each_sent && n + 1 < bytes.len() {
            sf.form.output_sent(&mut out);
        }
    }
    let mut outcome = FormOutcome::Continue;
    if keyed {
        let mut verify =
            |f: &mut Form, n: usize, a: &mut Vec<u8>, o: &mut Vec<u8>| ljnvfy(editor, f, n, a, o);
        outcome = sf.form.process(&mut verify, &mut out);
    }
    if each_sent && outcome == FormOutcome::Continue {
        sf.form.output_sent(&mut out);
    }
    if outcome == FormOutcome::Continue {
        editor.form = Some(sf);
        s.output.push_raw(out);
        return;
    }
    // goback: the cursor below the form, the colours, then the when-done routine
    if sf.is_full_screen() {
        let row = (u16::from(sf.form.maxy()) + 1).min(ANSILN);
        out.extend(format!("\x1B[{row};1f").into_bytes());
    }
    out.extend_from_slice(b"\x1B[0;1;32m");
    sf.read_answers(editor);
    s.output.push_raw(out);
    world
        .outbox
        .push(EngineOutput::SetCharMode { id, on: false });
    newchar::editor_done(world, id, outcome == FormOutcome::Save);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<u16> {
        let mut ain = Ain::default();
        bytes
            .iter()
            .map(|&b| ain.chr(b))
            .filter(|&c| c != 0)
            .collect()
    }

    // the host's key translator: the cursor keys, Home and End, F1-F8; other sequences end without a key
    #[test]
    fn ansi_keys() {
        assert_eq!(
            keys(b"\x1b[A\x1b[B\x1b[C\x1b[D"),
            [CRSRUP, CRSRDN, CRSRRT, CRSRLF]
        );
        assert_eq!(keys(b"\x1b[H\x1b[1;1f\x1b[K"), [HOME, HOME, END]);
        assert_eq!(keys(b"\x1bOP\x1bOr"), [F1, F8]);
        assert_eq!(keys(b"\x1b[2~x"), [u16::from(b'x')]);
        // ESC and any other byte is that byte
        assert_eq!(keys(b"\x1b\x1b\x1bx"), [0x1B, u16::from(b'x')]);
    }

    #[test]
    fn getasc_makes_cr_lf() {
        assert_eq!(getasc(b"a\rb\r"), b"a\r\nb\r\n");
    }

    #[test]
    fn fits_80x23_ansi() {
        let caps = |columns, rows, ansi| Caps {
            columns,
            rows,
            ansi,
            ..Caps::default()
        };
        assert!(fits(&caps(80, 23, true)));
        assert!(!fits(&caps(79, 25, true)));
        assert!(!fits(&caps(80, 22, true)));
        assert!(!fits(&caps(80, 25, false)));
    }
}
