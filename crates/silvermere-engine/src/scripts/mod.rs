//! The text block script engine: rooms, monsters, spells, deaths and the
//! sysop point at a WCCTEXT block whose lines are little scripts of `:`-separated
//! clauses (`checkitem 123 45:takeitem 123:teleport 100 1:text 789`).
//!
//! - this module: the string helpers that read words and numbers as the content expects
//!   ([`skip_word`], [`skip_blanks`], [`leading_int32`], [`leading_int`]), how a record splits
//!   into lines, the wildcard patterns ([`wildcard_match`]) and [`Clause`], the 42 keywords of
//!   `perform_matched_action` parsed into a typed clause.
//! - [`actions`]: `perform_matched_action` and the keyword handlers, `add_quest_exp`,
//!   `erase_item_from_room`, `run_room_exit_action` and the Remote Action exits that call
//!   it.
//! - [`special`]: the three block runners, `perform_special_command`,
//!   `perform_random_text_block` and `run_text_block`, and the
//!   input-stage hook.
//!
//! The blocks are read from `World::text_blocks` by `(part, number)`, one record (part) at
//! a time (a line never spans two records), at most 50 records a
//! walk. Parsing a record is a scan over at most 2000 bytes, so it is done per run rather
//! than cached: a run is a hash lookup and a scan.

pub mod actions;
pub mod special;

pub use actions::{
    LineResult, add_quest_exp, erase_item_from_room, perform_matched_action, run_room_exit_action,
    show_action_message,
};
pub use special::{
    perform_random_text_block, perform_special_command, room_command, run_text_block,
};

use crate::commands::parse::starts_with_ignore_case;

/// At most 20 `|` alternatives are tried.
pub const MAXALTS: usize = 20;
/// The records one block walk reads at most.
pub const MAX_RECORDS: i16 = 50;

/// Past the word (to the first white space or the end).
pub fn skip_word(s: &str) -> &str {
    let i = s
        .bytes()
        .position(|b| b.is_ascii_whitespace())
        .unwrap_or(s.len());
    &s[i..]
}

/// Past the white space.
pub fn skip_blanks(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_ascii_whitespace())
}

/// A number: leading white space, a sign, then digits; anything else ends the number (0
/// when there is none). The 32-bit result wraps.
pub fn leading_int32(s: &str) -> i32 {
    let s = skip_blanks(s).as_bytes();
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i32 = 0;
    for &b in digits.iter().take_while(|b| b.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
    }
    if neg { v.wrapping_neg() } else { v }
}

/// [`leading_int32`] cut to 16 bits.
pub fn leading_int(s: &str) -> i16 {
    leading_int32(s) as i16
}

/// The words of a clause after its keyword: [`skip_word`] then [`skip_blanks`] steps to the
/// next word, and a missing word reads as 0 (sic: a missing word is never refused).
#[derive(Debug, Clone, Copy)]
pub struct Args<'a> {
    rest: &'a str,
}

impl<'a> Args<'a> {
    /// The words after the keyword (`skip_blanks(skip_word(clause))`).
    pub fn of(clause: &'a str) -> Self {
        Self {
            rest: skip_blanks(skip_word(clause)),
        }
    }

    /// The current word and what follows it.
    pub fn rest(&self) -> &'a str {
        self.rest
    }

    /// `leading_int32(arg)` of the current word, then on to the next.
    pub fn long(&mut self) -> i32 {
        let v = leading_int32(self.rest);
        self.skip();
        v
    }

    /// `leading_int(arg)` of the current word, then on to the next.
    pub fn int(&mut self) -> i16 {
        let v = leading_int(self.rest);
        self.skip();
        v
    }

    /// On to the next word.
    pub fn skip(&mut self) {
        self.rest = skip_blanks(skip_word(self.rest));
    }
}

