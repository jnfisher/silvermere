//! The PROFILE screen, the settings `SET` changes, in the original's order and widths.
//!
//! Not carried: the `Debug:` line a sysop saw in development mode (`devmode`), which the
//! engine does not have (T-024). TODO(T-055): the sysop card brings `devmode`; add the line
//! back then.

use silvermere_text::Msg;
use silvermere_types::enums::{MessageLevel, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::SessionId;

use crate::world::World;

/// `a` when `yes`, else `b`, as the screen's `? :` pairs.
fn word(yes: bool, a: &str, b: &str) -> String {
    if yes { a } else { b }.to_string()
}

/// What the `Statusline:` row shows: On, Full, Off, or for any other mode the custom
/// template itself.
pub fn statline_word(mode: StatlineMode, template: &str) -> String {
    match mode {
        StatlineMode::On => "On".into(),
        StatlineMode::Full => "Full".into(),
        StatlineMode::Off => "Off".into(),
        _ => template.to_string(),
    }
}

/// The PROFILE screen for the session's own character, nothing without one.
pub fn display_profile(world: &mut World, id: SessionId) {
    display_profile_of(world, id, id);
}

/// The PROFILE screen for another player (SYSOP GOD user PROFILE, T-055): the screen of
/// `shown`'s character, sent to `to` as one send.
pub fn display_profile_of(world: &mut World, shown: SessionId, to: SessionId) {
    let keep_disabled = !world.config.keep_exp;
    let Some(s) = world.session(shown) else {
        return;
    };
    let Some(plr) = s.character.as_ref() else {
        return;
    };
    let st = plr.settings;
    let fl = plr.flags;
    // The halved slow-tick count since the last cleanup plus the minutes folded in
    let minutes = u64::from(plr.sesstime / 2) + u64::from(plr.lifemins);
    let mut rows = vec![
        Msg::ProfileLife { minutes },
        Msg::ProfileDisplayMode {
            mode: word(plr.verbose != 0, "Verbose", "Brief"),
        },
        Msg::ProfileStatline {
            mode: statline_word(plr.statline, &s.ust.statline),
        },
        Msg::ProfileBchannel {
            n: i32::from(plr.bchannel),
        },
        Msg::ProfileTalkSpeed {
            speed: word(plr.fasttalk != 0, "Fast", "Slow"),
        },
        Msg::ProfileFollow {
            mode: word(st.contains(PlayerSettings::BLINDFOLLOW), "Blind", "Normal"),
        },
        Msg::ProfileInterrupts {
            state: word(st.contains(PlayerSettings::ATKINTERRUPT), "On", "Off"),
        },
        Msg::ProfileReceive {
            state: word(
                st.contains(PlayerSettings::NORECEIVE),
                "Disabled",
                "Enabled",
            ),
        },
        Msg::ProfileWarnEvil {
            state: word(st.contains(PlayerSettings::EVILWARN), "Yes", "No"),
        },
        Msg::ProfileTelepaths {
            state: word(st.contains(PlayerSettings::NOTELEPATH), "No", "Yes"),
        },
        Msg::ProfileBlockEntrance {
            state: word(fl.contains(PlayerFlags::NOENTRYMSG), "Yes", "No"),
        },
        Msg::ProfileGossip {
            state: word(fl.contains(PlayerFlags::NOGOSSIP), "No", "Yes"),
            n: plr.gossmax,
        },
        Msg::ProfileAuction {
            state: word(fl.contains(PlayerFlags::NOAUCTION), "No", "Yes"),
            n: plr.auctmax,
        },
    ];
    // Gang members only
    if !plr.gang.is_empty() {
        rows.push(Msg::ProfileGangView {
            mode: word(fl.contains(PlayerFlags::GANGONLINE), "Online", "All"),
        });
    }
    rows.extend([
        Msg::ProfileKeepExp {
            state: word(st.contains(PlayerSettings::KEEPEXP), "Yes", "No"),
            note: if keep_disabled {
                "(Disabled on this system)".into()
            } else {
                String::new()
            },
        },
        Msg::ProfileMessages {
            level: match plr.msglevel {
                MessageLevel::High => "High",
                MessageLevel::Medium => "Medium",
                _ => "Low",
            }
            .into(),
        },
        Msg::ProfilePaging {
            state: word(plr.paging != 0, "Ok", "Off"),
            n: plr.pagemax,
        },
        Msg::ProfileResponses {
            mode: word(st.contains(PlayerSettings::BRIEFTALK), "Brief", "Verbose"),
        },
        Msg::ProfileStyle {
            style: word(
                st.contains(PlayerSettings::TECHNICAL),
                "Technical",
                "Fantasy",
            ),
        },
        Msg::ProfilePalette { n: plr.palette },
    ]);
    if plr.suicpw.is_empty() {
        rows.push(Msg::ProfileNoSuicide);
    }
    if s.flags.sysop {
        rows.push(Msg::ProfileSysop);
    }
    if let Some(t) = world.session_mut(to) {
        for row in rows {
            t.output.push(row);
        }
    }
}
