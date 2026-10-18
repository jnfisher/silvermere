//! WCCGANGS gangs: the `gang` record, state. Not carried: `modified`
//! (cache byte) and the `unk_*` members.

use silvermere_types::flags::GangFlags;
use silvermere_types::records::{Gang, layout};

use super::{DecodeError, LegacyClock, Raw, Text, record};

record! {
    /// The `gang` record, WCCGANGS: every member, in record order.
    pub struct GangRecord = layout::GANG {
        key: Text<20>,
        name: Text<20>,
        exp: u32,
        leader: Text<30>,
        created: i16,
        unk_0x04C: i16,
        members: i16,
        flags: i32,
        modified: u8,
        overexp: u32,
        overflows: i16,
        unk_0x05B: Raw<162>,
    }
}

/// The schema's gang; `created` (a DOS date) in `clock`'s zone.
pub fn to_gang(r: &GangRecord, clock: &LegacyClock) -> Result<Gang, DecodeError> {
    let created = clock.at(
        "gang",
        format_args!("key {}", r.key.trimmed()),
        "created",
        r.created as u16,
        0,
    )?;
    Ok(Gang {
        key: r.key.trimmed(),
        name: r.name.trimmed(),
        exp: r.exp,
        leader: r.leader.trimmed(),
        created,
        members: r.members,
        flags: GangFlags::from_bits_retain(r.flags as u32),
        overexp: r.overexp,
        overflows: r.overflows,
    })
}
