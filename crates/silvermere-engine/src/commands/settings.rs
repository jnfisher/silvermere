//! [`cmd_set`]: every `SET` option.
//!
//! The command has three shapes, decided by the word count as in the original:
//!
//! - `SET` alone lists the options.
//! - `SET {option}` toggles the option, or cycles it (`MESSAGES`), or asks for the
//!   suicide password or a number. A word that is no option is no command, so
//!   `execute_input` goes on to speech.
//! - `SET {option} {value}` sets it, or prints the option's `Valid ...` line; an option
//!   this shape does not know is ignored without a word. With more words only
//!   `SET STATLINE [FULL] CUSTOM {text}` is taken.
//!
//! Each shape is a table ordered so that the prefix ambiguity of `starts_with_ignore_case`
//! resolves as it does in the original (`SET S` is `SUICIDE`, `SET M` is `MAXPAGES` alone and
//! with a value). [`parse`] turns the words into a [`Change`] against the character as it is (a
//! toggle needs the current value); [`apply`] makes the change and prints its confirmation, which
//! is a function of the change alone, so the toggle and the two-word form share it.
//!
//! The character is not saved here; the settings reach the store at the
//! next save point (the 60 s sweep, logoff, hang-up: T-023). The custom statline is the
//! exception: it lives only in the profile, so `SET STATLINE [FULL] CUSTOM` saves that at
//! once.
//!
//! One option is new: `SET UTF8 ON|OFF` switches the session's output between CP437 and
//! UTF-8. It is the last two-word option, so it shadows
//! nothing the original had.
//! It flips `PlayerSession::caps.utf8`, the transport seam the flush reads; negotiating it
//! through telnet `CHARSET` is a later enhancement and would set the same flag.

use silvermere_rules::economy;
use silvermere_text::Msg;
use silvermere_text::statline::cut_template;
use silvermere_types::enums::{MessageLevel, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::SessionId;
use silvermere_types::names::{CURRENCY_NAMES, CURRENCY_NAMES_SINGULAR};
use silvermere_types::records::{Character, Coins};

use super::dispatch::Handled;
use super::{Cmd, CommandLine, starts_with_ignore_case};
use crate::player::profile;
use crate::state::newchar::suicide_pw;
use crate::world::World;

/// The largest page, gossip or auction limit (a byte).
pub const MAXLIMIT: i16 = 255;

/// How a two-state option was given: the one-word toggle or the two-word form. Only two
/// options word their confirmation differently by form (`STYLE`, `TALK`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Toggle,
    Value,
}

/// One `SET` change, parsed against the character's current settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// `SET` alone: the option list.
    Options,
    /// `SET SUICIDE`: the password prompts (T-022's `suicide_pw::start`).
    Suicide,
    /// `PS_KEEPEXP`.
    Keep(bool),
    /// `PS_TECHNICAL`.
    Technical(bool, Form),
    /// Gossip received (`PF_NOGOSSIP` clear).
    Gossip(bool),
    /// Auctions received (`PF_NOAUCTION` clear).
    Auction(bool),
    /// `fasttalk`.
    FastTalk(bool, Form),
    /// `PS_BRIEFTALK`.
    BriefTalk(bool),
    /// `PS_ATKINTERRUPT`.
    Interrupt(bool),
    /// `PS_EVILWARN`.
    EvilWarn(bool),
    /// Telepaths blocked (`PS_NOTELEPATH`).
    NoTelepath(bool),
    /// Game broadcasts seen (`PF_NOENTRYMSG` clear).
    Entrances(bool),
    /// `PS_BLINDFOLLOW`.
    BlindFollow(bool),
    /// Gifts accepted (`PS_NORECEIVE` clear).
    Receive(bool),
    /// `paging`.
    Paging(bool),
    /// `PF_GANGONLINE`.
    GangOnline(bool),
    /// `SET MAXPAGES` with no number.
    MaxpagesSyntax,
    /// `msglevel`.
    Messages(MessageLevel),
    /// `pagemax`.
    Maxpages(u8),
    /// `gossmax`.
    Maxgossip(u8),
    /// `auctmax`.
    Maxauction(u8),
    /// `sex`, `b'M'` or `b'F'`, paid for by the [`Change::Gender`] handler.
    Gender(u8),
    /// `palette`.
    Palette(u8),
    /// `statline` ON/OFF/FULL (no confirmation).
    Statline(StatlineMode),
    /// `statline` CUSTOM or FULL CUSTOM with its template, already cut to 60 characters.
    StatlineCustom(StatlineMode, String),
    /// `Session.caps.utf8` (new).
    Utf8(bool),
}

