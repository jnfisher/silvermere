//! The line-mode character editor: the form's fields prompted one per line, with the
//! original's field checks on every answer.
//!
//! The original shows the `EDITCHA1` full-screen form, or `EDTCHAN2` line by line on a
//! small or non-ANSI terminal. The host's full-screen library is not carried, so this
//! struct is the session: the race limits, the values as answered so far, the cursor, and
//! the unspent CP (`totalcp - used`). The full-screen form (T-063,
//! [`crate::commands::fullscreen_editor`]) is a view over the same struct: it keeps its
//! own state in [`StatEditor::fullscreen`] and checks answers with
//! [`StatEditor::verify_field`].
//!
//! The per-field prompt wording is this crate's own; it lives in [`PROMPTS`] and
//! [`StatEditor::prompt`] only, so it can change in one place. The validation texts are
//! the original's, and the semantics are the form's: Enter keeps the field's current
//! value, a stat answer is a number, the looks are picked from the three name tables, and
//! the last field is `SAVE`, `EDIT` (back to the first field) or `QUIT`.
//!
//! Two callers open it, as in the original: a new character ([`EditorDone::Creation`],
//! T-022) and `TRAIN STATS` or a stat reset ([`EditorDone::TrainStats`], T-047). The
//! second has no first-name field: the original shows it display-only.

use silvermere_rules::progression::cp_cost;
use silvermere_text::Msg;
use silvermere_types::names::{EYE_COLOURS, HAIR_COLOURS, HAIR_LENGTHS};
use silvermere_types::records::{Character, Race};

use crate::output::OutputBatch;
use crate::state::newchar::{capitalise, leading_int};

/// The six stats in the form's field order: the `TOT_*` fields 6, 9, 12, 15, 18 and 21.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    Strength,
    Intellect,
    Willpower,
    Agility,
    Health,
    Charm,
}

impl Stat {
    /// All six, in field order.
    pub const ALL: [Stat; 6] = [
        Stat::Strength,
        Stat::Intellect,
        Stat::Willpower,
        Stat::Agility,
        Stat::Health,
        Stat::Charm,
    ];

    /// The index into the per-stat arrays.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The name the field check prints (`Strength`, `Intellect`, `Willpower`, `Agility`,
    /// `Health`, `Charm`).
    pub const fn name(self) -> &'static str {
        match self {
            Stat::Strength => "Strength",
            Stat::Intellect => "Intellect",
            Stat::Willpower => "Willpower",
            Stat::Agility => "Agility",
            Stat::Health => "Health",
            Stat::Charm => "Charm",
        }
    }
}

/// The fields the cursor visits, in the form's order; the race, class and the
/// `BASE_*`/`MAX_*` fields were display-only and are shown in the stat prompts instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `LOCALNAME`: the given name (field 0).
    GivenName,
    /// `LASTNAME`: the family name (field 1).
    FamilyName,
    /// `TOT_*`: one stat.
    Stat(Stat),
    /// `HAIR_LEN`.
    HairLength,
    /// `HAIR_COL`.
    HairColour,
    /// `EYE_COL`.
    EyeColour,
    /// `SAVE(ALT=SAVE ALT=EDIT ALT=QUIT)`.
    Exit,
}

impl Field {
    /// The field after this one.
    pub const fn next(self) -> Field {
        match self {
            Field::GivenName => Field::FamilyName,
            Field::FamilyName => Field::Stat(Stat::Strength),
            Field::Stat(Stat::Strength) => Field::Stat(Stat::Intellect),
            Field::Stat(Stat::Intellect) => Field::Stat(Stat::Willpower),
            Field::Stat(Stat::Willpower) => Field::Stat(Stat::Agility),
            Field::Stat(Stat::Agility) => Field::Stat(Stat::Health),
            Field::Stat(Stat::Health) => Field::Stat(Stat::Charm),
            Field::Stat(Stat::Charm) => Field::HairLength,
            Field::HairLength => Field::HairColour,
            Field::HairColour => Field::EyeColour,
            Field::EyeColour => Field::Exit,
            Field::Exit => Field::Exit,
        }
    }

