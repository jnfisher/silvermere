//! What the player sees of a room (T-025): the room display, the exits line, the items
//! line, the light rules and the noise neighbours hear.
//!
//! - [`room`]: the short and long room display, descriptions read from files, the
//!   `Also here:` line, and the module globals ([`DisplayGlobals`]).
//! - [`exits`]: which exits show, the exits line and the live direction names.
//! - [`room_items`]: the items line and spotting hidden players.
//! - [`light`]: the light level and whether the player can see.
//! - [`movement_noise`]: `display_entry_movement`, the noise next door.
//! - [`bank`]: the bank balance screens (T-042).
//! - [`profile`]: `display_profile`, the PROFILE screen (T-028).
//!
//! The info commands (T-027): [`status`] (the sheet, HEALTH, EXPERIENCE), [`inventory`]
//! (INVENTORY, KEYS, WEALTH), [`who`] (the player list), [`topten`] (the top ten and the
//! top gangs), [`spells`] (SPELLS and POWERS) and [`describe`] (LOOK at a player,
//! monster, item or spell, and the WCCTEXT texts).
//!
//! [`gang`]: the gang member lists of BROADGANG (T-044).

pub mod actions;
pub mod bank;
pub mod describe;
pub mod exits;
pub mod gang;
pub mod inventory;
pub mod light;
pub mod long_text;
pub mod movement_noise;
pub mod party;
pub mod profile;
pub mod room;
pub mod room_items;
pub mod shop;
pub mod spells;
pub mod status;
// T-055
pub mod sysop;
pub mod topten;
pub mod who;

pub use exits::DirNames;
pub use light::{LightDesc, Sight, can_see};
pub use movement_noise::display_entry_movement;
pub use room::{Assets, DisplayGlobals, RoomMode, display_room};
