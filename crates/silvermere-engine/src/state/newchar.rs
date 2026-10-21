//! `NewChar(..)` and `RaceChange(..)` (T-047): character creation from the `[MAJORMUD]:`
//! menu.
//!
//! The creation flow, one function per state:
//!
//! | State | What happens | Here |
//! |---|---|---|
//! | the menu's `E` with no character | `create_player`, the race list | [`begin`] |
//! | `NewCharStep::Race` | the race is set, then the class list | [`on_race`] |
//! | `NewCharStep::Class` | the class is set, the lawful question or the editor | [`on_class`] |
//! | `NewCharStep::Lawful` | Y/N, `roll_stats`, `edit_character_stats` | [`on_lawful`] |
//! | `NewCharStep::Stats` | [`stat_editor`], full screen or line mode | [`on_stats`], [`editor_done`] |
//! | `NewCharStep::Validating`, `NewCharStep::Name` | the name check | [`validate_name`] |
//! | `Suicide(..)` | the suicide password | [`suicide_pw`] |
//!
//! The editor runs as the original's full-screen form (`EDITCHA1`) on an ANSI terminal of
//! at least 80x23 ([`crate::commands::fullscreen_editor`], T-063) and as the original's
//! line-mode path (the field prompts [`stat_editor`]'s) otherwise. Both are views over the
//! same [`StatEditor`] and its field check.
//!
//! The same editor serves an existing character (`TRAIN STATS`, and a stat reset at
//! entry; T-047): no first-name field, and [`editor_done`] sends the character back into
//! the realm. `RaceChange(..)` is [`race_change`]'s (T-047).

pub mod race_change;
pub mod stat_editor;
pub mod suicide_pw;
pub mod validate_name;

use silvermere_text::Msg;
use silvermere_types::enums::{NewCharStep, SessionState};
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus2};
use silvermere_types::ids::{ClassNo, RaceNo, SessionId};
use silvermere_types::records::Character;

use crate::commands::fullscreen_editor;
use crate::state::login::{menu_bytes, show_menu};
use crate::state::{Input, Outcome};
use crate::world::World;
use stat_editor::{EditorDone, Exit, StatEditor};

pub use crate::player::STARTSTAT;
pub use crate::player::create::{NEW_PLAYER_ENERGY, NEW_PLAYER_HP};
/// `MAXLIVES`: lives a new character starts with.
pub const MAXLIVES: i16 = 9;
/// The record version of MajorMUD 1.11p.
pub const PLAYER_VERSION: i16 = 22;

/// One input line in a state this module owns.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    let Some(s) = world.session(input.id) else {
        return Outcome::Logoff;
    };
    match s.state {
        SessionState::NewChar(NewCharStep::Race) => on_race(world, input),
        SessionState::NewChar(NewCharStep::Class) => on_class(world, input),
        SessionState::NewChar(NewCharStep::Lawful) => on_lawful(world, input),
        SessionState::NewChar(NewCharStep::Stats) => on_stats(world, input),
        SessionState::NewChar(NewCharStep::Validating) => {
            validate_name::on_validating(world, input)
        }
        SessionState::NewChar(NewCharStep::Name) => validate_name::on_new_name(world, input),
        SessionState::RaceChange(step) => race_change::on_line(world, input, step),
        state => Outcome::Unimplemented(state),
    }
}

/// The line split at blanks into words, as the host does it.
pub(crate) fn words(line: &str) -> Vec<&str> {
    line.split_ascii_whitespace().collect()
}

/// The leading number of a word, 0 when there is none.
pub(crate) fn leading_int(word: &str) -> i64 {
    let bytes = word.as_bytes();
    let mut i = 0;
    let neg = match bytes.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut n: i64 = 0;
    while let Some(d) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    if neg { -n } else { n }
}