    /// The field before this one.
    pub const fn prev(self) -> Field {
        match self {
            Field::GivenName | Field::FamilyName => Field::GivenName,
            Field::Stat(Stat::Strength) => Field::FamilyName,
            Field::Stat(Stat::Intellect) => Field::Stat(Stat::Strength),
            Field::Stat(Stat::Willpower) => Field::Stat(Stat::Intellect),
            Field::Stat(Stat::Agility) => Field::Stat(Stat::Willpower),
            Field::Stat(Stat::Health) => Field::Stat(Stat::Agility),
            Field::Stat(Stat::Charm) => Field::Stat(Stat::Health),
            Field::HairLength => Field::Stat(Stat::Charm),
            Field::HairColour => Field::HairLength,
            Field::EyeColour => Field::HairColour,
            Field::Exit => Field::EyeColour,
        }
    }

    /// The field's `SF_*` number.
    pub const fn sf(self) -> usize {
        match self {
            Field::GivenName => 0,
            Field::FamilyName => 1,
            Field::Stat(stat) => 6 + 3 * stat.index(),
            Field::HairLength => 22,
            Field::HairColour => 23,
            Field::EyeColour => 24,
            Field::Exit => 25,
        }
    }
}

/// Who opened the editor, and so what happens when it is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorDone {
    /// A new character; the names are checked and the name
    /// validation follows.
    Creation,
    /// `TRAIN STATS` or a character whose stats were reset; the character re-enters the realm.
    TrainStats,
}

/// How the editor was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// `SAVE`.
    Save,
    /// `QUIT`.
    Quit,
}

/// The prompt wording, the one place it is written. `{cur}` is the field's current
/// value, `{min}`/`{max}` the race limits of a stat, `{choices}` the slash-separated
/// words of a looks table.
pub const PROMPTS: Prompts = Prompts {
    given_name: "Given name [{cur}]: ",
    family_name: "Family name [{cur}]: ",
    stat: "{stat} ({min}-{max}) [{cur}]: ",
    hair_length: "Hair length ({choices}) [{cur}]: ",
    hair_colour: "Hair colour ({choices}) [{cur}]: ",
    eye_colour: "Eye colour ({choices}) [{cur}]: ",
    exit: "SAVE or QUIT? [{cur}]: ",
};

/// The templates of [`PROMPTS`].
#[derive(Debug, Clone, Copy)]
pub struct Prompts {
    pub given_name: &'static str,
    pub family_name: &'static str,
    pub stat: &'static str,
    pub hair_length: &'static str,
    pub hair_colour: &'static str,
    pub eye_colour: &'static str,
    pub exit: &'static str,
}

/// The editor session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatEditor {
    /// The race minimum of each stat (`minstr` ..), the base of the CP ladder.
    pub min: [u16; 6],
    /// The race maximum of each stat (`maxstr` ..).
    pub max: [u16; 6],
    /// The character's current stats when the editor opened (`str` ..): a stat may not go
    /// below them. For a new character these are the minimums.
    pub floor: [u16; 6],
    /// The totals as answered so far (`TOT_*`), in [`Stat`] order.
    pub values: [u16; 6],
    /// `totalcp`: the CP the character has to spend in all.
    pub totalcp: u16,
    /// `LOCALNAME`.
    pub given_name: String,
    /// `LASTNAME`.
    pub family_name: String,
    /// `HAIR_LEN`, an index into [`HAIR_LENGTHS`].
    pub hairlen: u8,
    /// `HAIR_COL`, an index into [`HAIR_COLOURS`].
    pub haircol: u8,
    /// `EYE_COL`, an index into [`EYE_COLOURS`].
    pub eyecol: u8,
    /// The field being answered.
    pub field: Field,
    /// `blanklast` (`BLANK_LASTNAME_ALLOWED`): an empty family name is allowed.
    pub blanklast: bool,
    /// The first name is offered (`SF_FIRSTNAME` not display-only).
    pub show_first_name: bool,
    /// What leaving the editor does.
    pub on_done: EditorDone,
    /// The full-screen form's own state when the editor runs full screen (T-063);
    /// `None` in line mode.
    pub fullscreen: Option<Box<crate::commands::fullscreen_editor::FullScreen>>,
}

