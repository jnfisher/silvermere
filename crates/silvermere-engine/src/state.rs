//! The session state machine shell: `SUBST_*` as [`SessionState`] and the modules that
//! own each state's input handling.
//!
//! `session::on_line` makes one `match` arm per state and calls the function in the
//! module that owns it. Every function below returns [`Outcome::Unimplemented`] until its
//! card fills it; this file is the registration, so later cards edit only their own file:
//!
//! | Module | States | Card |
//! |---|---|---|
//! | [`login`] | `Start`, `Enter` | T-021 |
//! | [`menu`] | `MainMenu`, `Help` | T-021 |
//! | [`newchar`] | `NewChar(..)`, `RaceChange(..)` | T-022, T-047 |
//! | [`play`] | `Playing` | T-024 |
//! | [`listing`] | `Listing`, `UserReport`, `Disband` | T-027, T-055, T-044 |
//! | [`suicide`] | `Suicide(..)` | T-022, T-068 |
//! | [`sysop_menu`] | `SysopMenu(..)` | T-055 |

use silvermere_types::enums::SessionState;
use silvermere_types::ids::SessionId;

pub mod listing;
pub mod login;
pub mod menu;
pub mod newchar;
pub mod play;
pub mod suicide;
pub mod sysop_menu;

/// What a line handler tells the loop (what the host's `sttrou` returned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Handled; the session stays.
    Continue,
    /// Leave the game and close the connection.
    Logoff,
    /// The state's handler is not implemented yet: nothing is printed, a line is logged.
    Unimplemented(SessionState),
}

/// A play state: in the realm, past the menus. The states
/// at 130 and above: `Playing`, `Listing`, the three `Suicide` prompts, `UserReport`,
/// `Disband`.
pub const fn in_realm(state: SessionState) -> bool {
    state.substt() >= SessionState::Playing.substt()
}

/// The input one state handler receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input<'a> {
    /// The session the line came from.
    pub id: SessionId,
    /// The edited line (no terminator).
    pub line: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use silvermere_types::enums::{NewCharStep, SuicideStep};

    #[test]
    fn realm_states_are_130_and_above() {
        assert!(in_realm(SessionState::Playing));
        assert!(in_realm(SessionState::Listing));
        assert!(in_realm(SessionState::Suicide(SuicideStep::Confirm)));
        assert!(in_realm(SessionState::Disband));
        assert!(!in_realm(SessionState::MainMenu));
        assert!(!in_realm(SessionState::NewChar(NewCharStep::Race)));
        assert!(!in_realm(SessionState::Start));
    }
}