/// Why `SET` changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// One word that is no option: the line is no command.
    NotOption,
    /// Two or more words the command ignores without a word (handled).
    Ignored,
    /// A value the option does not take: its `Valid ...` line (boxed: `Msg` is large).
    Invalid(Box<Msg>),
}

/// The one-word options, in the order that resolves an abbreviation as the original does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum One {
    Suicide,
    Keep,
    Style,
    Gossip,
    Auction,
    Speech,
    Talk,
    Responses,
    Interrupt,
    Warning,
    Telepath,
    Entrances,
    Follow,
    Receive,
    Page,
    Gang,
    Maxpages,
    Messages,
}

const ONE_WORD: [(&str, One); 18] = [
    ("suicide", One::Suicide),
    ("keep", One::Keep),
    ("style", One::Style),
    ("gossip", One::Gossip),
    ("auction", One::Auction),
    ("speech", One::Speech),
    ("talk", One::Talk),
    ("responses", One::Responses),
    ("interrupt", One::Interrupt),
    ("warning", One::Warning),
    ("telepath", One::Telepath),
    ("entrances", One::Entrances),
    ("follow", One::Follow),
    ("receive", One::Receive),
    ("page", One::Page),
    ("gang", One::Gang),
    ("maxpages", One::Maxpages),
    ("messages", One::Messages),
];

/// The two-word options, in the order that resolves an abbreviation as the original does, then
/// the new `UTF8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Two {
    Maxpages,
    Maxgossip,
    Maxauction,
    Gender,
    Style,
    Interrupt,
    Gossip,
    Auction,
    Keep,
    Responses,
    Messages,
    Receive,
    Warning,
    Gang,
    Telepath,
    Palette,
    Entrances,
    Page,
    Statline,
    Follow,
    Speech,
    Talk,
    Utf8,
}

const TWO_WORD: [(&str, Two); 23] = [
    ("maxpages", Two::Maxpages),
    ("maxgossip", Two::Maxgossip),
    ("maxauction", Two::Maxauction),
    ("gender", Two::Gender),
    ("style", Two::Style),
    ("interrupt", Two::Interrupt),
    ("gossip", Two::Gossip),
    ("auction", Two::Auction),
    ("keep", Two::Keep),
    ("responses", Two::Responses),
    ("messages", Two::Messages),
    ("receive", Two::Receive),
    ("warning", Two::Warning),
    ("gang", Two::Gang),
    ("telepath", Two::Telepath),
    ("palette", Two::Palette),
    ("entrances", Two::Entrances),
    ("page", Two::Page),
    ("statline", Two::Statline),
    ("follow", Two::Follow),
    ("speech", Two::Speech),
    ("talk", Two::Talk),
    ("utf8", Two::Utf8),
];

/// The first entry of `table` whose word `typed` abbreviates (`starts_with_ignore_case`).
fn lookup<T: Copy>(table: &[(&str, T)], typed: &str) -> Option<T> {
    table
        .iter()
        .find(|(word, _)| starts_with_ignore_case(typed, word))
        .map(|&(_, t)| t)
}

/// The value's position among `words` (the first the typed value abbreviates), so an
/// abbreviation picks the same word as in the original.
fn choice(typed: &str, words: &[&str]) -> Option<usize> {
    words.iter().position(|w| starts_with_ignore_case(typed, w))
}