/// One line of a pattern block (a room's command text, a random block): the pattern
/// before the first `:` and the clauses after it, to the line end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternLine {
    /// The `|` list of wildcard patterns, or a random block's percentage.
    pub pattern: String,
    /// The clauses, for `perform_matched_action`.
    pub action: String,
}

/// The lines of one record of a pattern block, as `perform_special_command` and
/// `perform_random_text_block` scan it: the pattern is
/// everything from a line's start to the next `:` anywhere further on (so a line with
/// no `:` joins the next one's pattern), the clauses run to the line end; the scan ends
/// when no `:` follows. (sic, not carried) After a matched line that
/// stopped, the original could go on matching past the end of the record's text; here the
/// record ends there.
pub fn pattern_lines(text: &str) -> Vec<PatternLine> {
    let mut out = Vec::new();
    let mut line = 0usize;
    while let Some(c) = text[line..].find(':') {
        let colon = line + c;
        let start = colon + 1;
        let end = text[start..].find('\n').map_or(text.len(), |e| start + e);
        out.push(PatternLine {
            pattern: text[line..colon].to_owned(),
            action: text[start..end].to_owned(),
        });
        if end >= text.len() {
            break;
        }
        line = end + 1;
    }
    out
}

/// The lines of one record of a block run as a whole, as
/// `run_text_block` scans it: the first line
/// always, then each next line while a `:` follows anywhere from its start.
pub fn whole_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    loop {
        let end = text[start..].find('\n').map_or(text.len(), |e| start + e);
        out.push(&text[start..end]);
        if end >= text.len() {
            break;
        }
        start = end + 1;
        if !text[start..].contains(':') {
            break;
        }
    }
    out
}

/// One pattern, ending at the
/// end or a `|`, against the typed text. Letters compare without case. A `*` turns
/// abbreviation on from there: a typed word may then stop short (a typed space skips
/// the rest of the pattern word). A match needs all the typed text used, and the
/// pattern used too (or only a `*` left), unless abbreviation is on.
fn match_wildcard_pattern(pattern: &[u8], text: &[u8]) -> bool {
    let (mut cp, mut tp) = (0usize, 0usize);
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let mut abbrev = false;
    while at(pattern, cp) != 0 && at(pattern, cp) != b'|' && at(text, tp) != 0 {
        let c = at(pattern, cp);
        let t = at(text, tp);
        if c == b'*' {
            abbrev = true;
        } else if abbrev && t == b' ' {
            while at(pattern, cp) != 0 && at(pattern, cp) != b'|' && at(pattern, cp) != b' ' {
                cp += 1;
            }
            if at(pattern, cp) == b' ' {
                cp += 1;
            }
            tp += 1;
            continue;
        } else if c.eq_ignore_ascii_case(&t) {
            tp += 1;
        } else {
            return false;
        }
        cp += 1;
    }
    let rest = at(pattern, cp);
    at(text, tp) == 0 && (abbrev || rest == 0 || rest == b'|' || rest == b'*')
}

/// Does the typed text match any of up
/// to [`MAXALTS`] `|`-separated patterns?
pub fn wildcard_match(patterns: &str, text: &str) -> bool {
    let p = patterns.as_bytes();
    let mut alts = vec![0usize];
    for (i, &b) in p.iter().enumerate() {
        if b == b'|' && alts.len() < MAXALTS {
            alts.push(i + 1);
        }
    }
    alts.iter()
        .any(|&a| match_wildcard_pattern(&p[a..], text.as_bytes()))
}

