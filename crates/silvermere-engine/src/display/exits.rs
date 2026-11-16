//! `Obvious exits:`: [`secret_visible_exit`], [`visible_exit`] and [`exits_msg`], with the
//! live direction names ([`DirNames`]).
//!
//! The ten directions differ only in their words, which [`WORDS`] keeps direction by
//! direction, the sic capitalisation included.

use std::collections::BTreeMap;

use silvermere_text::Msg;
use silvermere_types::enums::ExitType;
use silvermere_types::flags::ExitFlags;
use silvermere_types::ids::MessageNo;
use silvermere_types::names::DIRECTION_NAMES;
use silvermere_types::records::{Exit, ExitParams, Lock, Message, Room, TrapState};
use silvermere_types::sizes::NDIRS;

/// The live direction names: each name has a letter slot at byte 1 that the slow cycle
/// overwrites with a random capital (T-037), followed by a backspace, so the player reads
/// `north` but the bytes change every cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirNames {
    names: [String; NDIRS],
}

impl Default for DirNames {
    /// The table as compiled: a space in every slot.
    fn default() -> Self {
        Self {
            names: std::array::from_fn(|i| DIRECTION_NAMES[i].to_owned()),
        }
    }
}

impl DirNames {
    /// The name of direction `dir`, slot byte and backspace included.
    pub fn get(&self, dir: usize) -> &str {
        &self.names[dir]
    }

    /// Puts `letter` in the letter slot of direction `dir`. `letter` is ASCII.
    pub fn set_letter(&mut self, dir: usize, letter: char) {
        debug_assert!(letter.is_ascii());
        self.names[dir].replace_range(1..2, letter.encode_utf8(&mut [0; 4]));
    }
}