/// `leading_int` on the host's 16-bit `int`: leading blanks, a sign, then digits up to the
/// first non-digit, wrapping as the 16-bit multiply-add did; nothing numeric is 0.
pub fn leading_int16(text: &str) -> i16 {
    let s = text.trim_start_matches([' ', '\t']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i16 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        n = n.wrapping_mul(10).wrapping_add(i16::from(b - b'0'));
    }
    if neg { n.wrapping_neg() } else { n }
}

/// The stored limit for `SET MAX... {n}`: `leading_int`, cut to `MAXLIMIT`, then stored in an
/// `unsigned char` (a negative number wraps: `-5` is 251).
pub fn limit(text: &str) -> u8 {
    let val = leading_int16(text).min(MAXLIMIT);
    val as u8
}

/// The option parsing of SET: the change `line` asks of `plr` (whose current settings the toggles
/// read).
pub fn parse(line: &CommandLine, plr: &Character) -> Result<Change, SetError> {
    match line.argc() {
        0 | 1 => Ok(Change::Options),
        2 => one_word(line.arg(1), plr),
        3 => two_words(line.arg(1), line.arg(2)),
        _ => statline_custom(line),
    }
}

/// `SET {option}`.
fn one_word(option: &str, plr: &Character) -> Result<Change, SetError> {
    let st = plr.settings;
    let fl = plr.flags;
    Ok(
        match lookup(&ONE_WORD, option).ok_or(SetError::NotOption)? {
            One::Suicide => Change::Suicide,
            One::Keep => Change::Keep(!st.contains(PlayerSettings::KEEPEXP)),
            One::Style => Change::Technical(!st.contains(PlayerSettings::TECHNICAL), Form::Toggle),
            One::Gossip => Change::Gossip(fl.contains(PlayerFlags::NOGOSSIP)),
            One::Auction => Change::Auction(fl.contains(PlayerFlags::NOAUCTION)),
            One::Speech | One::Talk => Change::FastTalk(plr.fasttalk == 0, Form::Toggle),
            One::Responses => Change::BriefTalk(!st.contains(PlayerSettings::BRIEFTALK)),
            One::Interrupt => Change::Interrupt(!st.contains(PlayerSettings::ATKINTERRUPT)),
            One::Warning => Change::EvilWarn(!st.contains(PlayerSettings::EVILWARN)),
            One::Telepath => Change::NoTelepath(!st.contains(PlayerSettings::NOTELEPATH)),
            One::Entrances => Change::Entrances(fl.contains(PlayerFlags::NOENTRYMSG)),
            One::Follow => Change::BlindFollow(!st.contains(PlayerSettings::BLINDFOLLOW)),
            One::Receive => Change::Receive(st.contains(PlayerSettings::NORECEIVE)),
            One::Page => Change::Paging(plr.paging == 0),
            One::Gang => Change::GangOnline(!fl.contains(PlayerFlags::GANGONLINE)),
            One::Maxpages => Change::MaxpagesSyntax,
            // High -> Low -> Medium -> High
            One::Messages => Change::Messages(match plr.msglevel {
                MessageLevel::High => MessageLevel::Low,
                MessageLevel::Medium => MessageLevel::High,
                _ => MessageLevel::Medium,
            }),
        },
    )
}

/// A two-word option whose values are `words`, each giving a change; any other value
/// prints `invalid`.
fn pick(value: &str, words: &[&str], changes: &[Change], invalid: Msg) -> Result<Change, SetError> {
    choice(value, words)
        .map(|i| changes[i].clone())
        .ok_or_else(|| SetError::Invalid(Box::new(invalid)))
}