impl StatEditor {
    /// The form filled as the original fills it on opening:
    /// the names, the race limits, the current stats and the looks.
    pub fn open(plr: &Character, rac: &Race, blanklast: bool) -> Self {
        let u = |n: i16| n.max(0) as u16;
        Self {
            min: [
                u(rac.minstr),
                u(rac.minint),
                u(rac.minwil),
                u(rac.minagi),
                u(rac.minhea),
                u(rac.mincha),
            ],
            max: [
                u(rac.maxstr),
                u(rac.maxint),
                u(rac.maxwil),
                u(rac.maxagi),
                u(rac.maxhea),
                u(rac.maxcha),
            ],
            floor: [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
            values: [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
            totalcp: plr.totalcp,
            given_name: plr.firstname.clone(),
            family_name: plr.lastname.clone(),
            hairlen: plr.hairlen.min(5),
            haircol: plr.haircol.min(9),
            eyecol: plr.eyecol.min(16),
            field: Field::GivenName,
            blanklast,
            show_first_name: true,
            on_done: EditorDone::Creation,
            fullscreen: None,
        }
    }

    /// An existing character: the form of [`StatEditor::open`]
    /// with the first name locked; the cursor starts at the family name.
    pub fn open_existing(plr: &Character, rac: &Race, blanklast: bool) -> Self {
        let mut e = Self::open(plr, rac, blanklast);
        e.show_first_name = false;
        e.on_done = EditorDone::TrainStats;
        e.field = e.first_field();
        e
    }

    /// The first field the cursor visits.
    pub fn first_field(&self) -> Field {
        if self.show_first_name {
            Field::GivenName
        } else {
            Field::FamilyName
        }
    }

    /// The CP the six totals cost above the race minimums (as the field check and the
    /// save count it).
    pub fn used(&self) -> u32 {
        (0..6).map(|i| cp_cost(self.min[i], self.values[i])).sum()
    }

    /// `totalcp - used`: what the CP display shows.
    pub fn unspent(&self) -> i32 {
        i32::from(self.totalcp) - i32::try_from(self.used()).unwrap_or(i32::MAX)
    }

    /// The prompt for the current field, as bytes for the wire (no line end follows).
    pub fn prompt(&self) -> Vec<u8> {
        let text = match self.field {
            Field::GivenName => PROMPTS.given_name.replace("{cur}", &self.given_name),
            Field::FamilyName => PROMPTS.family_name.replace("{cur}", &self.family_name),
            Field::Stat(stat) => {
                let i = stat.index();
                PROMPTS
                    .stat
                    .replace("{stat}", stat.name())
                    .replace("{min}", &self.min[i].to_string())
                    .replace("{max}", &self.max[i].to_string())
                    .replace("{cur}", &self.values[i].to_string())
            }
            Field::HairLength => PROMPTS
                .hair_length
                .replace("{choices}", &HAIR_LENGTHS.join("/"))
                .replace("{cur}", HAIR_LENGTHS[usize::from(self.hairlen)]),
            Field::HairColour => PROMPTS
                .hair_colour
                .replace("{choices}", &HAIR_COLOURS.join("/"))
                .replace("{cur}", HAIR_COLOURS[usize::from(self.haircol)]),
            Field::EyeColour => PROMPTS
                .eye_colour
                .replace("{choices}", &EYE_COLOURS.join("/"))
                .replace("{cur}", EYE_COLOURS[usize::from(self.eyecol)]),
            Field::Exit => PROMPTS.exit.replace("{cur}", "SAVE"),
        };
        silvermere_text::cp437::encode(&text)
    }

    /// One answer for the current field. Appends the validation message (if refused) or
    /// the CP line (after an accepted stat or family name) and the next prompt to `out`;
    /// returns how the editor was left when the answer was SAVE or QUIT.
    pub fn on_line(&mut self, line: &str, out: &mut OutputBatch) -> Option<Exit> {
        let answer = line.trim();
        let outcome = match self.field {
            // Enter keeps the field's current value
            Field::GivenName | Field::FamilyName | Field::Stat(_) => {
                let answer = if answer.is_empty() {
                    self.current(self.field)
                } else {
                    answer.to_string()
                };
                self.verify_field(self.field, &answer)
            }
            Field::HairLength => self.choice_field(answer, HAIR_LENGTHS, |e, i| e.hairlen = i),
            Field::HairColour => self.choice_field(answer, HAIR_COLOURS, |e, i| e.haircol = i),
            Field::EyeColour => self.choice_field(answer, EYE_COLOURS, |e, i| e.eyecol = i),
            Field::Exit => {
                let answer = if answer.is_empty() { "SAVE" } else { answer };
                match answer.as_bytes()[0].to_ascii_uppercase() {
                    b'S' => return Some(Exit::Save),
                    b'Q' => return Some(Exit::Quit),
                    b'E' => {
                        // ALT=EDIT: back to the first field the cursor visits
                        self.field = self.first_field();
                        Verdict::Again
                    }
                    _ => Verdict::Again,
                }
            }
        };
        match outcome {
            Verdict::Reject(msg) => {
                // the refusal, shown by the kernel on its error line; here a line of its own
                out.push(msg);
                out.push_raw(b"\r\n".to_vec());
            }
            Verdict::ShowCp => {
                // the CP display
                out.push(Msg::CpToDistribute { cp: self.unspent() });
                self.field = self.field.next();
            }
            Verdict::Accept => {
                // the kernel's own check passed, on to the next field
                self.field = self.field.next();
            }
            // refused by the kernel's own check, or the cursor was moved: prompt again
            Verdict::Again => {}
        }
        out.push_raw(self.prompt());
        None
    }

    /// The text a name or stat field holds now: what the form shows, and what Enter
    /// on an untouched field answers. Empty for the looks and the exit field.
    pub fn current(&self, field: Field) -> String {
        match field {
            Field::GivenName => self.given_name.clone(),
            Field::FamilyName => self.family_name.clone(),
            Field::Stat(stat) => self.values[stat.index()].to_string(),
            _ => String::new(),
        }
    }

    /// The field check on one answer, shared by the line-mode and the full-screen editor
    /// (T-063): the given name (field 0), the family name (field 1) and the six totals. An
    /// accepted answer is stored; the cursor is not moved. The looks and the exit field
    /// are checked by the kernel and always pass here.
    pub fn verify_field(&mut self, field: Field, answer: &str) -> Verdict {
        match field {
            Field::GivenName => self.given_name_field(answer),
            Field::FamilyName => self.family_name_field(answer),
            Field::Stat(stat) => self.stat_field(stat, answer),
            Field::HairLength | Field::HairColour | Field::EyeColour | Field::Exit => {
                Verdict::Accept
            }
        }
    }

    /// Field 0: the given name.
    fn given_name_field(&mut self, answer: &str) -> Verdict {
        if answer.is_empty() {
            return Verdict::Reject(Msg::NameInvalidField);
        }
        if answer.chars().count() > 10 {
            return Verdict::Reject(Msg::GivenNameTooLong);
        }
        // strip blanks, capitalise, letters only
        let trimmed = answer.trim();
        if !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
            return Verdict::Reject(Msg::NameNotAlpha);
        }
        if trimmed.is_empty() {
            return Verdict::Reject(Msg::NameInvalidField);
        }
        self.given_name = capitalise(trimmed);
        Verdict::Accept
    }

    /// Field 1: the family name. An empty one is allowed only with
    /// `BLANK_LASTNAME_ALLOWED`; it is checked again on SAVE.
    fn family_name_field(&mut self, answer: &str) -> Verdict {
        if answer.is_empty() {
            if self.blanklast {
                self.family_name.clear();
                return Verdict::ShowCp;
            }
            return Verdict::Reject(Msg::LastNameRequired);
        }
        let trimmed = answer.trim();
        if trimmed.chars().count() > 18 {
            return Verdict::Reject(Msg::FamilyNameTooLong);
        }
        if !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
            return Verdict::Reject(Msg::NameNotAlpha);
        }
        self.family_name = capitalise(trimmed);
        Verdict::ShowCp
    }