/// The 42 keywords of `perform_matched_action`, in matching order: a clause
/// takes the first keyword it starts with (`starts_with_ignore_case`, no case), so the order
/// decides (`nomonsters` before `monsters`, `teleport` takes every `teleport_` form).
pub const KEYWORDS: [(&str, Keyword); 42] = [
    ("price", Keyword::Price),
    ("cast", Keyword::Cast),
    ("giveability", Keyword::GiveAbility),
    ("addability", Keyword::AddAbility),
    ("teleport", Keyword::Teleport),
    ("removeability", Keyword::RemoveAbility),
    ("nomonsters", Keyword::NoMonsters),
    ("monsters", Keyword::Monsters),
    ("needmonster", Keyword::NeedMonster),
    ("summon", Keyword::Summon),
    ("message", Keyword::Message),
    ("takeitem", Keyword::TakeItem),
    ("addexp", Keyword::AddExp),
    ("giveitem", Keyword::GiveItem),
    ("hideitem", Keyword::HideItem),
    ("checkability", Keyword::CheckAbility),
    ("class", Keyword::Class),
    ("race", Keyword::Race),
    ("goodaligned", Keyword::GoodAligned),
    ("evilaligned", Keyword::EvilAligned),
    ("minlevel", Keyword::MinLevel),
    ("maxlevel", Keyword::MaxLevel),
    ("checkitem", Keyword::CheckItem),
    ("failitem", Keyword::FailItem),
    ("text", Keyword::Text),
    ("roomtext", Keyword::RoomText),
    ("roomitem", Keyword::RoomItem),
    ("failroomitem", Keyword::FailRoomItem),
    ("clearitem", Keyword::ClearItem),
    ("addevil", Keyword::AddEvil),
    ("testability", Keyword::TestAbility),
    ("failability", Keyword::FailAbility),
    ("remoteaction", Keyword::RemoteAction),
    ("random", Keyword::Random),
    ("adddelay", Keyword::AddDelay),
    ("testskill", Keyword::TestSkill),
    ("checkskill", Keyword::CheckSkill),
    ("givecoins", Keyword::GiveCoins),
    ("test_tournament", Keyword::TestTournament),
    ("checkspell", Keyword::CheckSpell),
    ("flag", Keyword::Flag),
    ("learnspell", Keyword::LearnSpell),
];

/// A keyword of `perform_matched_action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Keyword {
    Price,
    Cast,
    GiveAbility,
    AddAbility,
    Teleport,
    RemoveAbility,
    NoMonsters,
    Monsters,
    NeedMonster,
    Summon,
    Message,
    TakeItem,
    AddExp,
    GiveItem,
    HideItem,
    CheckAbility,
    Class,
    Race,
    GoodAligned,
    EvilAligned,
    MinLevel,
    MaxLevel,
    CheckItem,
    FailItem,
    Text,
    RoomText,
    RoomItem,
    FailRoomItem,
    ClearItem,
    AddEvil,
    TestAbility,
    FailAbility,
    RemoteAction,
    Random,
    AddDelay,
    TestSkill,
    CheckSkill,
    GiveCoins,
    TestTournament,
    CheckSpell,
    Flag,
    LearnSpell,
}

impl Keyword {
    /// The keyword a clause starts with, if any.
    pub fn of(clause: &str) -> Option<Self> {
        KEYWORDS
            .iter()
            .find(|(k, _)| starts_with_ignore_case(k, clause))
            .map(|&(_, k)| k)
    }
}

/// The named teleports of the `teleport` clause, in matching order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    Silvermere,
    Sewers,
    SlumSewers,
    Slums,
    Darkwood,
    ElderGrove,
    Labyrinth,
    BlackCaverns,
    FungusHazy,
    CrackedLava,
    BlackFortress,
    Obsidian,
}

/// One named teleport: its keyword, map and room ranges. With more than one range `n`
/// is rolled from 1 with the upper bound `roll` and range `n - 1` taken, the first for any
/// roll past the last range (the fallback shares the first range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeleportArea {
    pub area: Area,
    pub keyword: &'static str,
    pub map: u32,
    /// The roll's upper bound, for several ranges.
    pub roll: i32,
    /// The room ranges (`lo..=hi`) a room is rolled from.
    pub ranges: &'static [(i64, i64)],
}