/// `SET {option} {value}`, then the new `UTF8`.
fn two_words(option: &str, value: &str) -> Result<Change, SetError> {
    use Change as C;
    let Some(two) = lookup(&TWO_WORD, option) else {
        return Err(SetError::Ignored);
    };
    match two {
        Two::Maxpages => Ok(C::Maxpages(limit(value))),
        Two::Maxgossip => Ok(C::Maxgossip(limit(value))),
        Two::Maxauction => Ok(C::Maxauction(limit(value))),
        // the value is read after the price is checked: see `gender`
        Two::Gender => Ok(match choice(value, &["male", "female"]) {
            Some(0) => C::Gender(b'M'),
            Some(_) => C::Gender(b'F'),
            None => C::Gender(0),
        }),
        Two::Style => pick(
            value,
            &["technical", "fantasy"],
            &[
                C::Technical(true, Form::Value),
                C::Technical(false, Form::Value),
            ],
            Msg::SetStyleOptions,
        ),
        Two::Interrupt => pick(
            value,
            &["On", "Off"],
            &[C::Interrupt(true), C::Interrupt(false)],
            Msg::SetInterruptOptions,
        ),
        Two::Gossip => pick(
            value,
            &["Off", "On"],
            &[C::Gossip(false), C::Gossip(true)],
            Msg::SetGossipOptions,
        ),
        Two::Auction => pick(
            value,
            &["Off", "On"],
            &[C::Auction(false), C::Auction(true)],
            Msg::SetAuctionOptions,
        ),
        Two::Keep => pick(
            value,
            &["off", "on"],
            &[C::Keep(false), C::Keep(true)],
            Msg::SetKeepOptions,
        ),
        Two::Responses => pick(
            value,
            &["brief", "verbose"],
            &[C::BriefTalk(true), C::BriefTalk(false)],
            Msg::SetResponsesOptions,
        ),
        Two::Messages => pick(
            value,
            &["High", "Medium", "Low"],
            &[
                C::Messages(MessageLevel::High),
                C::Messages(MessageLevel::Medium),
                C::Messages(MessageLevel::Low),
            ],
            Msg::SetMessagesOptions,
        ),
        // (sic) the INTERRUPT line
        Two::Receive => pick(
            value,
            &["On", "Off"],
            &[C::Receive(true), C::Receive(false)],
            Msg::SetInterruptOptions,
        ),
        Two::Warning => pick(
            value,
            &["On", "Off"],
            &[C::EvilWarn(true), C::EvilWarn(false)],
            Msg::SetWarningOptions,
        ),
        Two::Gang => pick(
            value,
            &["online", "all"],
            &[C::GangOnline(true), C::GangOnline(false)],
            Msg::SetGangOptions,
        ),
        Two::Telepath => pick(
            value,
            &["On", "Off"],
            &[C::NoTelepath(false), C::NoTelepath(true)],
            Msg::SetTelepathOptions,
        ),
        Two::Palette => pick(
            value,
            &["0", "1", "2", "3"],
            &[C::Palette(0), C::Palette(1), C::Palette(2), C::Palette(3)],
            Msg::SetPaletteOptions,
        ),
        Two::Entrances => pick(
            value,
            &["On", "Off"],
            &[C::Entrances(true), C::Entrances(false)],
            Msg::SetEntranceOptions,
        ),
        Two::Page => pick(
            value,
            &["On", "Ok", "Off"],
            &[C::Paging(true), C::Paging(true), C::Paging(false)],
            Msg::SetPageOptions,
        ),
        Two::Statline => pick(
            value,
            &["On", "Off", "Full"],
            &[
                C::Statline(StatlineMode::On),
                C::Statline(StatlineMode::Off),
                C::Statline(StatlineMode::Full),
            ],
            Msg::SetStatlineOptions,
        ),
        Two::Follow => pick(
            value,
            &["blind", "normal"],
            &[C::BlindFollow(true), C::BlindFollow(false)],
            Msg::SetFollowOptions,
        ),
        Two::Speech | Two::Talk => pick(
            value,
            &["fast", "slow"],
            &[
                C::FastTalk(true, Form::Value),
                C::FastTalk(false, Form::Value),
            ],
            Msg::SetTalkOptions,
        ),
        Two::Utf8 => pick(
            value,
            &["on", "off"],
            &[C::Utf8(true), C::Utf8(false)],
            Msg::SetUtf8Options,
        ),
    }
}

