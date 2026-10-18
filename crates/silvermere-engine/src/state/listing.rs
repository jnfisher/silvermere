//! `Listing`, `UserReport` and `Disband`: paged lists and the disband confirmation.
//!
//! T-027 fills `Listing`, the paged TOPTEN list; T-044 `Disband` (the gang member list
//! pages through `Listing` too, `display::gang`); T-055 `UserReport`, the sysop's paged
//! user report (its lines come from `state::sysop_menu::on_tick`).
//!
//! In the original a paged list comes out a line at a time, many lines a second, and a line
//! typed meanwhile stops it. Here the command shows the heading
//! and the first line and the next engine tick ([`on_tick`]) shows the rest, so a line
//! typed before that tick stops the list.

use silvermere_text::Msg;
use silvermere_types::enums::SessionState;
use silvermere_types::flags::SessionFlags;

use crate::display::topten::display_next_topten;
use crate::state::{Input, Outcome};
use crate::world::World;

/// One input line in a state this module owns.
///
/// `Listing`: a one-word line beginning with `X` stops the list (paging flag cleared, back
/// to `Playing`, `Command interrupted.` without a prompt); any other line is swallowed.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    let Some(s) = world.session_mut(input.id) else {
        return Outcome::Logoff;
    };
    match s.state {
        SessionState::Listing => {
            let mut words = input.line.split_whitespace();
            let one_x = matches!(
                (words.next(), words.next()),
                (Some(w), None) if w.starts_with(['x', 'X'])
            );
            if one_x {
                s.state = SessionState::Playing;
                s.ust.flags.remove(SessionFlags::TOPTEN);
                s.output.push(Msg::CommandInterrupted);
            }
            Outcome::Continue
        }
        // the sysop's user report (T-055)
        SessionState::UserReport => {
            let mut words = input.line.split_whitespace();
            let one_x = matches!(
                (words.next(), words.next()),
                (Some(w), None) if w.starts_with(['x', 'X'])
            );
            if one_x {
                s.state = SessionState::Playing;
                s.output.push(Msg::SysopReportInterrupted);
                world.sysop.reports.remove(&input.id);
            }
            Outcome::Continue
        }
        SessionState::Disband => {
            crate::commands::gang::on_disband_answer(world, input.id, input.line);
            Outcome::Continue
        }
        state => Outcome::Unimplemented(state),
    }
}

/// The polls since the last tick: every session reading a paged TOPTEN list gets the rest
/// of it (`display_next_topten` until the paging stops).
pub fn on_tick(world: &mut World) {
    for id in world.session_ids() {
        // at most the 100 entries `topten[]` holds, as a guard
        for _ in 0..=silvermere_types::sizes::NTOPLIST {
            let paging = world.session(id).is_some_and(|s| {
                s.state == SessionState::Listing && s.ust.flags.contains(SessionFlags::TOPTEN)
            });
            if !paging {
                break;
            }
            display_next_topten(world, id);
        }
    }
}