/// The stock world's named teleports. World-specific content: a different world replaces
/// the table ([`special::Hooks::teleport_areas`]).
pub const STOCK_TELEPORTS: [TeleportArea; 12] = [
    // Silvermere: six ranges
    TeleportArea {
        area: Area::Silvermere,
        keyword: "teleport_silvermere",
        map: 1,
        roll: 18,
        ranges: &[
            (3, 10),
            (12, 37),
            (45, 62),
            (66, 108),
            (112, 116),
            (117, 129),
            (130, 141),
            (143, 146),
            (184, 191),
            (207, 230),
            (231, 238),
            (240, 256),
            (301, 304),
            (344, 348),
            (394, 397),
            (437, 445),
            (1013, 1016),
        ],
    },
    TeleportArea {
        area: Area::Sewers,
        keyword: "teleport_sewers",
        map: 1,
        roll: 3,
        ranges: &[(547, 737), (751, 815)],
    },
    TeleportArea {
        area: Area::SlumSewers,
        keyword: "teleport_slumsewers",
        map: 1,
        roll: 0,
        ranges: &[(859, 1010)],
    },
    TeleportArea {
        area: Area::Slums,
        keyword: "teleport_slums",
        map: 1,
        roll: 0,
        ranges: &[(1072, 1231)],
    },
    TeleportArea {
        area: Area::Darkwood,
        keyword: "teleport_darkwood",
        map: 1,
        roll: 3,
        ranges: &[(1396, 1417), (1647, 1750)],
    },
    TeleportArea {
        area: Area::ElderGrove,
        keyword: "teleport_eldergrove",
        map: 7,
        roll: 0,
        ranges: &[(21, 133)],
    },
    TeleportArea {
        area: Area::Labyrinth,
        keyword: "teleport_labyrinth",
        map: 1,
        roll: 0,
        ranges: &[(1828, 2106)],
    },
    TeleportArea {
        area: Area::BlackCaverns,
        keyword: "teleport_blackcaverns",
        map: 1,
        roll: 0,
        ranges: &[(1450, 1635)],
    },
    TeleportArea {
        area: Area::FungusHazy,
        keyword: "teleport_fungushazy",
        map: 7,
        roll: 0,
        ranges: &[(156, 729)],
    },
    TeleportArea {
        area: Area::CrackedLava,
        keyword: "teleport_crackedlava",
        map: 7,
        roll: 0,
        ranges: &[(732, 1006)],
    },
    TeleportArea {
        area: Area::BlackFortress,
        keyword: "teleport_blackfortress",
        map: 7,
        roll: 3,
        ranges: &[(1173, 1217), (1223, 1284)],
    },
    TeleportArea {
        area: Area::Obsidian,
        keyword: "teleport_obsidian",
        map: 7,
        roll: 0,
        ranges: &[(1289, 1358)],
    },
];

/// Where a `teleport` clause goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teleport {
    /// A named area (`teleport_silvermere`, ...), index into the teleport table.
    Named(usize),
    /// `teleport <room> [<map>]`; 0 for a missing word.
    Room { room: i32, map: i32 },
}

/// The stats `checkskill` and `testskill` name, in test order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skill {
    Agility,
    Strength,
    Intellect,
    Wisdom,
    Health,
    Charm,
    Spellcasting,
    Perception,
    Stealth,
    Thievery,
    Traps,
    Picklocks,
    Tracking,
    MagicResistance,
    CurrentHp,
    /// An unknown name: a value of 0 against a range of 100.
    Unknown,
}

/// The skill names, matched by `starts_with_ignore_case` in this order.
pub const SKILLS: [(&str, Skill); 15] = [
    ("agility", Skill::Agility),
    ("strength", Skill::Strength),
    ("intellect", Skill::Intellect),
    ("wisdom", Skill::Wisdom),
    ("health", Skill::Health),
    ("charm", Skill::Charm),
    ("spellcasting", Skill::Spellcasting),
    ("perception", Skill::Perception),
    ("stealth", Skill::Stealth),
    ("thievery", Skill::Thievery),
    ("traps", Skill::Traps),
    ("picklocks", Skill::Picklocks),
    ("tracking", Skill::Tracking),
    ("magicresistance", Skill::MagicResistance),
    ("current_hp", Skill::CurrentHp),
];

