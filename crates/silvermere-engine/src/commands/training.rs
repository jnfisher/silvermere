//! Training and stats: train, the stat editor, the race and class change.
//!
//! TRAIN alone is [`cmd_train`] over `progression::train_level` (T-039); TRAIN STATS
//! ([`train_stats`], T-047) opens the line-mode stat editor that character creation uses
//! (`state::newchar::edit_character_stats`).

use silvermere_text::Msg;
use silvermere_types::enums::{Ability, RoomType, ShopType};
use silvermere_types::flags::{PlayerFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};

use super::dispatch::{Handled, arg_is};
use super::rest::has_ability;
use super::{Cmd, CommandLine};
use crate::player::{SavePoint, save_at};
use crate::progression::train_level;
use crate::state::newchar::edit_character_stats;
use crate::tell::{tell_game, tell_user};
use crate::world::World;

/// The abilities that mean a stat is changed by magic just now.
const STAT_ALTERING: [Ability; 6] = [
    Ability::Intellect,
    Ability::Wisdom,
    Ability::Strength,
    Ability::Health,
    Ability::Agility,
    Ability::Charm,
];

/// TRAIN: refused in tournament mode; resting and meditation end; `TRAIN` alone trains a
/// level; `TRAIN STATS` is [`train_stats`]; anything else after TRAIN is not the command.
pub fn cmd_train(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let tournament = world.config.tournament_play;
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    if tournament {
        tell_user(world, id, Msg::TrainTournament);
        return Handled::Done;
    }
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    if line.argc() == 2 && arg_is(line, 1, "stats") {
        return train_stats(world, id);
    }
    if line.argc() == 1 {
        train_level(world, id);
        return Handled::Done;
    }
    Handled::NotCommand
}

/// The `TRAIN STATS` branch of [`cmd_train`]. Refused, in the order the original refuses:
/// not a shop room; a stat-altering ability on the character; not a training shop or a trainer of
/// another class (`trainclass` 0 teaches every class). Tournament mode was refused before, in
/// [`cmd_train`].
///
/// Otherwise the character is saved, the realm hears `%s just left the Realm.` (the BBS
/// invisibility test not carried) and the editor opens without the first-name field. The
/// editor state is below `SUBST_PLAYING`, so the player is out of the realm until the editor's end
/// brings them back (`state::newchar::editor_done`). No prompt follows: the editor's first field is
/// the prompt.
pub fn train_stats(world: &mut World, id: SessionId) -> Handled {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return Handled::NotCommand;
    };
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let (class, name) = (plr.class, plr.firstname.clone());
    let shopnum = match world.room(key) {
        Some(rm) if rm.room_type == RoomType::Shop => rm.shopnum,
        _ => {
            tell_user(world, id, Msg::TrainStatsNotHere);
            return Handled::Done;
        }
    };
    if STAT_ALTERING.iter().any(|&a| has_ability(world, id, a)) {
        tell_user(world, id, Msg::TrainStatsAltered);
        return Handled::Done;
    }
    let teaches = world.shops.get(&shopnum).is_some_and(|shp| {
        shp.shop_type == ShopType::Training && (shp.trainclass.0 == 0 || shp.trainclass == class)
    });
    if !teaches {
        tell_user(world, id, Msg::TrainStatsNotHere);
        return Handled::Done;
    }
    save_at(world, id, SavePoint::TrainStats);
    tell_game(
        world,
        Some(id),
        PlayerFlags::NOENTRYMSG,
        true,
        None,
        &Msg::PlayerLeftRealm { who: name },
    );
    edit_character_stats(world, id, false);
    Handled::Wait
}
