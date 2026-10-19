//! Every integration test of silvermere-text is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod ansi_macros;
mod component;
mod cp437;
mod hp_colour;
mod menu;
mod msg_catalogue;
mod patterns_snapshot;
mod prompt;
mod smoke;
mod statline;
mod wrap;
