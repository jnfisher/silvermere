//! WCCACTS social actions: the `action` record. Not carried: `unk_0x390`.

use silvermere_types::flags::ActionTargets;
use silvermere_types::records::{Action, layout};

use super::{Raw, Text, record};

record! {
    /// The `action` record, WCCACTS: every member, in record order.
    pub struct ActionRecord = layout::ACTION {
        name: Text<30>,
        texts: [Text<80>; 11],
        targets: i16,
        unk_0x390: Raw<98>,
    }
}

/// The schema's action.
pub fn to_action(r: &ActionRecord) -> Action {
    Action {
        name: r.name.trimmed(),
        texts: r.texts.clone().map(|t| t.trimmed()),
        targets: ActionTargets::from_bits_retain(r.targets as u16),
    }
}