/// The menu's "no character yet" path: `create_player`, then
/// `Please choose a race from the following list:`, the race list and `CHSRACE`; the
/// session is at [`NewCharStep::Race`]. T-021's menu calls it for `E`/`P` when
/// `load_player` finds no character.
///
/// `today` is the day as the database stores it (`event_loop::day_start`).
pub fn begin(world: &mut World, id: SessionId, today: i64) {
    let Some((account, userid)) = world
        .session(id)
        .and_then(|s| s.account.as_ref())
        .map(|a| (a.id, a.username.clone()))
    else {
        return;
    };
    // `create_player` (T-023), with the account's kept profile and evil points; the
    // session half of the profile (the custom statline text, the day byte) goes into the
    // session.
    let c = crate::player::create_player(world, account, &userid, today);
    let profile = crate::player::profile::lookup(world, account).cloned();
    let Some(s) = world.session_mut(id) else {
        return;
    };
    if let Some(rec) = profile {
        crate::player::profile::restore_session(&mut s.ust, &rec, c.deathdate, today);
    }
    // `create_player` starts from a zeroed record (`clear_player`), `dragging` 0 included, and
    // the new character goes on into the realm without a `load_player` that would read
    // one back: his first marched step is refused
    s.party = crate::party::Party::from_record(c.dragging);
    s.character = Some(c);
    s.created = false;
    s.editor = None;
    s.output.push(Msg::ChooseRaceList);
    s.state = SessionState::NewChar(NewCharStep::Race);
    display_race_list(world, id);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::Chsrace);
    }
}

/// The race list: an ANSI reset, then one `[n]  name` row per race in race-number order
/// (the Btrieve key walk); the list stops after 200 races.
pub fn display_race_list(world: &mut World, id: SessionId) {
    let mut rows: Vec<(u16, String)> = world
        .races
        .values()
        .map(|r| (r.number.0, r.name.clone()))
        .collect();
    rows.sort_unstable();
    rows.truncate(200);
    push_list(world, id, rows);
}

/// The class list: as the race list, stopping after 100 classes.
pub fn display_class_list(world: &mut World, id: SessionId) {
    let mut rows: Vec<(u16, String)> = world
        .classes
        .values()
        .map(|c| (c.number.0, c.name.clone()))
        .collect();
    rows.sort_unstable();
    rows.truncate(100);
    push_list(world, id, rows);
}

fn push_list(world: &mut World, id: SessionId, rows: Vec<(u16, String)>) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    // ANSI reset
    s.output.push(Msg::AnsiNormal);
    for (n, name) in rows {
        // one more blank after a one-digit number
        s.output.push(if n < 10 {
            Msg::ListRowShort { n, name }
        } else {
            Msg::ListRowLong { n, name }
        });
    }
}

/// The `X` exits: back to the `[MAJORMUD]:` menu through T-021's [`show_menu`], in the
/// main menu state.
pub(crate) fn back_to_menu(world: &mut World, id: SessionId) {
    show_menu(world, id);
    if let Some(s) = world.session_mut(id) {
        s.editor = None;
        s.state = SessionState::MainMenu;
    }
}

/// The character cleared on the way back to the menu (the "start over again" exits): the
/// slot is wiped, so the menu sees no character.
pub(crate) fn clear_and_menu(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
        s.character = None;
        s.editor = None;
        s.state = SessionState::MainMenu;
    }
}

/// The `?`/`HELP` branch of the list prompts: `?` alone prints the list again; `? topic`
/// is the help text of the second word.
///
/// TODO T-027: `help_text` is T-027's; a topic prints nothing yet.
pub(crate) fn help_or_list(
    world: &mut World,
    id: SessionId,
    w: &[&str],
    list: fn(&mut World, SessionId),
) {
    if w.len() == 1 {
        list(world, id);
    } else {
        tracing::debug!(?id, topic = w[1], "help topic at the creation list: T-027");
    }
}

/// `NewCharStep::Race` (the new-character path).
pub fn on_race(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(first) = w.first() else {
        tell(world, id, Msg::MustChooseRace);
        return Outcome::Continue;
    };
    if first.to_ascii_uppercase().starts_with('X') {
        back_to_menu(world, id);
        return Outcome::Continue;
    }
    if first.starts_with('?') || first.eq_ignore_ascii_case("help") {
        help_or_list(world, id, &w, display_race_list);
        return Outcome::Continue;
    }
    // The race number is the first word's leading number; the race must exist in the race
    // file.
    let racenum = leading_int(first);
    let race = u16::try_from(racenum)
        .ok()
        .map(RaceNo)
        .filter(|r| world.races.contains_key(r));
    let Some(race) = race else {
        tell(world, id, Msg::MustChooseValidRace);
        return Outcome::Continue;
    };
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let Some(plr) = s.character.as_mut() else {
        // no race record: the race is refused
        s.output.push(Msg::MustChooseValidRace);
        return Outcome::Continue;
    };
    plr.race = race;
    s.output.push(Msg::ChooseClassList);
    s.state = SessionState::NewChar(NewCharStep::Class);
    display_class_list(world, id);
    tell(world, id, Msg::Chsclass);
    Outcome::Continue
}

