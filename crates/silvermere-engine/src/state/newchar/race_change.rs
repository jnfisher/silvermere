//! `RaceChange(..)`: the one-time race change, and the class change when
//! `ENABLE_CLASS_CHANGE` is on, from the `[MAJORMUD]:` menu.
//!
//! | State | What happens | Here |
//! |---|---|---|
//! | the menu's hidden word `CHANGERACE` | `can_user_change_race`, question 1 | [`start`] |
//! | `RaceChangeStep::ReadHelp` | YES, question 2 | [`on_line`] |
//! | `RaceChangeStep::TimeLimit` | YES, question 3 | [`on_line`] |
//! | `RaceChangeStep::NoSysop` | YES, the race list | [`on_line`] |
//! | `RaceChangeStep::Race` | `one_time_race_change`, the class list or done | [`on_race`] |
//! | `RaceChangeStep::Class` | `one_time_class_change`, done | [`on_class`] |
//!
//! The stored record is the row the session holds at the menu (the one `E` loads), and
//! the write is a save ([`SavePoint::RaceChange`], [`SavePoint::ClassChange`]). The
//! change sets `cp` to -1, so the next entry's `load_player` rolls the stats again for
//! the new race and hands back the CP (T-023), and the character goes through the stat
//! editor.

use silvermere_text::Msg;
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{RaceChangeStep, SessionState};
use silvermere_types::flags::PlayerFlags;
use silvermere_types::ids::{ClassNo, RaceNo, SessionId};
use silvermere_types::records::Character;

use super::{
    back_to_menu, display_class_list, display_race_list, help_or_list, leading_int, tell, words,
};
use crate::commands::parse::eq_ignore_case;
use crate::player::{SavePoint, save_at};
use crate::state::{Input, Outcome};
use crate::world::World;

/// The first record version that has `racechg`.
pub const RACECHG_VER: i16 = 18;

/// Whether one record may change race: `Some(minutes)` while the change is open, `None`
/// once it is not.
///
/// A record of version 18 or later that never changed may (the minutes are the full
/// `racechgdur`); one that changed may change again while no more than `racechgdur`
/// minutes of play have passed since (half of `sesstime`, plus `lifemins`, minus `racechg`,
/// unsigned), and the minutes are what is left. `PF_RACECHANGED` with a zero `racechg` (every
/// character made since) is closed for good. `sesstime` is the record's slow-tick count since the
/// last midnight cleanup, which folds it into `lifemins`.
pub fn can_user_change_race(c: &Character, sesstime: u32, cfg: &GameConfig) -> Option<i32> {
    let dur = cfg.race_change_duration;
    if c.version < RACECHG_VER {
        return None;
    }
    if !c.flags.contains(PlayerFlags::RACECHANGED) {
        return Some(dur as i32);
    }
    if c.racechg == 0 {
        return None;
    }
    // unsigned, so a racechg ahead of the play time wraps and closes
    let elapsed = (sesstime / 2)
        .wrapping_add(c.lifemins)
        .wrapping_sub(c.racechg);
    (dur >= elapsed).then(|| (dur - elapsed) as i32)
}

/// [`can_user_change_race`] for the session's held record; `None` with no character.
pub fn change_open(world: &World, id: SessionId) -> Option<i32> {
    let s = world.session(id)?;
    let c = s.character.as_ref().filter(|_| s.created)?;
    can_user_change_race(c, c.sesstime, &world.config)
}

/// The menu's `CHANGERACE`: when the change is open, question 1 and
/// [`RaceChangeStep::ReadHelp`]. Returns `false` when it is not, and the word is then the
/// menu's `C` (the first letter is all the rest of the menu reads).
pub fn start(world: &mut World, id: SessionId) -> bool {
    if change_open(world, id).is_none() {
        return false;
    }
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::ChgReadHelp);
        s.state = SessionState::RaceChange(RaceChangeStep::ReadHelp);
    }
    true
}

/// One line in a race-change state.
pub fn on_line(world: &mut World, input: Input<'_>, step: RaceChangeStep) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let yes = w.first().is_some_and(|a| eq_ignore_case(a, "yes"));
    match step {
        RaceChangeStep::Race => return on_race(world, id, &w),
        RaceChangeStep::Class => return on_class(world, id, &w),
        // anything but YES is the menu
        _ if !yes => back_to_menu(world, id),
        RaceChangeStep::ReadHelp => {
            next(world, id, Msg::ChgTimeLimit, RaceChangeStep::TimeLimit);
        }
        RaceChangeStep::TimeLimit => {
            next(world, id, Msg::ChgNoSysop, RaceChangeStep::NoSysop);
        }
        RaceChangeStep::NoSysop => {
            tell(world, id, Msg::ChooseRaceList);
            display_race_list(world, id);
            next(world, id, Msg::Chsrace, RaceChangeStep::Race);
        }
    }
    Outcome::Continue
}

fn next(world: &mut World, id: SessionId, msg: Msg, step: RaceChangeStep) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
        s.state = SessionState::RaceChange(step);
    }
}

