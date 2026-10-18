//! Items: the pack, wearing and wielding, the floor, limited copies and light sources
//! (T-030). The command handlers that use them are in [`crate::commands::inventory`].
//!
//! - [`inventory`]: add and remove, find by name, charges, weights.
//! - [`equip`]: wear, remove, ready, and the abilities an item gives while on.
//! - [`can_use`]: `user_can_use`, pure.
//! - [`room_items`]: the floor and hidden lists of a room.
//! - [`limited`]: copies in play of limited items and who holds them.
//! - [`light`]: lighting a light source.
//! - [`use_item`]: USE and its kin: scrolls, one-time casts, potions, keys, Auto Use (T-053).

pub mod can_use;
pub mod equip;
pub mod inventory;
pub mod light;
pub mod limited;
pub mod room_items;
pub mod use_item;
