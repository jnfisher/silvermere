//! Every integration test of silvermere-import is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

// The testkit's stock-download helper, by path and once for the binary (`crate::stock`): the
// import crate takes no dependency on the testkit.
#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock;

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