/// `NewCharStep::Class` (the new-character path).
pub fn on_class(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(first) = w.first() else {
        tell(world, id, Msg::MustChooseClass);
        return Outcome::Continue;
    };
    if first.to_ascii_uppercase().starts_with('X') {
        back_to_menu(world, id);
        return Outcome::Continue;
    }
    if first.starts_with('?') || first.eq_ignore_ascii_case("help") {
        help_or_list(world, id, &w, display_class_list);
        return Outcome::Continue;
    }
    // The class number is the first word's leading number.
    let classnum = leading_int(first);
    let class = u16::try_from(classnum)
        .ok()
        .map(ClassNo)
        .filter(|c| world.classes.contains_key(c));
    let Some(class) = class else {
        tell(world, id, Msg::MustChooseValidClass);
        return Outcome::Continue;
    };
    let enable_lawful = world.config.enable_lawful;
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let Some(plr) = s.character.as_mut() else {
        s.output.push(Msg::MustChooseValidClass);
        return Outcome::Continue;
    };
    plr.class = class;
    // A race that starts evil, or a server without ENABLAWF, rolls at once; otherwise the
    // lawful question (GOODTEXT, GOODPMPT).
    if plr.evil > 0 || !enable_lawful {
        roll_stats(world, id);
        edit_character_stats(world, id, true);
    } else {
        s.output.push(Msg::Goodtext);
        s.output.push(Msg::Goodpmpt);
        s.state = SessionState::NewChar(NewCharStep::Lawful);
    }
    Outcome::Continue
}

/// `NewCharStep::Lawful`: `Y` makes the character lawful for life (`lawful -10`,
/// `evil -51`; evil actions are refused from then on), `N` takes the force off; anything else
/// asks again.
pub fn on_lawful(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let answer = (w.len() == 1)
        .then(|| w[0].as_bytes()[0].to_ascii_uppercase())
        .filter(|c| matches!(c, b'Y' | b'N'));
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    match answer {
        Some(b'Y') => {
            if let Some(plr) = s.character.as_mut() {
                plr.lawful = -10;
                plr.evil = -51;
            }
        }
        Some(_) => {
            // negative evil points become 0
            if let Some(plr) = s.character.as_mut() {
                plr.lawful = 0;
                if plr.evil < 0 {
                    plr.evil = 0;
                }
            }
        }
        None => {
            s.output.push(Msg::LawfulYOrN);
            s.output.push(Msg::Goodpmpt);
            return Outcome::Continue;
        }
    }
    roll_stats(world, id);
    edit_character_stats(world, id, true);
    Outcome::Continue
}

/// T-023's [`crate::player::roll_stats`] on the session's character, with the account's
/// sex, the secondary stats in mode 0 and the class title.
pub fn roll_stats(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let sex = s.account.as_ref().map_or(b'M', |a| a.sex);
    let Some(mut c) = s.character.take() else {
        return;
    };
    crate::player::roll_stats(world, &mut c, sex);
    if let Some(s) = world.sessions.get_mut(&id) {
        s.character = Some(c);
    }
}