/// `SET STATLINE FULL CUSTOM {text}` and `SET STATLINE CUSTOM {text}`: the text runs to
/// the end of the raw line, cut to 60 characters. Any other line of four or more words is ignored.
fn statline_custom(line: &CommandLine) -> Result<Change, SetError> {
    if !starts_with_ignore_case(line.arg(1), "statline") {
        return Err(SetError::Ignored);
    }
    let (mode, from) = if line.argc() > 4 && starts_with_ignore_case(line.arg(2), "full") {
        if !starts_with_ignore_case(line.arg(3), "custom") {
            return Err(SetError::Ignored);
        }
        (StatlineMode::FullCustom, 4)
    } else if starts_with_ignore_case(line.arg(2), "custom") {
        (StatlineMode::Custom, 3)
    } else {
        return Err(SetError::Ignored);
    };
    let text = cut_template(line.rest_from(from)).to_string();
    Ok(Change::StatlineCustom(mode, text))
}

/// The confirmation of a two-state change.
fn confirmation(change: &Change) -> Option<Msg> {
    use Change as C;
    Some(match *change {
        C::Keep(true) => Msg::SetKeepOn,
        C::Keep(false) => Msg::SetKeepOff,
        C::Technical(true, Form::Toggle) => Msg::SetStyleTechnical,
        // (sic) the original's misspelt line
        C::Technical(true, Form::Value) => Msg::SetStyleTechincal,
        C::Technical(false, _) => Msg::SetStyleFantasy,
        C::Gossip(true) => Msg::SetGossipOn,
        C::Gossip(false) => Msg::SetGossipOff,
        C::Auction(true) => Msg::SetAuctionOn,
        C::Auction(false) => Msg::SetAuctionOff,
        C::FastTalk(true, Form::Toggle) => Msg::SetTalkFast,
        C::FastTalk(false, Form::Toggle) => Msg::SetTalkSlow,
        C::FastTalk(true, Form::Value) => Msg::SetTalkFastLine,
        C::BriefTalk(true) => Msg::SetResponsesBrief,
        C::BriefTalk(false) => Msg::SetResponsesVerbose,
        C::Interrupt(true) => Msg::SetInterruptOn,
        C::Interrupt(false) => Msg::SetInterruptOff,
        C::EvilWarn(true) => Msg::SetWarningOn,
        C::EvilWarn(false) => Msg::SetWarningOff,
        C::NoTelepath(true) => Msg::SetTelepathBlocked,
        C::NoTelepath(false) => Msg::SetTelepathAllowed,
        C::Entrances(true) => Msg::SetEntrancesOn,
        C::Entrances(false) => Msg::SetEntrancesOff,
        C::BlindFollow(true) => Msg::SetFollowBlind,
        C::BlindFollow(false) => Msg::SetFollowNormal,
        C::Receive(true) => Msg::SetReceiveOn,
        C::Receive(false) => Msg::SetReceiveOff,
        C::Paging(true) => Msg::SetPaging { state: "Ok".into() },
        C::Paging(false) => Msg::SetPaging {
            state: "Off".into(),
        },
        C::GangOnline(true) => Msg::SetGangOnline,
        C::GangOnline(false) => Msg::SetGangAll,
        C::Messages(MessageLevel::High) => Msg::SetMessagesHigh,
        C::Messages(MessageLevel::Medium) => Msg::SetMessagesMedium,
        C::Messages(MessageLevel::Low) => Msg::SetMessagesLow,
        C::Maxpages(n) => Msg::SetMaxpages { n },
        C::Maxgossip(n) => Msg::SetMaxgossip { n },
        C::Maxauction(n) => Msg::SetMaxauction { n },
        C::Palette(n) => Msg::SetPalette { n },
        C::Utf8(on) => Msg::SetUtf8 {
            state: if on { "on" } else { "off" }.into(),
        },
        C::MaxpagesSyntax => Msg::SetMaxpagesSyntax,
        C::Options => Msg::SetOptions,
        _ => return None,
    })
}

