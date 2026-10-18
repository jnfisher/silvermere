//! The evil law (T-038): evil points, the attacker/victim timers, FORGIVE.
//!
//! - [`points`]: `add_evil_points`, the refusals and the charge every
//!   evil act goes through (attacking a good monster or a player, robbing, harmful
//!   spells), and `update_allowed_worn_items` for a changed legal level.
//! - [`timers`]: the `eviltimers` list ([`EvilTimer`] in [`World::evil_timers`]):
//!   `already_evil`, `should_give_evil`, `display_evil_star`, `add_evil_timer`,
//!   `remove_single_timer`, `decrement_evil_timers` (one round per slow tick, from
//!   `cycles::slow`) and `is_in_retaliation` (room entry, SYSOP GOTO).
//! - [`forgive`]: `attempt_to_forgive`; the FORGIVE command is
//!   `commands::evil`.
//!
//! The pure maths (legal levels, the cost and its scaling, the points kept on death) is
//! `silvermere_rules::evil`.
//!
//! Elsewhere: the kept points at death and their return for the next character are
//! `player::profile::save_evil_points` and `player::create` (T-023); the evil
//! reincarnation room (`evilreincroom`, 40 or more points) is `combat::kill_user` and
//! `player::suicide`; the guards' outlaw rule (`OUTLAWPTS`) is T-036's; the daily
//! forgiveness (`epcycle`, `epamount`, `maxepday`) is T-056's; the sysop's clearing of
//! the kept records and the evil timer display are T-055's.
//!
//! Jail: `jailroom` is only read at startup, which falls back to room 1 when the room
//! does not exist. Nothing in the game sends a player there (no guard, death or sysop
//! command does), so there is nothing to carry beyond the option itself.
//!
//! The timers name sessions; session ids are reused lowest first, and, as in the original,
//! a timer is not cleared when a player leaves; it counts down only while its attacker is
//! in the game.
//!
//! [`World::evil_timers`]: crate::world::World::evil_timers

pub mod forgive;
pub mod points;
pub mod timers;

pub use forgive::attempt_to_forgive;
pub use points::{add_evil_points, update_allowed_worn_items};
pub use timers::{
    EvilTimer, Give, TimerKind, add_evil_timer, already_evil, decrement_evil_timers,
    display_evil_star, is_in_retaliation, remove_single_timer, should_give_evil, shows_evil_star,
};
