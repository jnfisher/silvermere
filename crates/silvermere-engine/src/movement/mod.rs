//! Movement (T-026): `move_user` and everything a step through an exit touches.
//!
//! - [`move_user`]: `move_user` split into the status refusals, the exit gate,
//!   `user_allowed_in_room`, the move and its messages; `set_user_location`,
//!   `stop_following`, the monsters' free attack and the followers.
//! - [`exits`]: the live exit parameters over the content ([`exits::ExitOverride`]),
//!   `exit_is_available`, the lock states and one function per gated exit type.
//! - [`doors`]: OPEN, CLOSE, LOCK, PICKLOCK and the door branch of BASH;
//!   `open_door_by_force`.
//! - [`timed_exits`]: `add_exit_to_list` and the reverting on the scheduler.
//! - [`search`]: SEARCH, `search_for_hidden_exits`, `clear_search_flags`.
//!
//! What other cards own is reached through [`HOOKS`], as `commands::input` does for its
//! stages: auto-combat and retaliation (T-034, T-038), room spells (T-051), monster
//! generation and movement (T-035), item charges (T-030) and the ability lookups
//! (T-032). Every default is the "nothing happens" answer, except the follower move,
//! whose default carries a following monster along with its texts so a walk already has
//! its followers.

pub mod doors;
pub mod exits;
pub mod move_user;
pub mod search;
pub mod timed_exits;

use silvermere_types::enums::{Ability, Direction};
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId};

use crate::world::World;

pub use move_user::{MoveType, move_user, move_user_with};

/// The combat state `move_user` and the door commands ask about: auto-combat, attackers,
/// retaliation and the monsters' free swing. T-034, T-036 and T-038 fill them.
pub trait CombatHooks: Sync {
    /// Whether the player is in auto-combat.
    fn in_autocombat(&self, _world: &World, _id: SessionId) -> bool {
        false
    }
    /// Whether someone in the game fights the player.
    fn being_attacked(&self, _world: &World, _id: SessionId) -> bool {
        false
    }
    /// Whether a monster in the player's room would attack him.
    fn monster_could_attack(&self, _world: &mut World, _id: SessionId) -> bool {
        false
    }
    /// Whether the player holds a retaliation timer.
    fn in_retaliation(&self, _world: &World, _id: SessionId) -> bool {
        false
    }
    /// The player's auto-combat ends.
    fn kill_autocombat(&self, _world: &mut World, _id: SessionId) {}
    /// Everyone fighting the player stops.
    fn kill_autocombat_against(&self, _world: &mut World, _id: SessionId) {}
    /// `*Combat Off*` to the player.
    fn display_autocombat_broken(&self, _world: &mut World, _id: SessionId) {}
    /// A backstab entry becomes a plain attack.
    fn clear_backstab_attack_type(&self, _world: &mut World, _id: SessionId) {}
    /// One round of the monster's attacks on the player.
    fn attack_monster_user(&self, _world: &mut World, _monster: ActiveMonsterId, _id: SessionId) {}
}

/// A room or exit casts a spell on the player. T-051.
pub trait SpellHooks: Sync {
    fn room_cast_on_user(&self, _world: &mut World, _id: SessionId, _spell: i32) {}
}

/// The monster side of a move: `generate_monster` for a room's specific monster and a
/// lair's fill, `move_monster` for a follower. T-035.
pub trait MonsterHooks: Sync {
    /// A monster made in `room` to `spec`; `true` when one was made.
    fn generate_monster(&self, _world: &mut World, _room: RoomKey, _spec: GenerateSpec) -> bool {
        false
    }
    /// The move for a monster following a player out of `from`: `true` when it moved.
    /// Default: no move.
    fn move_monster(&self, _world: &mut World, _monster: ActiveMonsterId, _dir: Direction) -> bool {
        false
    }
}

/// The arguments of a `generate_monster` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerateSpec {
    /// The monster group: 0 for the specific monster.
    pub group: i16,
    /// The known monster, or 0 for any of the group.
    pub monster: silvermere_types::ids::MonsterNo,
    /// The level range.
    pub minlevel: i16,
    pub maxlevel: i16,
}

/// One use off the item in inventory slot `slot`, the item dropped when used up. T-030.
pub trait ItemHooks: Sync {
    fn deduct_item_charge(&self, _world: &mut World, _id: SessionId, _slot: usize) {}
}

/// A player's abilities. The default reads only the record's own ability slots
/// (`player.abils`); [`CharacterTotals`] adds up every source.
pub trait AbilityHooks: Sync {
    fn has(&self, world: &World, id: SessionId, ability: Ability) -> bool {
        self.value(world, id, ability).is_some()
    }
    /// The value, or `None` when nothing gives the ability.
    fn value(&self, world: &World, id: SessionId, ability: Ability) -> Option<i32> {
        let plr = world.session(id)?.player()?;
        plr.abils
            .iter()
            .find(|(a, _)| *a == ability && *a != Ability::None)
            .map(|(_, v)| i32::from(*v))
    }
}

/// The character's full ability total: own slots, race, class, worn items and spells, as
/// the inventory display adds them up.
pub struct CharacterTotals;

impl AbilityHooks for CharacterTotals {
    fn has(&self, world: &World, id: SessionId, ability: Ability) -> bool {
        world
            .session(id)
            .and_then(|s| s.player())
            .is_some_and(|plr| crate::items::inventory::has_ability(world, plr, ability))
    }
    /// The total, or `None` when no source gives the ability.
    fn value(&self, world: &World, id: SessionId, ability: Ability) -> Option<i32> {
        let plr = world.session(id)?.player()?;
        crate::items::inventory::has_ability(world, plr, ability)
            .then(|| crate::items::inventory::ability_value(world, plr, ability))
    }
}

/// The hooks later cards implement.
pub struct Hooks {
    pub combat: &'static dyn CombatHooks,
    pub spells: &'static dyn SpellHooks,
    pub monsters: &'static dyn MonsterHooks,
    pub items: &'static dyn ItemHooks,
    pub abilities: &'static dyn AbilityHooks,
}

/// The defaults.
pub struct Defaults;

impl CombatHooks for Defaults {}
impl SpellHooks for Defaults {}
impl MonsterHooks for Defaults {}
impl ItemHooks for Defaults {}
impl AbilityHooks for Defaults {}

/// The hooks in use. A later card points a field at its own implementation.
pub static HOOKS: Hooks = Hooks {
    combat: &crate::combat::Hooks, // T-033 (attack_monster_user); T-034, T-036, T-038
    spells: &crate::spells::room::RoomCasts,
    monsters: &crate::monsters::MONSTER_HOOKS,
    items: &Defaults, // T-030
    abilities: &CharacterTotals,
};

/// `dirnames[dir]`: the live direction name, letter slot and backspace included.
pub fn dirname(world: &World, dir: Direction) -> String {
    world
        .display
        .dirnames
        .get(usize::from(dir.code()))
        .to_owned()
}

/// A direction from the number the records and commands carry (`CMD_NORTH` .. `CMD_DOWN`
/// equal the direction numbers).
pub fn direction(code: i16) -> Option<Direction> {
    u16::try_from(code)
        .ok()
        .and_then(|c| Direction::COMPASS.get(usize::from(c)).copied())
}
