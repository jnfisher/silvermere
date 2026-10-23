//! The world: rooms, players, monsters, timers, command dispatch, events.
//!
//! The engine owns all live game state in memory and runs on one dedicated world thread.
//! It receives input events and emits output events and persistence requests over
//! channels; it never imports the network or database crates, which keeps it testable
//! without sockets or a database. The design is `docs/engine.md`; the pieces are:
//!
//! - [`world`]: [`World`], loaded once from a [`WorldSnapshot`], reloaded for the admin API.
//! - [`scheduler`]: the timer queue and the game's tick periods; [`Clock`] and [`Rng`].
//! - [`cycles`]: the slow, medium and fast cycle bodies, regeneration, light, the stun.
//! - [`event_loop`]: [`Engine`], [`EngineInput`], [`EngineOutput`], `catch_unwind`, flush.
//! - [`ids`]: dense [`SessionId`] allocation from a free list.
//! - [`player_session`]: [`PlayerSession`], the typed per-session state and flags.
//! - [`output`]: [`OutputBatch`], one write per session per event.
//! - [`tell`]: [`tell_user`], [`tell_room`], [`tell_game`], [`tell_gang`] and [`tell_group`]
//!   with the original's filters.
//! - [`state`] and [`session`]: the `SUBST_*` state machine shell and the lifecycle
//!   callbacks; every state's handler is a stub a later card fills.
//! - [`commands`]: the handler module list, registered here once.
//! - [`comms`]: finding a player by name, the page allowance, the forget list (T-045).
//! - [`player`]: the player record: create, load, save, secondary stats, profiles (T-023).
//! - [`display`]: the room display (T-025).
//! - [`items`]: the pack, wearing, the floor, limited copies, light sources (T-030).
//! - [`movement`]: `move_user`, the exit gates, doors, timed reverts, search (T-026).
//! - [`gangs`]: gang records, the pending invitations, membership and gang exp (T-044).
//! - [`scripts`]: the text block script engine, room commands, exit actions (T-054).
//! - [`cleanup`]: the daily cleanup, with the players on line (T-056).

#![forbid(unsafe_code)]

pub mod cleanup;
pub mod combat;
pub mod commands;
pub mod comms;
pub mod cycles;
pub mod display;
pub mod event_loop;
pub mod evil;
pub mod gangs;
pub mod help;
pub mod ids;
pub mod items;
pub mod metrics;
pub mod monsters;
pub mod movement;
pub mod output;
pub mod party;
pub mod player;
pub mod player_session;
pub mod progression;
pub mod scheduler;
pub mod scripts;
pub mod session;
pub mod shops;
pub mod shutdown;
pub mod spells;
pub mod state;
pub mod stock_faults;
pub mod tell;
pub mod world;

pub use event_loop::{
    AdminRequest, AuditValue, ConnId, EchoMode, Engine, EngineInput, EngineOutput, GameEvent,
    PersistRequest, WhoEntry, spawn,
};
pub use ids::{ServerFull, SessionIds};
pub use output::OutputBatch;
pub use player_session::{AccountRef, Caps, PlayerSession, clear_player};
pub use scheduler::{Clock, Periods, Ran, Rng, Routine, Scheduler, SeededRng, SystemClock};
pub use shutdown::{EngineControl, ShutdownReport};
pub use silvermere_types::ids::SessionId;
pub use state::Outcome;
pub use tell::{Noise, TellBlind, tell_game, tell_gang, tell_group, tell_room, tell_user};
pub use world::{Compat, RoomState, World, WorldError, WorldSnapshot};