/// The words of one direction's block in `display_visible_exits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirWords {
    /// The argument of a found hidden exit's message (the `%s` of its first line).
    pub hidden_arg: &'static str,
    /// A found hidden exit with no message.
    pub secret: &'static str,
    /// A Door or Key exit, closed and open.
    pub door: [&'static str; 2],
    /// A Gate exit, closed and open.
    pub gate: [&'static str; 2],
}

/// The ten blocks of `display_visible_exits`, in `DIR_` order.
pub const WORDS: [DirWords; NDIRS] = [
    // North: "Secret passage" with a capital S (sic)
    DirWords {
        hidden_arg: "north",
        secret: "Secret passage north",
        door: ["closed door north", "open door north"],
        gate: ["closed gate north", "open gate north"],
    },
    // South
    DirWords {
        hidden_arg: "south",
        secret: "secret passage south",
        door: ["closed door south", "open door south"],
        gate: ["closed gate south", "open gate south"],
    },
    // East: "East" with a capital E (sic)
    DirWords {
        hidden_arg: "east",
        secret: "secret passage East",
        door: ["closed door east", "open door east"],
        gate: ["closed gate east", "open gate east"],
    },
    // West
    DirWords {
        hidden_arg: "west",
        secret: "secret passage west",
        door: ["closed door west", "open door west"],
        gate: ["closed gate west", "open gate west"],
    },
    // Northeast
    DirWords {
        hidden_arg: "northeast",
        secret: "secret passage northeast",
        door: ["closed door northeast", "open door northeast"],
        gate: ["closed gate northeast", "open gate northeast"],
    },
    // Northwest
    DirWords {
        hidden_arg: "northwest",
        secret: "secret passage northwest",
        door: ["closed door northwest", "open door northwest"],
        gate: ["closed gate northwest", "open gate northwest"],
    },
    // Southeast
    DirWords {
        hidden_arg: "southeast",
        secret: "secret passage southeast",
        door: ["closed door southeast", "open door southeast"],
        gate: ["closed gate southeast", "open gate southeast"],
    },
    // Southwest
    DirWords {
        hidden_arg: "southwest",
        secret: "secret passage southwest",
        door: ["closed door southwest", "open door southwest"],
        gate: ["closed gate southwest", "open gate southwest"],
    },
    // Up: doors, keys and gates are all a trap door "above"
    DirWords {
        hidden_arg: "above",
        secret: "secret passage above",
        door: ["closed trap door above", "open trap door above"],
        gate: ["closed trap door above", "open trap door above"],
    },
    // Down: the same, "below"
    DirWords {
        hidden_arg: "below",
        secret: "secret passage below",
        door: ["closed trap door below", "open trap door below"],
        gate: ["closed trap door below", "open trap door below"],
    },
];

/// A Hidden exit found by a search or visible but blocked, a Trap exit that is not hidden,
/// a Timed exit that is open.
pub fn secret_visible_exit(exit: &Exit) -> bool {
    match exit.params() {
        ExitParams::Hidden(h) => {
            h.flags.contains(ExitFlags::VISSEARCHED) || h.flags.contains(ExitFlags::VISBLOCKED)
        }
        ExitParams::Trap(t) => t.state != TrapState::Hidden,
        ExitParams::Timed(t) => !t.closed(),
        _ => false,
    }
}

/// An exit `Obvious exits:` names: a secretly visible one ([`secret_visible_exit`]), or one
/// with a target that is not a Trap, Remote Action, Hidden, Timed or Text exit.
pub fn visible_exit(exit: &Exit) -> bool {
    if secret_visible_exit(exit) {
        return true;
    }
    exit.target.0 != 0
        && !matches!(
            exit.kind,
            ExitType::Trap
                | ExitType::RemoteAction
                | ExitType::Hidden
                | ExitType::Timed
                | ExitType::Text
        )
}

/// The wording of one visible exit.
pub fn exit_word(
    dir: usize,
    exit: &Exit,
    dirnames: &DirNames,
    messages: &BTreeMap<MessageNo, Message>,
) -> String {
    let words = &WORDS[dir];
    // closed while the lock state is not open
    let open = |lock: Lock| usize::from(lock == Lock::Open);
    match exit.params() {
        ExitParams::Hidden(h) => {
            let msg = (h.name.0 != 0).then(|| messages.get(&h.name)).flatten();
            match msg {
                Some(m) => printf_s(&m.msgs[0], words.hidden_arg),
                None => words.secret.to_owned(),
            }
        }
        ExitParams::Door(d) => words.door[open(d.lock)].to_owned(),
        ExitParams::Gate(d) => words.gate[open(d.lock)].to_owned(),
        ExitParams::Key(k) => words.door[open(k.lock)].to_owned(),
        _ => dirnames.get(dir).to_owned(),
    }
}

/// The text after `Obvious exits: `, `None` when no exit is visible (`NONE!!!`).
pub fn exit_list(
    room: &Room,
    dirnames: &DirNames,
    messages: &BTreeMap<MessageNo, Message>,
) -> Option<String> {
    let words: Vec<String> = room
        .exits
        .iter()
        .enumerate()
        .filter(|(_, e)| visible_exit(e))
        .map(|(dir, e)| exit_word(dir, e, dirnames, messages))
        .collect();
    (!words.is_empty()).then(|| words.join(", "))
}

/// The visible exits as one message: the label in `COL_EXITS`, the list or `NONE!!!`,
/// wrapped at 79 columns.
pub fn exits_msg(room: &Room, dirnames: &DirNames, messages: &BTreeMap<MessageNo, Message>) -> Msg {
    match exit_list(room, dirnames, messages) {
        Some(exits) => Msg::RoomExits { exits },
        None => Msg::RoomExitsNone,
    }
}

/// A WCCMSG text printed as a format with one `%s`: the first `%s` takes `arg` and `%%` is
/// a percent sign. Other conversions are left as written.
fn printf_s(fmt: &str, arg: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + arg.len());
    let mut used = false;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                Some('s') if !used => {
                    chars.next();
                    out.push_str(arg);
                    used = true;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_slot() {
        let mut d = DirNames::default();
        assert_eq!(d.get(0), "n \u{8}orth");
        d.set_letter(0, 'J');
        assert_eq!(d.get(0), "nJ\u{8}orth");
    }

    #[test]
    fn printf_one_string() {
        assert_eq!(
            printf_s("a crack to the %s", "north"),
            "a crack to the north"
        );
        assert_eq!(printf_s("100%% %s %s", "x"), "100% x %s");
    }
}
