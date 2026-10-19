//! Every integration test of silvermere-import is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

// Several files include the testkit's stock-download source as their own module, as they did as
// separate binaries, so its unit tests still run once per file.
#![allow(clippy::duplicate_mod)]

mod support {
    pub mod btrieve_v5;
    pub mod world_digest;
}

mod assets;
mod btrieve_fixtures;
mod decode_offsets;
mod equivalence;
mod import_latency;
mod invariants;
mod live_dat;
mod patch_merge;
mod post_update_equivalence;
mod smoke;
mod stock_pins;
mod stock_vir;