    /// Fields 6, 9, 12, 15, 18, 21: a stat total. The
    /// new value replaces the old one's cost in `used`; below the current value, above
    /// the race maximum, or over the CP in hand is refused.
    fn stat_field(&mut self, stat: Stat, answer: &str) -> Verdict {
        let i = stat.index();
        let value = leading_int(answer);
        let value = u16::try_from(value.clamp(0, i64::from(u16::MAX))).unwrap_or(0);
        let mut used = self.used();
        used -= cp_cost(self.min[i], self.values[i]);
        used += cp_cost(self.min[i], value);
        if self.floor[i] > value {
            return Verdict::Reject(Msg::StatTooLow {
                stat: stat.name().to_string(),
                n: self.floor[i],
            });
        }
        if self.max[i] < value {
            return Verdict::Reject(Msg::StatTooHigh {
                stat: stat.name().to_string(),
                n: self.max[i],
            });
        }
        if u32::from(self.totalcp) < used {
            return Verdict::Reject(Msg::StatTooMuch {
                stat: stat.name().to_string(),
            });
        }
        self.values[i] = value;
        Verdict::ShowCp
    }

    /// A `MULTICHOICE` field: the answer names an entry of `table` (case does not
    /// matter) or gives its 1-based number; Enter keeps the current one.
    fn choice_field(
        &mut self,
        answer: &str,
        table: &[&str],
        set: impl FnOnce(&mut Self, u8),
    ) -> Verdict {
        if answer.is_empty() {
            return Verdict::Accept;
        }
        let by_name = table
            .iter()
            .position(|name| name.eq_ignore_ascii_case(answer));
        let by_number = answer
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=table.len()).contains(n))
            .map(|n| n - 1);
        match by_name.or(by_number) {
            Some(i) => {
                set(self, i as u8);
                Verdict::Accept
            }
            None => Verdict::Again,
        }
    }
}