/// The effects of SET: `change` made to the session's character, then its confirmation.
/// The answer is the command's result.
pub fn apply(world: &mut World, id: SessionId, change: Change) -> Handled {
    use Change as C;
    match change {
        C::Suicide => {
            suicide_pw::start(world, id);
            // no prompt: the password question follows
            return Handled::Wait;
        }
        C::Gender(sex) => return gender(world, id, sex),
        C::StatlineCustom(mode, text) => {
            if let Some(s) = world.session_mut(id) {
                if let Some(plr) = s.character.as_mut() {
                    plr.statline = mode;
                }
                s.ust.statline = text;
            }
            // the profile is saved at once; nothing printed
            let keep = world
                .session(id)
                .and_then(|s| s.character.as_ref())
                .is_some_and(|c| profile::keep_exp(world, c));
            profile::save(world, id, keep);
            return Handled::Done;
        }
        _ => {}
    }
    let keep_disabled = !world.config.keep_exp;
    let talk_char = world.config.talk_char;
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    let st = &mut plr.settings;
    let fl = &mut plr.flags;
    match change {
        C::Keep(on) => st.set(PlayerSettings::KEEPEXP, on),
        C::Technical(on, _) => st.set(PlayerSettings::TECHNICAL, on),
        C::Gossip(on) => fl.set(PlayerFlags::NOGOSSIP, !on),
        C::Auction(on) => fl.set(PlayerFlags::NOAUCTION, !on),
        C::FastTalk(on, _) => plr.fasttalk = u8::from(on),
        C::BriefTalk(on) => st.set(PlayerSettings::BRIEFTALK, on),
        C::Interrupt(on) => st.set(PlayerSettings::ATKINTERRUPT, on),
        C::EvilWarn(on) => st.set(PlayerSettings::EVILWARN, on),
        C::NoTelepath(on) => st.set(PlayerSettings::NOTELEPATH, on),
        C::Entrances(on) => fl.set(PlayerFlags::NOENTRYMSG, !on),
        C::BlindFollow(on) => st.set(PlayerSettings::BLINDFOLLOW, on),
        C::Receive(on) => st.set(PlayerSettings::NORECEIVE, !on),
        C::Paging(on) => plr.paging = u8::from(on),
        C::GangOnline(on) => fl.set(PlayerFlags::GANGONLINE, on),
        C::Messages(level) => plr.msglevel = level,
        C::Maxpages(n) => plr.pagemax = n,
        C::Maxgossip(n) => plr.gossmax = n,
        C::Maxauction(n) => plr.auctmax = n,
        C::Palette(n) => plr.palette = n,
        // no confirmation
        C::Statline(mode) => plr.statline = mode,
        C::Utf8(on) => s.caps.utf8 = on,
        C::Options | C::MaxpagesSyntax => {}
        C::Suicide | C::Gender(_) | C::StatlineCustom(..) => unreachable!("handled above"),
    }
    match &change {
        // the talk character is the sysop's
        C::FastTalk(false, Form::Value) => s.output.push(Msg::SetTalkSlowLine { ch: talk_char }),
        other => {
            if let Some(msg) = confirmation(other) {
                s.output.push(msg);
            }
        }
    }
    // the choice is kept, but the sysop's KEEPEXP decides whether it counts
    if change == C::Keep(true) && keep_disabled {
        s.output.push(Msg::SetKeepDisabled);
    }
    Handled::Done
}