impl Skill {
    fn of(word: &str) -> Self {
        SKILLS
            .iter()
            .find(|(n, _)| starts_with_ignore_case(n, word))
            .map_or(Skill::Unknown, |&(_, s)| s)
    }

    /// The roll range (`maxval`).
    pub fn range(self) -> i32 {
        match self {
            Skill::Agility
            | Skill::Strength
            | Skill::Intellect
            | Skill::Wisdom
            | Skill::Health
            | Skill::Charm
            | Skill::MagicResistance => 150,
            Skill::Spellcasting
            | Skill::Stealth
            | Skill::Thievery
            | Skill::Traps
            | Skill::Picklocks
            | Skill::Tracking => 175,
            Skill::Perception => 110,
            Skill::CurrentHp | Skill::Unknown => 100,
        }
    }
}

/// The verb of a `flag` clause. (sic) The verb never takes effect in play, so only the
/// range test runs; the verb is parsed here from the word the author meant, for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagVerb {
    Set,
    Check,
    Test,
    Clear,
    Other,
}

/// A coin of `givecoins`; copper for a missing or unknown letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coin {
    Runic,
    Platinum,
    Gold,
    Silver,
    Copper,
}

/// One clause of an action line, its numbers read as the handler reads them (a missing
/// number is 0). Message numbers are WCCMSG records, blocks WCCTEXT records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clause {
    Price {
        copper: i32,
        msg: i32,
    },
    Cast {
        spell: i32,
    },
    GiveAbility {
        ability: i32,
        value: i32,
    },
    AddAbility {
        ability: i32,
        amount: i32,
    },
    Teleport(Teleport),
    RemoveAbility {
        ability: i32,
    },
    NoMonsters {
        msg: i32,
    },
    Monsters {
        msg: i32,
    },
    NeedMonster {
        monster: i32,
        msg: i32,
    },
    Summon {
        monster: i32,
    },
    Message {
        msg: i32,
    },
    TakeItem {
        item: i32,
        msg: i32,
    },
    AddExp {
        exp: i32,
    },
    GiveItem {
        item: i32,
    },
    HideItem {
        item: i32,
    },
    CheckAbility {
        ability: i16,
        min: i16,
    },
    Class {
        class: i32,
    },
    Race {
        race: i32,
    },
    GoodAligned {
        limit: i32,
        msg: i32,
    },
    EvilAligned {
        limit: i32,
        msg: i32,
    },
    MinLevel {
        level: i32,
        msg: i32,
    },
    MaxLevel {
        level: i32,
        msg: i32,
    },
    CheckItem {
        item: i32,
        msg: i32,
    },
    FailItem {
        item: i32,
        msg: i32,
    },
    Text {
        block: i32,
    },
    RoomText {
        block: i32,
    },
    RoomItem {
        item: i32,
        msg: i32,
    },
    FailRoomItem {
        item: i32,
        msg: i32,
    },
    ClearItem {
        item: i32,
        msg: i32,
    },
    AddEvil {
        points: i16,
    },
    TestAbility {
        ability: i16,
        max: i16,
    },
    FailAbility {
        ability: i16,
        msg: i16,
    },
    RemoteAction {
        room: i32,
        msg: i32,
        action: i32,
        exit: i16,
    },
    Random {
        block: i32,
    },
    AddDelay {
        ticks: i32,
    },
    TestSkill {
        skill: Skill,
        modifier: i16,
        block: i16,
    },
    CheckSkill {
        skill: Skill,
        min: i16,
        msg: i16,
    },
    GiveCoins {
        amount: i32,
        coin: Coin,
    },
    TestTournament,
    CheckSpell {
        spell: i32,
        block: i32,
    },
    Flag {
        flag: i16,
        verb: FlagVerb,
        msg: i32,
    },
    LearnSpell {
        spell: i16,
    },
    /// No keyword: it is skipped and the line goes on.
    Unknown(String),
}