/// The stat editor. On an ANSI screen of at least 80x23 the full-screen form opens and
/// the session goes to character mode (T-063); otherwise the line-mode path: the looks
/// are kept inside the tables, the CP cost chart and the race's range of every stat are
/// printed, and the editor's first field is prompted. The session is at
/// [`NewCharStep::Stats`] until SAVE, EDIT or QUIT; that state is not `Playing`, so the
/// character is out of the realm meanwhile, as in the original.
///
/// `newchar` false (`TRAIN STATS`, a stat reset) locks the first name:
/// [`StatEditor::open_existing`]. The chart, the ranges and the CP line are the same
/// bytes either way.
pub fn edit_character_stats(world: &mut World, id: SessionId, newchar: bool) {
    let blanklast = world.config.blank_lastname_allowed;
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    let (Some(rac), Some(cls)) = (world.races.get(&plr.race), world.classes.get(&plr.class)) else {
        return;
    };
    // hair lengths 0..5, hair colours 0..9, eye colours 0..16
    plr.hairlen = plr.hairlen.min(5);
    plr.haircol = plr.haircol.min(9);
    plr.eyecol = plr.eyecol.min(16);
    let mut editor = if newchar {
        StatEditor::open(plr, rac, blanklast)
    } else {
        StatEditor::open_existing(plr, rac, blanklast)
    };
    // the EDITCHA1 form on an ANSI screen of at least 80x23 (T-063)
    if fullscreen_editor::fits(&s.caps) && fullscreen_editor::installed() {
        fullscreen_editor::open(&mut editor, &rac.name, &cls.name, &mut s.output);
        s.editor = Some(editor);
        s.state = SessionState::NewChar(NewCharStep::Stats);
        world
            .outbox
            .push(crate::EngineOutput::SetCharMode { id, on: true });
        world.touch(id);
        return;
    }
    s.output.push(Msg::StatEditorPreamble);
    // (sic) a stray '\0' after each number makes the second %d print 0 instead of the
    // race maximum.
    for (label, min) in [
        ("Strength  ", rac.minstr),
        ("Intellect ", rac.minint),
        ("Willpower ", rac.minwil),
        ("Agility   ", rac.minagi),
        ("Health    ", rac.minhea),
        ("Charm     ", rac.mincha),
    ] {
        s.output.push(Msg::StatRange {
            label: label.to_string(),
            min: min.max(0) as u16,
            max: 0,
        });
    }
    // the CP left: totalcp - (totalcp - cp)
    s.output.push(Msg::CpToDistribute {
        cp: i32::from(plr.cp),
    });
    s.output.push_raw(editor.prompt());
    s.editor = Some(editor);
    s.state = SessionState::NewChar(NewCharStep::Stats);
    world.touch(id);
}

/// One line typed into the editor: the field is checked, then the next prompt; SAVE and
/// QUIT finish the editor.
pub fn on_stats(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let Some(editor) = s.editor.as_mut() else {
        // No editor open in the editor state: back out to the menu.
        back_to_menu(world, id);
        return Outcome::Continue;
    };
    match editor.on_line(input.line, &mut s.output) {
        None => {}
        Some(Exit::Save) => editor_done(world, id, true),
        Some(Exit::Quit) => editor_done(world, id, false),
    }
    Outcome::Continue
}