/// `SET GENDER {MALE|FEMALE}`: a negative GENDERGO forbids it; otherwise the price is checked
/// before the word (`sex` 0 is neither), then paid with the coins listed, or free when GENDERGO is
/// 0.
fn gender(world: &mut World, id: SessionId, sex: u8) -> Handled {
    let gold = world.config.gender_gold;
    let currency = world.config.usr_cur_name.clone();
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    let Ok(gold) = u32::try_from(gold) else {
        s.output.push(Msg::SetGenderForbidden);
        return Handled::Done;
    };
    let price = economy::convert_currency(0, 0, gold, 0, 0);
    if !economy::can_pay(plr.coins, price) {
        s.output.push(Msg::SetCannotAfford);
        return Handled::Done;
    }
    let word = match sex {
        b'M' => "male",
        b'F' => "female",
        _ => {
            s.output.push(Msg::SetGenderOptions);
            return Handled::Done;
        }
    };
    plr.sex = sex;
    let gender = word.to_string();
    if gold == 0 {
        s.output.push(Msg::SetGenderFree { gender });
        return Handled::Done;
    }
    let paid = match economy::pay(plr.coins, price) {
        Some(p) => {
            plr.coins = p.left;
            p.paid
        }
        None => Coins::default(),
    };
    let coins = coins_handed_over(paid, &currency);
    s.output.push(Msg::SetGenderPaid { coins, gender });
    Handled::Done
}

/// What a payment that lists its coins shows after "You hand over": each coin type paid, largest
/// first, as ` <n> <name>` separated by commas (`proper_currency_name`: singular for one; the top
/// currency always the sysop's `USRCURNM`), or ` nothing`.
pub fn coins_handed_over(paid: Coins, usr_cur_name: &str) -> String {
    let name = |n: u32, cur: usize| {
        if n == 1 {
            CURRENCY_NAMES_SINGULAR[cur]
        } else {
            CURRENCY_NAMES[cur]
        }
    };
    let parts = [
        (paid.runic, usr_cur_name),
        (paid.platinum, name(paid.platinum, 3)),
        (paid.gold, name(paid.gold, 2)),
        (paid.silver, name(paid.silver, 1)),
        (paid.copper, name(paid.copper, 0)),
    ];
    let listed: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n != 0)
        .map(|(n, name)| format!(" {n} {name}"))
        .collect();
    if listed.is_empty() {
        " nothing".into()
    } else {
        listed.join(",")
    }
}

/// SET, with every option.
pub fn cmd_set(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = world.session(id).and_then(|s| s.character.as_ref()) else {
        return Handled::NotCommand;
    };
    match parse(line, plr) {
        Ok(change) => apply(world, id, change),
        Err(SetError::NotOption) => Handled::NotCommand,
        Err(SetError::Ignored) => Handled::Done,
        Err(SetError::Invalid(msg)) => {
            if let Some(s) = world.session_mut(id) {
                s.output.push(*msg);
            }
            Handled::Done
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // leading_int, MAXLIMIT, the unsigned char store
    #[test]
    fn limits() {
        assert_eq!(limit("5"), 5);
        assert_eq!(limit("255"), 255);
        assert_eq!(limit("256"), 255);
        assert_eq!(limit("abc"), 0);
        assert_eq!(limit("12abc"), 12);
        assert_eq!(limit("-5"), 251);
        assert_eq!(limit("70000"), 255); // 70000 wraps to 4464 in 16 bits
        assert_eq!(limit("65536"), 0);
    }

    // the coins listed after "You hand over"
    #[test]
    fn coin_list() {
        let c = |runic, platinum, gold, silver, copper| Coins {
            runic,
            platinum,
            gold,
            silver,
            copper,
        };
        assert_eq!(
            coins_handed_over(c(0, 0, 0, 0, 0), "runic coins"),
            " nothing"
        );
        assert_eq!(
            coins_handed_over(c(0, 0, 2, 5, 0), "runic coins"),
            " 2 gold crowns, 5 silver nobles"
        );
        assert_eq!(
            coins_handed_over(c(1, 1, 0, 0, 1), "runic coins"),
            " 1 runic coins, 1 platinum piece, 1 copper farthing"
        );
    }
}
