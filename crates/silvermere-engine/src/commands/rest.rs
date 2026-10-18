//! REST and MEDITATE. The ticks that heal are the fast cycle's
//! (`cycles::fast`).
//!
//! T-024's dispatch (`commands::misc::cmd_rest`, `cmd_meditate`) calls [`cmd_rest`] and
//! [`cmd_meditate`].
//!
//! Resting and meditating end at the same moments as in the original: each command or event
//! that ends them clears resting or meditating itself, not the dispatcher: for example movement,
//! the combat commands and a monster's attack, being knocked out. REST is not refused under attack:
//! it breaks off the player's own auto-combat first.

use silvermere_rules::abilities::{Ability, AbilityLookup, CharacterAbilities};
use silvermere_text::Msg;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};

use crate::items::equip::break_off_autocombat;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// REST: a poisoned player is too sick; otherwise the room is told (unless the player was resting
/// already), hiding, sneaking and meditating end, the rest count starts again, and `You are now
/// resting.`.
pub fn cmd_rest(world: &mut World, id: SessionId) {
    // resting ends combat; only the player is told
    break_off_autocombat(world, id, false);
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if plr.poison > 0 {
        tell_user(world, id, Msg::RestSick);
        return;
    }
    let tell_others = !plr.status.contains(PlayerStatus::RESTING);
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let who = plr.firstname.clone();
    plr.status.insert(PlayerStatus::RESTING);
    plr.hidden = 0;
    plr.status.remove(PlayerStatus::SNEAKING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    s.ticks.resttick = 0;
    if tell_others {
        // tell_room sends the whole prf buffer: the dispatcher's colour reset goes to the
        // room ahead of the line, and the player's own line comes without it (crate::tell)
        tell_room(
            world,
            room,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::RestOther { who },
        );
    }
    tell_user(world, id, Msg::RestStart);
}

/// MEDITATE: only with the Meditate ability (otherwise nothing at all); a poisoned player is too
/// sick, full mana does not need it (and is clamped to the maximum); otherwise the room is told
/// (unless already meditating), hiding, sneaking and resting end, the meditate count starts again,
/// and `You are now meditating.`.
pub fn cmd_meditate(world: &mut World, id: SessionId) {
    if !has_ability(world, id, Ability::Meditate) {
        return;
    }
    // meditating ends combat; only the player is told
    break_off_autocombat(world, id, false);
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if plr.poison > 0 {
        tell_user(world, id, Msg::MeditateSick);
        return;
    }
    if i32::from(plr.mana) >= i32::from(plr.maxmana) {
        plr.mana = i16::try_from(plr.maxmana).unwrap_or(i16::MAX);
        tell_user(world, id, Msg::MeditateUseless);
        return;
    }
    let tell_others = !plr.status2.contains(PlayerStatus2::MEDITATING);
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let who = plr.firstname.clone();
    plr.status2.insert(PlayerStatus2::MEDITATING);
    plr.hidden = 0;
    plr.status.remove(PlayerStatus::SNEAKING);
    plr.status.remove(PlayerStatus::RESTING);
    s.ticks.medtick = 0;
    if tell_others {
        // tell_room sends the whole prf buffer: the dispatcher's colour reset goes to the
        // room ahead of the line, and the player's own line comes without it (crate::tell)
        tell_room(
            world,
            room,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::MeditateOther { who },
        );
    }
    tell_user(world, id, Msg::MeditateStart);
}

/// Whether the player has `ability`, from the world's tables.
pub fn has_ability(world: &World, id: SessionId, ability: Ability) -> bool {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    let spells = |n| world.spells.get(&n);
    let items = |n| world.items.get(&n);
    CharacterAbilities {
        character: plr,
        race: world.races.get(&plr.race),
        class: world.classes.get(&plr.class),
        spells: &spells,
        items: &items,
    }
    .has(ability)
}