impl Clause {
    /// The clause with its numbers, or [`Clause::Unknown`] (`teleports` is the named
    /// teleport table, tested before the plain form).
    pub fn parse(clause: &str, teleports: &[TeleportArea]) -> Self {
        let Some(kw) = Keyword::of(clause) else {
            return Clause::Unknown(clause.to_owned());
        };
        let mut a = Args::of(clause);
        match kw {
            Keyword::Price => {
                let copper = a.long();
                Clause::Price {
                    copper,
                    msg: a.long(),
                }
            }
            Keyword::Cast => Clause::Cast { spell: a.long() },
            Keyword::GiveAbility => {
                let ability = a.long();
                Clause::GiveAbility {
                    ability,
                    value: a.long(),
                }
            }
            Keyword::AddAbility => {
                let ability = a.long();
                Clause::AddAbility {
                    ability,
                    amount: a.long(),
                }
            }
            Keyword::Teleport => match teleports
                .iter()
                .position(|t| starts_with_ignore_case(t.keyword, clause))
            {
                Some(i) => Clause::Teleport(Teleport::Named(i)),
                None => {
                    let room = a.long();
                    Clause::Teleport(Teleport::Room {
                        room,
                        map: a.long(),
                    })
                }
            },
            Keyword::RemoveAbility => Clause::RemoveAbility { ability: a.long() },
            Keyword::NoMonsters => Clause::NoMonsters { msg: a.long() },
            Keyword::Monsters => Clause::Monsters { msg: a.long() },
            Keyword::NeedMonster => {
                let monster = a.long();
                Clause::NeedMonster {
                    monster,
                    msg: a.long(),
                }
            }
            Keyword::Summon => Clause::Summon { monster: a.long() },
            Keyword::Message => Clause::Message { msg: a.long() },
            Keyword::TakeItem => {
                let item = a.long();
                Clause::TakeItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::AddExp => Clause::AddExp { exp: a.long() },
            Keyword::GiveItem => Clause::GiveItem { item: a.long() },
            Keyword::HideItem => Clause::HideItem { item: a.long() },
            Keyword::CheckAbility => {
                let ability = a.int();
                Clause::CheckAbility {
                    ability,
                    min: a.int(),
                }
            }
            Keyword::Class => Clause::Class { class: a.long() },
            Keyword::Race => Clause::Race { race: a.long() },
            Keyword::GoodAligned => {
                let limit = a.long();
                Clause::GoodAligned {
                    limit,
                    msg: a.long(),
                }
            }
            Keyword::EvilAligned => {
                let limit = a.long();
                Clause::EvilAligned {
                    limit,
                    msg: a.long(),
                }
            }
            Keyword::MinLevel => {
                let level = a.long();
                Clause::MinLevel {
                    level,
                    msg: a.long(),
                }
            }
            Keyword::MaxLevel => {
                let level = a.long();
                Clause::MaxLevel {
                    level,
                    msg: a.long(),
                }
            }
            Keyword::CheckItem => {
                let item = a.long();
                Clause::CheckItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::FailItem => {
                let item = a.long();
                Clause::FailItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::Text => Clause::Text { block: a.long() },
            Keyword::RoomText => Clause::RoomText { block: a.long() },
            Keyword::RoomItem => {
                let item = a.long();
                Clause::RoomItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::FailRoomItem => {
                let item = a.long();
                Clause::FailRoomItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::ClearItem => {
                let item = a.long();
                Clause::ClearItem {
                    item,
                    msg: a.long(),
                }
            }
            Keyword::AddEvil => Clause::AddEvil { points: a.int() },
            Keyword::TestAbility => {
                let ability = a.int();
                Clause::TestAbility {
                    ability,
                    max: a.int(),
                }
            }
            Keyword::FailAbility => {
                let ability = a.int();
                Clause::FailAbility {
                    ability,
                    msg: a.int(),
                }
            }
            Keyword::RemoteAction => {
                let room = a.long();
                let msg = a.long();
                let action = a.long();
                Clause::RemoteAction {
                    room,
                    msg,
                    action,
                    exit: a.int(),
                }
            }
            Keyword::Random => Clause::Random { block: a.long() },
            Keyword::AddDelay => Clause::AddDelay { ticks: a.long() },
            Keyword::TestSkill | Keyword::CheckSkill => {
                let skill = Skill::of(a.rest());
                a.skip();
                let n = a.int();
                let m = a.int();
                if kw == Keyword::TestSkill {
                    Clause::TestSkill {
                        skill,
                        modifier: n,
                        block: m,
                    }
                } else {
                    Clause::CheckSkill {
                        skill,
                        min: n,
                        msg: m,
                    }
                }
            }
            Keyword::GiveCoins => {
                let amount = a.long();
                let coin = match a.rest().bytes().next().map(|b| b.to_ascii_uppercase()) {
                    Some(b'R') => Coin::Runic,
                    Some(b'P') => Coin::Platinum,
                    Some(b'G') => Coin::Gold,
                    Some(b'S') => Coin::Silver,
                    _ => Coin::Copper,
                };
                Clause::GiveCoins { amount, coin }
            }
            Keyword::TestTournament => Clause::TestTournament,
            Keyword::CheckSpell => {
                let spell = a.long();
                Clause::CheckSpell {
                    spell,
                    block: a.long(),
                }
            }
            Keyword::Flag => {
                let flag = a.int();
                let verb = match a.rest() {
                    w if starts_with_ignore_case("set", w) => FlagVerb::Set,
                    w if starts_with_ignore_case("check", w) => FlagVerb::Check,
                    w if starts_with_ignore_case("test", w) => FlagVerb::Test,
                    w if starts_with_ignore_case("clear", w) => FlagVerb::Clear,
                    _ => FlagVerb::Other,
                };
                a.skip();
                Clause::Flag {
                    flag,
                    verb,
                    msg: a.long(),
                }
            }
            Keyword::LearnSpell => Clause::LearnSpell { spell: a.int() },
        }
    }

    /// The keyword, `None` for [`Clause::Unknown`].
    pub fn keyword(&self) -> Option<Keyword> {
        Some(match self {
            Clause::Price { .. } => Keyword::Price,
            Clause::Cast { .. } => Keyword::Cast,
            Clause::GiveAbility { .. } => Keyword::GiveAbility,
            Clause::AddAbility { .. } => Keyword::AddAbility,
            Clause::Teleport(_) => Keyword::Teleport,
            Clause::RemoveAbility { .. } => Keyword::RemoveAbility,
            Clause::NoMonsters { .. } => Keyword::NoMonsters,
            Clause::Monsters { .. } => Keyword::Monsters,
            Clause::NeedMonster { .. } => Keyword::NeedMonster,
            Clause::Summon { .. } => Keyword::Summon,
            Clause::Message { .. } => Keyword::Message,
            Clause::TakeItem { .. } => Keyword::TakeItem,
            Clause::AddExp { .. } => Keyword::AddExp,
            Clause::GiveItem { .. } => Keyword::GiveItem,
            Clause::HideItem { .. } => Keyword::HideItem,
            Clause::CheckAbility { .. } => Keyword::CheckAbility,
            Clause::Class { .. } => Keyword::Class,
            Clause::Race { .. } => Keyword::Race,
            Clause::GoodAligned { .. } => Keyword::GoodAligned,
            Clause::EvilAligned { .. } => Keyword::EvilAligned,
            Clause::MinLevel { .. } => Keyword::MinLevel,
            Clause::MaxLevel { .. } => Keyword::MaxLevel,
            Clause::CheckItem { .. } => Keyword::CheckItem,
            Clause::FailItem { .. } => Keyword::FailItem,
            Clause::Text { .. } => Keyword::Text,
            Clause::RoomText { .. } => Keyword::RoomText,
            Clause::RoomItem { .. } => Keyword::RoomItem,
            Clause::FailRoomItem { .. } => Keyword::FailRoomItem,
            Clause::ClearItem { .. } => Keyword::ClearItem,
            Clause::AddEvil { .. } => Keyword::AddEvil,
            Clause::TestAbility { .. } => Keyword::TestAbility,
            Clause::FailAbility { .. } => Keyword::FailAbility,
            Clause::RemoteAction { .. } => Keyword::RemoteAction,
            Clause::Random { .. } => Keyword::Random,
            Clause::AddDelay { .. } => Keyword::AddDelay,
            Clause::TestSkill { .. } => Keyword::TestSkill,
            Clause::CheckSkill { .. } => Keyword::CheckSkill,
            Clause::GiveCoins { .. } => Keyword::GiveCoins,
            Clause::TestTournament => Keyword::TestTournament,
            Clause::CheckSpell { .. } => Keyword::CheckSpell,
            Clause::Flag { .. } => Keyword::Flag,
            Clause::LearnSpell { .. } => Keyword::LearnSpell,
            Clause::Unknown(_) => return None,
        })
    }
}

/// The clauses of an action line as `perform_matched_action` cuts them: split at each `:`,
/// a last empty piece ending the line; each with whether a `:` followed it (what
/// `nomonsters` tests).
pub fn clauses(line: &str) -> Vec<(&str, bool)> {
    let mut pieces: Vec<&str> = line.split(':').collect();
    let n = pieces.len();
    if pieces.last().is_some_and(|p| p.is_empty()) {
        pieces.pop();
    }
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, p)| (p, i + 1 < n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Numbers read as perform_matched_action reads them
    #[test]
    fn leading_int32_reads_leading_digits() {
        assert_eq!(leading_int32("  -42x"), -42);
        assert_eq!(leading_int32("+7"), 7);
        assert_eq!(leading_int32(""), 0);
        assert_eq!(leading_int32("lever"), 0);
        assert_eq!(leading_int("70000"), 70000i32 as i16);
    }

    // A missing word reads as 0
    #[test]
    fn args_step_word_by_word() {
        let mut a = Args::of("checkitem 123   45");
        assert_eq!((a.long(), a.long(), a.long()), (123, 45, 0));
    }

    // A last empty piece ends the line; the flag is what nomonsters tests
    #[test]
    fn clauses_cut_at_each_colon() {
        assert_eq!(clauses("a:b"), [("a", true), ("b", false)]);
        assert_eq!(clauses("a::b:"), [("a", true), ("", true), ("b", true)]);
        assert!(clauses("").is_empty());
    }

    // Prefix matching in keyword order
    #[test]
    fn keywords_match_by_prefix_in_order() {
        assert_eq!(Keyword::of("nomonsters 5"), Some(Keyword::NoMonsters));
        assert_eq!(Keyword::of("MONSTERS"), Some(Keyword::Monsters));
        assert_eq!(Keyword::of("teleport_slumsewers"), Some(Keyword::Teleport));
        assert_eq!(Keyword::of("check class 3"), None);
        assert_eq!(
            Clause::parse("teleport_slumsewers", &STOCK_TELEPORTS),
            Clause::Teleport(Teleport::Named(2))
        );
        assert_eq!(
            Clause::parse("teleport_slums", &STOCK_TELEPORTS),
            Clause::Teleport(Teleport::Named(3))
        );
        assert_eq!(
            Clause::parse("testskill picklocks 10 77", &STOCK_TELEPORTS),
            Clause::TestSkill {
                skill: Skill::Picklocks,
                modifier: 10,
                block: 77
            }
        );
    }
}
