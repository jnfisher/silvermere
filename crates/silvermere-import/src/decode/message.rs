//! WCCMSG messages: the `mudmsg` record. Not carried: `modified` (cache
//! byte), `unk_0x0FB`.

use silvermere_types::ids::MessageNo;
use silvermere_types::records::{Message, layout};

use super::{Raw, Text, record};

record! {
    /// The `mudmsg` record, WCCMSG: every member, in record order.
    pub struct MudmsgRecord = layout::MUDMSG {
        number: i32,
        msgs: [Text<82>; 3],
        modified: u8,
        unk_0x0FB: Raw<2>,
    }
}

/// The schema's message.
pub fn to_message(r: &MudmsgRecord) -> Message {
    Message {
        number: MessageNo(r.number),
        msgs: r.msgs.clone().map(|t| t.trimmed()),
    }
}