/// The editor finished, for a new character.
///
/// With `save`: the chosen stats become base and current values and their CP cost is
/// charged; the last name and the first name are checked, a failure sends the player back
/// to the menu with the record wiped; then the character is marked not validated, the
/// suicide-password reminder is printed, the "game full" check runs against
/// `max_sessions` and the name check starts. Without `save`: the menu (twice, sic).
///
/// Not carried: the full-screen teardown, the paying-account flag and the unpaid-account
/// texts, the secondary stats and `save_player` (T-023: a record that is not validated is
/// never saved), the last-played day (T-023), the user list rebuild (T-027).
pub fn editor_done(world: &mut World, id: SessionId, save: bool) {
    let blanklast = world.config.blank_lastname_allowed;
    let max_in_game = usize::from(world.ids.max());
    // the players in the realm right now
    let ningame = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .count();
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(editor) = s.editor.take() else {
        return;
    };
    if editor.on_done == EditorDone::TrainStats {
        existing_done(world, id, &editor, save, ningame);
        return;
    }
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    // back to line mode, in the game state
    s.state = SessionState::Playing;
    if !save {
        // A new character that was not saved goes back to the main menu; the original
        // prints the menu twice (sic).
        back_to_menu(world, id);
        let menu = menu_bytes(world, id);
        if let Some(s) = world.session_mut(id) {
            s.output.push_raw(menu);
        }
        return;
    }
    apply_stats(plr, &editor);
    // the last name, trimmed, letters only, 18 characters at most
    let mut lastname: String = editor.family_name.trim().chars().take(18).collect();
    let last_ok = lastname.chars().all(|c| c.is_ascii_alphabetic());
    if lastname.is_empty() && !blanklast {
        clear_and_menu(world, id, Msg::LastNameMustStartOver);
        return;
    }
    if !last_ok {
        clear_and_menu(world, id, Msg::LastNameInvalidStartOver);
        return;
    }
    if let Some(first) = lastname.get(..1) {
        // The field check capitalised the field; it is stored as answered.
        let up = first.to_ascii_uppercase();
        lastname.replace_range(..1, &up);
    }
    plr.lastname = lastname;
    plr.haircol = editor.haircol;
    plr.hairlen = editor.hairlen;
    plr.eyecol = editor.eyecol;
    // the first name, trimmed, letters only, 10 characters at most
    let firstname: String = editor.given_name.trim().chars().take(10).collect();
    let first_ok = firstname.chars().all(|c| c.is_ascii_alphabetic());
    if firstname.is_empty() || !first_ok {
        clear_and_menu(world, id, Msg::NameInvalidStartOver);
        return;
    }
    plr.firstname = capitalise(&firstname);
    // the name may not be a game command; parse_command wants it in upper case
    let word: String = plr
        .firstname
        .chars()
        .take(10)
        .collect::<String>()
        .to_ascii_uppercase();
    if validate_name::parse_command(&word) {
        clear_and_menu(world, id, Msg::NameInUseStartOver);
        return;
    }
    plr.settings.remove(PlayerSettings::VALIDATED);
    s.state = SessionState::NewChar(NewCharStep::Validating);
    // The secondary stats, in mode 2 when the 32-bit `exp` field is above 0 (a 1.11
    // character and a kept profile leave it at 0).
    let mode = if plr.exp > 0 {
        crate::player::CalcMode::KeepCurrent
    } else {
        crate::player::CalcMode::Refill
    };
    let mut c = plr.clone();
    crate::player::recalculate(world, &mut c, mode);
    // No save: it is refused, the name is not validated yet.
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.character = Some(c);
    crate::player::save_at(world, id, crate::player::SavePoint::EnterRealm);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if plr.suicpw.is_empty() {
        s.output.push(Msg::SuicidePwNotice);
    }
    // the game is full (maxingame is max_sessions here)
    if max_in_game <= ningame {
        s.output.push(Msg::GameFull);
        s.output.push(Msg::MudMenuPromptNormal);
        s.state = SessionState::MainMenu;
    }
    // a new name is checked in the background
    s.output.push(Msg::NameValidating);
    validate_name::begin(world, id);
}

/// The six totals become the base and current stats, and their cost above the race
/// minimums is charged against `totalcp`.
fn apply_stats(plr: &mut Character, editor: &StatEditor) {
    let used = editor.used();
    plr.cp = i16::try_from(i64::from(plr.totalcp) - i64::from(used)).unwrap_or(i16::MIN);
    let v = editor.values;
    plr.basstr = v[0];
    plr.str = v[0];
    plr.basint = v[1];
    plr.intel = v[1];
    plr.baswil = v[2];
    plr.wil = v[2];
    plr.basagi = v[3];
    plr.agi = v[3];
    plr.bashea = v[4];
    plr.hea = v[4];
    plr.bascha = v[5];
    plr.cha = v[5];
}