/// `RaceChangeStep::Race`: no word, `X`, `?`/`HELP` as for a new character; a race number
/// runs [`one_time_race_change`]; then the class list (`classchg`) or the done text and
/// the menu state.
fn on_race(world: &mut World, id: SessionId, w: &[&str]) -> Outcome {
    let Some(first) = w.first() else {
        tell(world, id, Msg::MustChooseRace);
        return Outcome::Continue;
    };
    if first.to_ascii_uppercase().starts_with('X') {
        back_to_menu(world, id);
        return Outcome::Continue;
    }
    if first.starts_with('?') || first.eq_ignore_ascii_case("help") {
        help_or_list(world, id, w, display_race_list);
        return Outcome::Continue;
    }
    let race = u16::try_from(leading_int(first)).ok().map(RaceNo);
    if !race.is_some_and(|r| one_time_race_change(world, id, r)) {
        tell(world, id, Msg::MustChooseValidRace);
        return Outcome::Continue;
    }
    if world.config.enable_class_change {
        tell(world, id, Msg::ChooseClassList);
        display_class_list(world, id);
        next(world, id, Msg::Chsclass, RaceChangeStep::Class);
    } else {
        next_menu(world, id, Msg::RaceChangeDone);
    }
    Outcome::Continue
}

/// `RaceChangeStep::Class`: as the race step, for the class.
fn on_class(world: &mut World, id: SessionId, w: &[&str]) -> Outcome {
    let Some(first) = w.first() else {
        tell(world, id, Msg::MustChooseClass);
        return Outcome::Continue;
    };
    if first.to_ascii_uppercase().starts_with('X') {
        back_to_menu(world, id);
        return Outcome::Continue;
    }
    if first.starts_with('?') || first.eq_ignore_ascii_case("help") {
        help_or_list(world, id, w, display_class_list);
        return Outcome::Continue;
    }
    let class = u16::try_from(leading_int(first)).ok().map(ClassNo);
    if !class.is_some_and(|c| one_time_class_change(world, id, c)) {
        tell(world, id, Msg::MustChooseValidClassPlain);
        return Outcome::Continue;
    }
    next_menu(world, id, Msg::ClassChangeDone);
    Outcome::Continue
}

/// The done text, then `MainMenu` with no menu drawn: the player's Enter (an empty line
/// at the menu) draws it.
fn next_menu(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
        s.state = SessionState::MainMenu;
    }
}

/// The race change on the held record: the race must exist; the first change keeps the
/// minutes played in `racechg`; `PF_RACECHANGED` and `cp = -1`; the record is written.
pub fn one_time_race_change(world: &mut World, id: SessionId, race: RaceNo) -> bool {
    if !world.races.contains_key(&race) {
        return false;
    }
    let Some(s) = world.session_mut(id) else {
        return false;
    };
    let Some(c) = s.character.as_mut().filter(|_| s.created) else {
        return false;
    };
    let sesstime = c.sesstime;
    c.race = race;
    if c.racechg == 0 && !c.flags.contains(PlayerFlags::RACECHANGED) {
        c.racechg = (sesstime / 2).wrapping_add(c.lifemins);
    }
    c.flags |= PlayerFlags::RACECHANGED;
    c.cp = -1;
    save_at(world, id, SavePoint::RaceChange);
    true
}

/// The class change on the held record: the class must exist; `PF_RACECHANGED`, `cp = -1`
/// and `randomhp = 0`; `racechg` is left as it is; the record is written.
pub fn one_time_class_change(world: &mut World, id: SessionId, class: ClassNo) -> bool {
    if !world.classes.contains_key(&class) {
        return false;
    }
    let Some(s) = world.session_mut(id) else {
        return false;
    };
    let Some(c) = s.character.as_mut().filter(|_| s.created) else {
        return false;
    };
    c.class = class;
    c.flags |= PlayerFlags::RACECHANGED;
    c.cp = -1;
    c.randomhp = 0;
    save_at(world, id, SavePoint::ClassChange);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_session::clear_player;

    fn record(flags: PlayerFlags, racechg: u32, lifemins: u32) -> Character {
        let mut c = clear_player();
        c.version = 22;
        c.flags = flags;
        c.racechg = racechg;
        c.lifemins = lifemins;
        c
    }

    fn cfg(minutes: u32) -> GameConfig {
        GameConfig {
            race_change_duration: minutes,
            ..GameConfig::default()
        }
    }

    // never changed is open, for the full duration
    #[test]
    fn never_changed_is_open() {
        let c = record(PlayerFlags::empty(), 0, 5000);
        assert_eq!(can_user_change_race(&c, 0, &cfg(300)), Some(300));
    }

    // the undo window, in minutes of play (sesstime is half-minutes)
    #[test]
    fn undo_window_counts_minutes_of_play() {
        let c = record(PlayerFlags::RACECHANGED, 1000, 1000);
        assert_eq!(can_user_change_race(&c, 0, &cfg(300)), Some(300));
        assert_eq!(can_user_change_race(&c, 200, &cfg(300)), Some(200));
        let c = record(PlayerFlags::RACECHANGED, 1000, 1300);
        assert_eq!(can_user_change_race(&c, 0, &cfg(300)), Some(0));
        assert_eq!(can_user_change_race(&c, 2, &cfg(300)), None);
    }

    // changed with racechg 0 (every new character) is closed; so is an old record and a
    // racechg ahead of the play time (unsigned wrap)
    #[test]
    fn closed_cases() {
        let c = record(PlayerFlags::RACECHANGED, 0, 10);
        assert_eq!(can_user_change_race(&c, 0, &cfg(720)), None);
        let mut c = record(PlayerFlags::empty(), 0, 10);
        c.version = 17;
        assert_eq!(can_user_change_race(&c, 0, &cfg(720)), None);
        let c = record(PlayerFlags::RACECHANGED, 50, 10);
        assert_eq!(can_user_change_race(&c, 0, &cfg(720)), None);
    }
}
