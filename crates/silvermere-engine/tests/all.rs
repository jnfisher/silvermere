//! Every integration test of silvermere-engine is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

// Some files keep their tests in an inner module named like the file (for a `gang::` filter),
// which as a module here is `gangs::gang`, or `sysop::sysop` where the names match.
#![allow(clippy::module_inception)]

mod common;

// The testkit pieces m2_fight uses, by path: the engine may not name silvermere-testkit, not
// even as a dev-dependency (tests/deps.rs), and no build edge is added here. At the crate root,
// so their `crate::` paths hold; `captures.rs` decodes with `silvermere_net::cp437`, a plain
// table, which this crate provides under that name.
extern crate self as silvermere_net;
#[path = "../../silvermere-testkit/src/captures.rs"]
#[allow(dead_code)]
mod captures;
#[path = "../../silvermere-testkit/src/clock.rs"]
#[allow(dead_code)]
mod clock;
#[path = "../../silvermere-net/src/cp437.rs"]
#[allow(dead_code)]
mod cp437;
#[path = "../../silvermere-testkit/src/scripts/m2.rs"]
#[allow(dead_code)]
mod m2;
#[path = "../../silvermere-testkit/src/normalise.rs"]
#[allow(dead_code)]
mod normalise;
#[path = "../../silvermere-testkit/src/patterns.rs"]
#[allow(dead_code)]
mod patterns;
#[path = "../../silvermere-testkit/src/stats.rs"]
#[allow(dead_code)]
mod stats;

mod admin;
mod autocombat;
mod bank;
mod casting;
mod character_creation;
mod cleanup;
mod combat_flow;
mod comms;
mod cycles;
mod deps;
mod dismiss_hunters;
mod evil;
mod exit_types;
mod fullscreen_editor;
mod gangs;
mod groups;
mod info_commands;
mod inventory;
mod lifecycle;
mod login_flow;
mod m2_fight;
mod monster_casting;
mod monster_combat;
mod movement;
mod output_order;
mod panic;
mod parse_command;
mod pending_commands;
mod player_record;
mod progression;
mod room_display;
mod room_spells;
mod scheduler;
mod scripts;
mod session_ids;
mod settings;
mod shutdown;
mod spawning;
mod startup_passes;
mod stealth_commands;
mod suicide;
mod sysop;
mod tell;
mod tick_order;
mod train_stats;
mod world_load;
// The stock-download helper, declared once for every test module here (`crate::stock`).
#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock;
