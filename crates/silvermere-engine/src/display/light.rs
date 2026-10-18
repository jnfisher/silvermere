//! Light: the room's light level and whether a player can see in it.
//!
//! [`level`] is the pure part: the sum, the cap and the description table. [`light_at`]
//! gathers its inputs from the world and [`can_see`] prints the refusal the room
//! display and LOOK show when the player cannot see.

use silvermere_rules::abilities::{Ability, AbilityLookup, CharacterAbilities};
use silvermere_text::Msg;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::SessionId;

use crate::world::World;

/// Below this nothing can be seen.
pub const LIGHT_DARK: i32 = -150;
/// Above this the light is blinding.
pub const LIGHT_BLIND: i32 = 900;
/// Ordinary light stops here; Blinding Light is added after the cap.
pub const LIGHT_CAP: i32 = 900;

/// The words for a light level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LightDesc {
    /// Below -200.
    PitchBlack,
    /// -200 up to -151.
    VeryDark,
    /// -150 up to -101.
    BarelyVisible,
    /// -100 up to -1.
    DimlyLit,
    /// 0 up to 199.
    Regular,
    /// 200 up to 899.
    Daylight,
    /// 900 and over.
    Blinding,
}

impl LightDesc {
    /// The description from the level: each bound is exclusive.
    pub fn of(light: i32) -> Self {
        match light {
            l if l < -200 => Self::PitchBlack,
            l if l < -150 => Self::VeryDark,
            l if l < -100 => Self::BarelyVisible,
            l if l < 0 => Self::DimlyLit,
            l if l < 200 => Self::Regular,
            l if l < 900 => Self::Daylight,
            _ => Self::Blinding,
        }
    }

    /// The text.
    pub fn text(self) -> &'static str {
        match self {
            Self::PitchBlack => "pitch black",
            Self::VeryDark => "very dark",
            Self::BarelyVisible => "barely visible",
            Self::DimlyLit => "dimly lit",
            Self::Regular => "Regular Light",
            Self::Daylight => "Daylight",
            Self::Blinding => "You are blind!",
        }
    }
}

/// The pure part of the light level: the room's light, the character's Alter User Light,
/// and the personal and room light of every player in the room (`present`, the character
/// included), capped at [`LIGHT_CAP`], then Blinding Light on top.
pub fn level(room: i32, user_light: i32, present: i32, blinding: i32) -> (i32, LightDesc) {
    let light = (room + user_light + present).min(LIGHT_CAP) + blinding;
    (light, LightDesc::of(light))
}

/// The light level for a session: `None` without a character or when the room is not loaded.
///
/// The character's Alter User Light and Blinding Light come from one ability lookup over
/// every source (T-030). A lit light source counts through `player.light`, which is summed
/// with the others here.
pub fn light_at(world: &World, id: SessionId) -> Option<(i32, LightDesc)> {
    let plr = world.session(id)?.player()?;
    let key = silvermere_types::ids::RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let room = world.room(key)?;
    // Every player in the game in the same room, this one included.
    let present: i32 = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .filter_map(|i| world.session(i).and_then(|s| s.player()))
        .filter(|p| p.room == key.room && p.map == key.map)
        .map(|p| i32::from(p.light) + i32::from(p.roomlight))
        .sum();
    let (user_light, blinding) = {
        let spells = |n| world.spells.get(&n);
        let items = |n| world.items.get(&n);
        CharacterAbilities {
            character: plr,
            race: world.races.get(&plr.race),
            class: world.classes.get(&plr.class),
            spells: &spells,
            items: &items,
        }
        .pair(Ability::UserLight, Ability::BlindingLight)
    };
    Some(level(i32::from(room.light), user_light, present, blinding))
}

/// What `can_see` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    /// The player sees; the light level and its words, for the room's light line.
    Sees(i32, LightDesc),
    /// `ST_BLIND`: `You are blind.`
    Blind,
    /// Below [`LIGHT_DARK`].
    Dark(LightDesc),
    /// Above [`LIGHT_BLIND`].
    Blinded(LightDesc),
}

/// The decision of [`can_see`] without the message. A session with no character sees, at
/// light 0.
pub fn sight(world: &World, id: SessionId) -> Sight {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return Sight::Sees(0, LightDesc::Regular);
    };
    if plr.status.contains(PlayerStatus::BLIND) {
        return Sight::Blind;
    }
    let Some((light, desc)) = light_at(world, id) else {
        return Sight::Sees(0, LightDesc::Regular);
    };
    if light < LIGHT_DARK {
        Sight::Dark(desc)
    } else if light > LIGHT_BLIND {
        Sight::Blinded(desc)
    } else {
        Sight::Sees(light, desc)
    }
}

/// On a refusal the reason goes to the player and `None` comes back; otherwise the light
/// level and its words.
pub fn can_see(world: &mut World, id: SessionId) -> Option<(i32, LightDesc)> {
    let msg = match sight(world, id) {
        Sight::Sees(light, desc) => return Some((light, desc)),
        Sight::Blind => Msg::Blind,
        Sight::Dark(desc) => Msg::RoomDark {
            light: desc.text().into(),
        },
        Sight::Blinded(desc) => Msg::LightBlinding {
            light: desc.text().into(),
        },
    };
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_at_the_bounds() {
        let table = [
            (-201, LightDesc::PitchBlack),
            (-200, LightDesc::VeryDark),
            (-151, LightDesc::VeryDark),
            (-150, LightDesc::BarelyVisible),
            (-101, LightDesc::BarelyVisible),
            (-100, LightDesc::DimlyLit),
            (-1, LightDesc::DimlyLit),
            (0, LightDesc::Regular),
            (199, LightDesc::Regular),
            (200, LightDesc::Daylight),
            (899, LightDesc::Daylight),
            (900, LightDesc::Blinding),
        ];
        for (light, want) in table {
            assert_eq!(LightDesc::of(light), want, "{light}");
        }
    }

    // The cap, then Blinding Light
    #[test]
    fn cap_then_blinding() {
        assert_eq!(level(800, 0, 300, 0), (900, LightDesc::Blinding));
        assert_eq!(level(800, 0, 300, 50), (950, LightDesc::Blinding));
        assert_eq!(level(-175, 0, 100, 0), (-75, LightDesc::DimlyLit));
        assert_eq!(level(-175, 0, 0, 0), (-175, LightDesc::VeryDark));
    }
}
