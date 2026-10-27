//! Telnet test client, scripted sessions, fixtures, fake clock/rng.
//!
//! Test support shared by the other crates' integration tests: a small telnet client that
//! drives an in-process server, scripted sessions, fixtures, the pattern JSON loader, and the
//! deterministic TestClock and seeded RNG. Used as a dev-dependency only.
//!
//! - [`client::TelnetClient`]: `bridge.py` in Rust (refuses options, CP437, raw capture,
//!   `until(regex)`).
//! - [`scripted::ScriptedSession`]: replays a T-005 `script.json`.
//! - [`clock::TestClock`] and [`rng::seeded_rng`], implementing `silvermere_rules::{Clock, Rng}`.
//! - [`patterns::PatternSet`]: `patterns.json` and a line classifier.
//! - [`capture_compare`]: split, mask and compare server output with a T-005 capture.
//! - [`captures`]: `raw_to_lines`, `plain` and the capture directories.
//! - [`normalise`]: mask random fields by pattern field and diff event sequences.
//! - [`scripts::m2::M2`]: the M2 fight world and an engine driver on the `TestClock`.
//! - [`stats`]: chi-square and z-tests for the A/B comparisons (`tools/differential/stats.py`).
//! - [`server::TestServer`]: an in-process server to point a client at.
//! - [`stock`]: the stock distribution, downloaded once, checked and unpacked (T-120).
//! - [`world_cache`]: the world a server reads from a template database, read once (T-147).

#![forbid(unsafe_code)]

pub mod capture_compare;
pub mod captures;
pub mod client;
pub mod clock;
pub mod normalise;
pub mod patterns;
pub mod rng;
pub mod scripted;
/// In-process scenario drivers per milestone.
pub mod scripts {
    pub mod m2;
}
pub mod server;
pub mod shared_template;
pub mod stats;
pub mod stock;
pub mod world_cache;

pub use captures::{Capture, plain, raw_to_lines};
pub use client::{Captured, TelnetClient};
pub use clock::TestClock;
pub use normalise::{Difference, MaskSpec, diff_events, normalise};
pub use patterns::{Event, PatternSet};
pub use rng::{assert_uniform, seeded_rng};
pub use scripted::{ScriptedSession, Step, Transcript};
pub use server::TestServer;
