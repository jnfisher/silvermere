//! The character editor's session: the race limits, the character's stats when it opened,
//! the answers the editor's end reads, and the game's field checks.
//!
//! The editor itself is the host's form library ([`crate::state::newchar::form`]): the
//! game's `EDITCHA1` form full screen, or `EDTCHAN2` line by line. Its verify routine runs
//! [`StatEditor::verify_field`] on the names and the six totals; the editor's end
//! ([`crate::state::newchar::editor_done`]) reads the answers from here.
//!
//! Two callers open it, as in the game: a new character ([`EditorDone::Creation`],
//! T-022) and `TRAIN STATS` or a stat reset ([`EditorDone::TrainStats`], T-047). The
//! second has no first-name field: the game shows it display-only.

use silvermere_rules::progression::cp_cost;
use silvermere_text::Msg;
use silvermere_types::records::{Character, Race};

use crate::state::newchar::form::StatForm;
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

/// The fields the game's check looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `LOCALNAME`: the given name (field 0).
    GivenName,
    /// `LASTNAME`: the family name (field 1).
    FamilyName,
    /// `TOT_*`: one stat.
    Stat(Stat),
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

/// The editor session.
#[derive(Debug, Clone)]
pub struct StatEditor {
    /// The race minimum of each stat (`minstr` ..), the base of the CP ladder.
    pub min: [u16; 6],
    /// The race maximum of each stat (`maxstr` ..).
    pub max: [u16; 6],
    /// The character's current stats when the editor opened (`str` ..): a stat may not go
    /// below them. For a new character these are the minimums.
    pub floor: [u16; 6],
    /// The six totals (`TOT_*`), in [`Stat`] order: as the form holds them when a field is
    /// checked, and as saved at the end.
    pub values: [u16; 6],
    /// `totalcp`: the CP the character has to spend in all.
    pub totalcp: u16,
    /// `LOCALNAME`.
    pub given_name: String,
    /// `LASTNAME`.
    pub family_name: String,
    /// `HAIR_LEN`, an index into the hair lengths.
    pub hairlen: u8,
    /// `HAIR_COL`, an index into the hair colours.
    pub haircol: u8,
    /// `EYE_COL`, an index into the eye colours.
    pub eyecol: u8,
    /// `blanklast` (`BLANK_LASTNAME_ALLOWED`): an empty family name is allowed.
    pub blanklast: bool,
    /// The first name is offered (`SF_FIRSTNAME` not display-only).
    pub show_first_name: bool,
    /// What leaving the editor does.
    pub on_done: EditorDone,
    /// The form while it is open.
    pub form: Option<Box<StatForm>>,
    /// The record as stored, when entering opened the editor: leaving it unsaved leaves the
    /// stored record, which the next entry loads again.
    pub stored: Option<Box<Character>>,
}

impl StatEditor {
    /// The session as the game fills the form on opening:
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
            blanklast,
            show_first_name: true,
            on_done: EditorDone::Creation,
            form: None,
            stored: None,
        }
    }

    /// An existing character: the session of [`StatEditor::open`] with the first name
    /// locked.
    pub fn open_existing(plr: &Character, rac: &Race, blanklast: bool) -> Self {
        let mut e = Self::open(plr, rac, blanklast);
        e.show_first_name = false;
        e.on_done = EditorDone::TrainStats;
        e
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

    /// The game's check of one answer: the given name (field 0), the family name (field 1)
    /// and the six totals, against [`StatEditor::values`] as the form holds them. An
    /// accepted answer is stored here: a name as the game rewrites it (blanks taken out,
    /// the first letter capitalised), a total in [`StatEditor::values`].
    pub fn verify_field(&mut self, field: Field, answer: &str) -> Verdict {
        match field {
            Field::GivenName => self.given_name_field(answer),
            Field::FamilyName => self.family_name_field(answer),
            Field::Stat(stat) => self.stat_field(stat, answer),
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
        // white space taken out, letters only, capitalised
        let trimmed: String = answer
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let trimmed = trimmed.as_str();
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
        let trimmed: String = answer
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let trimmed = trimmed.as_str();
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
        // `atol` into the game's unsigned (a minus cannot reach it: MIN=10 keeps it out)
        let value = leading_int(answer.trim_start()) as u16;
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

    fn strength(stat: &str, n: u16) -> Msg {
        match stat {
            "low" => Msg::StatTooLow {
                stat: "Strength".into(),
                n,
            },
            _ => Msg::StatTooHigh {
                stat: "Strength".into(),
                n,
            },
        }
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
        let s = Field::Stat(Stat::Strength);
        assert_eq!(
            e.verify_field(s, "54"),
            Verdict::Reject(strength("low", 55))
        );
        assert_eq!(
            e.verify_field(s, "121"),
            Verdict::Reject(strength("high", 120))
        );
        // atol into an unsigned: a negative number is above the maximum
        assert_eq!(
            e.verify_field(s, "-3"),
            Verdict::Reject(strength("high", 120))
        );
        assert_eq!(
            e.verify_field(s, "120"),
            Verdict::Reject(Msg::StatTooMuch {
                stat: "Strength".into()
            })
        );
        assert_eq!(e.values[0], 55);
        assert_eq!(e.verify_field(s, "79"), Verdict::ShowCp);
        assert_eq!(e.values[0], 79);
        assert_eq!(e.unspent(), 58);
    }

    // the family name (field 1): a blank family name needs BLANK_LASTNAME_ALLOWED
    #[test]
    fn blank_family_name_by_option() {
        let mut e = editor();
        assert_eq!(
            e.verify_field(Field::FamilyName, ""),
            Verdict::Reject(Msg::LastNameRequired)
        );
        e.blanklast = true;
        assert_eq!(e.verify_field(Field::FamilyName, ""), Verdict::ShowCp);
        assert_eq!(
            e.verify_field(Field::FamilyName, "van dyke"),
            Verdict::ShowCp
        );
        assert_eq!(e.family_name, "Vandyke");
    }

    // the given name (field 0)
    #[test]
    fn given_name_rules() {
        let mut e = editor();
        let g = Field::GivenName;
        assert_eq!(
            e.verify_field(g, "Bartholomew"),
            Verdict::Reject(Msg::GivenNameTooLong)
        );
        assert_eq!(
            e.verify_field(g, "k4simir"),
            Verdict::Reject(Msg::NameNotAlpha)
        );
        assert_eq!(
            e.verify_field(g, ""),
            Verdict::Reject(Msg::NameInvalidField)
        );
        assert_eq!(e.verify_field(g, "kasimir"), Verdict::Accept);
        assert_eq!(e.given_name, "Kasimir");
    }

    // an existing character has the first name locked
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
        let e = StatEditor::open_existing(&plr, &kang(), false);
        assert_eq!(e.on_done, EditorDone::TrainStats);
        assert!(!e.show_first_name);
        assert_eq!(e.floor[0], 60, "never below the current value");
        assert_eq!(e.unspent(), 15);
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
}
