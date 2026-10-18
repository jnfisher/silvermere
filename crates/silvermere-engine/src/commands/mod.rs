//! The command handlers (`cmd_*`), grouped by family so each later card owns one file.
//!
//! The dispatch table (T-024, `parse_command`) references these modules;
//! this list is registered once so no later card edits it.
//!
//! | Module | Family | Card |
//! |---|---|---|
//! | [`movement`] | directions, go, follow, drag, exits | T-026 |
//! | [`combat`] | attack, bash, smash, break, energy | T-033, T-034 |
//! | [`inventory`] | get, drop, equip, remove, inventory, use, give | T-030, T-053 |
//! | [`shop`] | buy, sell, list, appraise | T-041 |
//! | [`bank`] | deposit, withdraw, balance, loans | T-042 |
//! | [`training`] | train, stats, race change | T-039, T-047 |
//! | [`group`] | join, invite, leave, disband, follow | T-043 |
//! | [`gang`] | gang commands and the gang house | T-044 |
//! | [`comms`] | say, yell, gossip, telepath, broadcast, auction, whisper | T-045 |
//! | [`info`] | look, status, exp, who, top, spells, help | T-025, T-027 |
//! | [`settings`] | `cmd_set` | T-028 |
//! | [`social`] | actions | T-046 |
//! | [`sysop`] | `cmd_sysop` and the in-game sysop commands | T-055 |
//! | [`evil`] | forgive | T-038 |
//! | [`fullscreen_editor`] | the full-screen character editor's keys (`EDITCHA1`) | T-063 |
//! | [`misc`] | rest, meditate, search, hide, sneak, quit, everything else | T-024 onwards |
//! | [`rest`] | the REST and MEDITATE bodies `misc` calls | T-037 |
//!
//! The pipeline itself (T-024): [`parse`] (`parse_command` and the line as words) over
//! [`table`] (the command words), [`dispatch`] (`handle_commands`), [`pending`] (the
//! typed-ahead queue, the command delay, flood control) and [`input`] (`execute_input`,
//! the eleven stages a typed line goes through, and the key handler).

pub mod ask_greet;
pub mod bank;
pub mod cast;
pub mod combat;
pub mod comms;
pub mod dispatch;
pub mod evil;
pub mod fullscreen_editor;
pub mod gang;
pub mod group;
pub mod info;
pub mod input;
pub mod inventory;
pub mod misc;
pub mod movement;
pub mod parse;
pub mod pending;
pub mod rest;
pub mod settings;
pub mod shop;
pub mod social;
pub mod sysop;
pub mod table;
pub mod training;
pub mod use_item;

pub use dispatch::{Handled, dispatch};
pub use input::execute_input;
pub use parse::{CommandLine, Word, eq_ignore_case, parse_command, starts_with_ignore_case};
pub use silvermere_types::enums::CommandCode as Cmd;