/// The CP to raise a stat from `from` to `to` over the race minimum `min`: the difference
/// of two CP-ladder totals, which is how the field check replaces a field's cost. Zero
/// when `to <= from`.
pub fn raise_cost(min: u16, from: u16, to: u16) -> u32 {
    cp_cost(min, to).saturating_sub(cp_cost(min, from))
}

/// The field check's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Refused, with the message shown.
    Reject(Msg),
    /// Accepted, the CP display is redrawn.
    ShowCp,
    /// Accepted.
    Accept,
    /// The kernel's own check refused the answer (not a table word), or EDIT moved the
    /// cursor: prompt the current field again.
    Again,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_session::clear_player;
    use crate::world::blank::clear_race;
    use silvermere_types::ids::RaceNo;

    /// Kang (WCCRACE 11), the race of the fixed test sheet.
    fn kang() -> Race {
        let mut r = clear_race(RaceNo(11));
        r.minint = 30;
        r.minwil = 45;
        r.minstr = 55;
        r.minhea = 50;
        r.minagi = 30;
        r.mincha = 30;
        r.maxint = 90;
        r.maxwil = 110;
        r.maxstr = 120;
        r.maxhea = 110;
        r.maxagi = 90;
        r.maxcha = 85;
        r.basecp = 100;
        r
    }

    fn editor() -> StatEditor {
        let mut plr = clear_player();
        plr.firstname = "KASIMIR".into();
        plr.str = 55;
        plr.intel = 30;
        plr.wil = 45;
        plr.agi = 30;
        plr.hea = 50;
        plr.cha = 30;
        plr.totalcp = 100;
        StatEditor::open(&plr, &kang(), false)
    }

    fn text(out: &OutputBatch) -> String {
        let bytes = out.render(Default::default(), b"");
        silvermere_text::cp437::decode(&silvermere_text::strip::strip_ansi(&bytes))
    }

    // the CP ladder over the six totals
    #[test]
    fn used_sums_the_six_costs() {
        let mut e = editor();
        assert_eq!(e.used(), 0);
        e.values = [79, 30, 45, 42, 74, 30];
        assert_eq!(e.used(), 42 + 14 + 42);
        assert_eq!(e.unspent(), 2);
    }

    // a stat total (field 6)
    #[test]
    fn stat_refusals() {
        let mut e = editor();
        e.field = Field::Stat(Stat::Strength);
        let mut out = OutputBatch::default();
        assert_eq!(e.on_line("54", &mut out), None);
        assert!(text(&out).starts_with("Strength may not be lower than 55.\r\n"));
        out.clear();
        e.on_line("121", &mut out);
        assert!(text(&out).starts_with("Strength may not be higher than 120.\r\n"));
        out.clear();
        e.on_line("120", &mut out);
        assert!(text(&out).starts_with("You may not assign that much to Strength\r\n"));
        assert_eq!(e.field, Field::Stat(Stat::Strength));
        out.clear();
        e.on_line("79", &mut out);
        assert!(text(&out).starts_with("\r\nYou have 58 CP to distribute.\r\n"));
        assert_eq!(e.field, Field::Stat(Stat::Intellect));
    }

    // the family name (field 1): a blank family name needs BLANK_LASTNAME_ALLOWED
    #[test]
    fn blank_family_name_by_option() {
        let mut e = editor();
        e.field = Field::FamilyName;
        let mut out = OutputBatch::default();
        e.on_line("", &mut out);
        assert!(text(&out).starts_with("You must enter a last name!\r\n"));
        assert_eq!(e.field, Field::FamilyName);
        e.blanklast = true;
        out.clear();
        e.on_line("", &mut out);
        assert!(text(&out).starts_with("\r\nYou have 100 CP to distribute.\r\n"));
        assert_eq!(e.field, Field::Stat(Stat::Strength));
    }

    #[test]
    fn enter_keeps_the_current_value() {
        let mut e = editor();
        let mut out = OutputBatch::default();
        e.on_line("", &mut out);
        assert_eq!(e.given_name, "KASIMIR");
        assert_eq!(e.field, Field::FamilyName);
    }

    // the given name (field 0)
    #[test]
    fn given_name_rules() {
        let mut e = editor();
        let mut out = OutputBatch::default();
        e.on_line("Bartholomew", &mut out);
        assert!(text(&out).starts_with("Your given name can only be 10 characters long!"));
        out.clear();
        e.on_line("k4simir", &mut out);
        assert!(text(&out).starts_with("Only ALPHA characters are allowed in a name!"));
        out.clear();
        e.on_line("kasimir", &mut out);
        assert_eq!(e.given_name, "Kasimir");
        assert_eq!(e.field, Field::FamilyName);
    }

    #[test]
    fn looks_by_name_or_number_and_the_exit_field() {
        let mut e = editor();
        e.field = Field::HairLength;
        let mut out = OutputBatch::default();
        e.on_line("LONG", &mut out);
        assert_eq!(e.hairlen, 3);
        e.on_line("10", &mut out);
        assert_eq!(e.haircol, 9);
        e.on_line("violet", &mut out);
        assert_eq!(e.eyecol, 14);
        assert_eq!(e.field, Field::Exit);
        assert_eq!(e.on_line("edit", &mut out), None);
        assert_eq!(e.field, Field::GivenName);
        e.field = Field::Exit;
        assert_eq!(e.on_line("q", &mut out), Some(Exit::Quit));
        assert_eq!(e.on_line("", &mut out), Some(Exit::Save));
    }

    // an existing character has the first name locked; EDIT goes back
    // to the first field the cursor visits
    #[test]
    fn existing_character_has_no_first_name_field() {
        let mut plr = clear_player();
        plr.firstname = "Kasimir".into();
        plr.lastname = "Stonefist".into();
        plr.str = 60;
        plr.intel = 30;
        plr.wil = 45;
        plr.agi = 30;
        plr.hea = 50;
        plr.cha = 30;
        plr.totalcp = 20;
        let mut e = StatEditor::open_existing(&plr, &kang(), false);
        assert_eq!(e.on_done, EditorDone::TrainStats);
        assert!(!e.show_first_name);
        assert_eq!(e.prompt(), b"Family name [Stonefist]: ");
        assert_eq!(e.floor[0], 60, "never below the current value");
        assert_eq!(e.unspent(), 15);
        e.field = Field::Exit;
        let mut out = OutputBatch::default();
        assert_eq!(e.on_line("edit", &mut out), None);
        assert_eq!(e.field, Field::FamilyName);
        assert_eq!(e.given_name, "Kasimir");
    }

    // the CP ladder: the band totals 10, 30, 60, 100, 150
    #[test]
    fn band_totals() {
        let totals: Vec<u32> = (1..=5).map(|b| raise_cost(55, 55, 55 + b * 10)).collect();
        assert_eq!(totals, [10, 30, 60, 100, 150]);
        // across the first boundary: 5 at 1 CP, 5 at 2 CP
        assert_eq!(raise_cost(55, 60, 70), 15);
        assert_eq!(raise_cost(55, 70, 60), 0);
    }

    proptest::proptest! {
        // raise_cost: zero when `to` equals `from`, monotone in `to`, and the
        // single-point steps add up to the one-shot cost
        #[test]
        fn raise_cost_properties(min in 0u16..120, a in 0u16..120, b in 0u16..120) {
            let (from, to) = (a.max(min), a.max(min).saturating_add(b));
            proptest::prop_assert_eq!(raise_cost(min, from, from), 0);
            proptest::prop_assert!(raise_cost(min, from, to) <= raise_cost(min, from, to + 1));
            let steps: u32 = (from..to).map(|v| raise_cost(min, v, v + 1)).sum();
            proptest::prop_assert_eq!(steps, raise_cost(min, from, to));
        }
    }

    #[test]
    fn prompts_come_from_one_table() {
        let e = editor();
        assert_eq!(e.prompt(), b"Given name [KASIMIR]: ");
        let mut s = e.clone();
        s.field = Field::Stat(Stat::Health);
        assert_eq!(s.prompt(), b"Health (50-110) [50]: ");
        s.field = Field::Exit;
        assert_eq!(s.prompt(), b"SAVE or QUIT? [SAVE]: ");
    }
}
