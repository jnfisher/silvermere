//! Every integration test of silvermere-testkit is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod common;
// the engine tests' helpers, which the stock_* suites drive the engine with
#[path = "../../silvermere-engine/tests/common/mod.rs"]
mod engine_common;
// `crate::stock` for the engine helpers above (they install the stock text)
#[allow(unused_imports)]
use silvermere_testkit::stock;
#[path = "support/stock_records.rs"]
mod stock_records;

mod client;
mod clock_rng;
mod m4_cleanup_hangup;
mod m4_items;
mod m4_scripts;
mod m4_spells;
mod m4_sysop;
mod normalise;
mod patterns;
mod raw_to_lines;
mod scripted;
mod startup_stock;
mod stock_ab_sheets;
mod stock_item_use;
mod stock_scripts;
mod stock_shops;
mod stock_social;
mod stock_spells;
