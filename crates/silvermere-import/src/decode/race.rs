//! WCCRACE races: the `race` record. Not carried: `modified` (cache byte)
//! and the `unk_*` members (`unk_0x05C` stays on the flat record for T-009).

use silvermere_types::ids::RaceNo;
use silvermere_types::records::{Race, layout};

use super::{Raw, Text, abilities, record};

record! {
    /// The `race` record, WCCRACE: every member, in record order.
    pub struct RaceRecord = layout::RACE {
        number: u16,
        name: Text<30>,
        minint: i16,
        minwil: i16,
        minstr: i16,
        minhea: i16,
        minagi: i16,
        mincha: i16,
        hpbonus: i16,
        maxenergy: i16,
        restored: i16,
        abilities: [i16; 10],
        basecp: i16,
        abilvals: [i16; 10],
        unk_0x05C: Raw<6>,
        exp: i16,
        modified: u8,
        unk_0x065: u8,
        maxint: i16,
        maxwil: i16,
        maxstr: i16,
        maxhea: i16,
        maxagi: i16,
        maxcha: i16,
        unk_0x072: Raw<12>,
    }
}

/// The schema's race.
pub fn to_race(r: &RaceRecord) -> Race {
    Race {
        number: RaceNo(r.number),
        name: r.name.trimmed(),
        minint: r.minint,
        minwil: r.minwil,
        minstr: r.minstr,
        minhea: r.minhea,
        minagi: r.minagi,
        mincha: r.mincha,
        hpbonus: r.hpbonus,
        maxenergy: r.maxenergy,
        restored: r.restored,
        abilities: abilities(&r.abilities, &r.abilvals),
        basecp: r.basecp,
        exp: r.exp,
        maxint: r.maxint,
        maxwil: r.maxwil,
        maxstr: r.maxstr,
        maxhea: r.maxhea,
        maxagi: r.maxagi,
        maxcha: r.maxcha,
    }
}