/// The editor finished, for an existing character, the first name locked: `TRAIN STATS`
/// or a stat reset. `ningame` is the count of players in the realm, which does not
/// include this one while the editor is open.
///
/// With `save`: the stats are charged as for a new character; a last name that is not all
/// letters still wipes the record (the editor never lets one through); a blank one is
/// kept; the looks are stored and `show_status` prints the new stats; then the secondary
/// stats, `save_player` and, with no suicide password yet, the `SET SUICIDE` notice,
/// which the original prints for an existing character too. The game-full check sends the
/// player to the menu; otherwise the stat-reset mark is cleared and the realm hears
/// `%s just entered the Realm.`.
///
/// Without `save`: a character whose stats were not reset re-enters the realm unchanged,
/// unless the game is full; one whose stats were reset goes to the menu. A player left in
/// the realm gets the prompt.
///
/// Not carried: the paying-account branch (unpaid accounts are deleted outright), the
/// last-played day (the next save at the menu or logoff carries the day), the polling
/// start (the cycles run for every session in the realm) and the user list rebuild
/// (T-027).
fn existing_done(
    world: &mut World,
    id: SessionId,
    editor: &StatEditor,
    save: bool,
    ningame: usize,
) {
    let full = usize::from(world.max_in_game) <= ningame;
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    // back to line mode, in the game state
    s.state = SessionState::Playing;
    let statreset = plr.status2.contains(PlayerStatus2::STATRESET);
    let validated = plr.settings.contains(PlayerSettings::VALIDATED);
    let name = plr.firstname.clone();
    if save {
        apply_stats(plr, editor);
        let lastname: String = editor.family_name.trim().chars().take(18).collect();
        if !lastname.chars().all(|c| c.is_ascii_alphabetic()) {
            clear_and_menu(world, id, Msg::LastNameInvalidStartOver);
            return;
        }
        plr.lastname = lastname;
        plr.haircol = editor.haircol;
        plr.hairlen = editor.hairlen;
        plr.eyecol = editor.eyecol;
        plr.firstname = capitalise(&plr.firstname);
        crate::commands::info::show_status(world, id);
        // the secondary stats, in mode 2 when `exp` is above 0
        let Some(s) = world.session_mut(id) else {
            return;
        };
        let Some(mut c) = s.character.take() else {
            return;
        };
        let mode = if c.exp > 0 {
            crate::player::CalcMode::KeepCurrent
        } else {
            crate::player::CalcMode::Refill
        };
        crate::player::recalculate(world, &mut c, mode);
        let no_suicpw = c.suicpw.is_empty();
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
        crate::player::save_at(world, id, crate::player::SavePoint::EnterRealm);
        if no_suicpw {
            tell(world, id, Msg::SuicidePwNotice);
        }
        if full {
            game_full_menu(world, id);
        } else if validated {
            if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                plr.status2.remove(PlayerStatus2::STATRESET);
            }
            announce_entry(world, id, name);
        }
    } else if !statreset {
        if full {
            game_full_menu(world, id);
        } else if validated {
            announce_entry(world, id, name);
        }
    } else {
        // the stats were reset and are still to be spent
        back_to_menu(world, id);
    }
    if let Some(s) = world.session_mut(id)
        && validated
        && s.state != SessionState::MainMenu
    {
        s.push_prompt();
    }
}

/// The game is full: `We're sorry, but the game is currently full.`, the bare
/// `[MAJORMUD]:` prompt, and the menu state.
fn game_full_menu(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::GameFull);
        s.output.push(Msg::MudMenuPromptNormal);
        s.state = SessionState::MainMenu;
    }
}

/// `"%s just entered the Realm."` to the game, filtered by `PF_NOENTRYMSG`, with the
/// listeners' prompts. The BBS invisible test is not carried.
fn announce_entry(world: &mut World, id: SessionId, who: String) {
    crate::tell::tell_game(
        world,
        Some(id),
        ENTRY_MASK,
        true,
        None,
        &Msg::PlayerEntered { who },
    );
}

/// The name with its first letter in upper case.
pub(crate) fn capitalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars();
    if let Some(c) = chars.next() {
        out.push(c.to_ascii_uppercase());
    }
    out.extend(chars);
    out
}

/// A message sent to the session itself.
pub(crate) fn tell(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// The mask `tell_game` takes for the entrance line (`PF_NOENTRYMSG`).
pub(crate) const ENTRY_MASK: PlayerFlags = PlayerFlags::NOENTRYMSG;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_int_reads_the_leading_number() {
        assert_eq!(leading_int("11"), 11);
        assert_eq!(leading_int("2abc"), 2);
        assert_eq!(leading_int("abc"), 0);
        assert_eq!(leading_int("-3"), -3);
        assert_eq!(leading_int(""), 0);
    }

    #[test]
    fn words_split_at_blanks() {
        assert_eq!(words("  y  n "), ["y", "n"]);
        assert!(words("   ").is_empty());
    }

    #[test]
    fn capitalise_only_the_first_letter() {
        assert_eq!(capitalise("kaSimir"), "KaSimir");
        assert_eq!(capitalise(""), "");
    }
}
