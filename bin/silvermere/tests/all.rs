//! Every integration test of silvermere is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below. `metrics_recorder.rs` is a binary of its own (Cargo.toml).

mod common;

mod account_delete;
mod admin_api;
mod admin_complete;
mod backup_cli;
mod cleanup_store;
mod cli;
mod docs_in_sync;
mod first_start;
mod inventory_persist;
mod login_e2e;
mod login_timeout;
mod m1_walk;
mod m3_bank;
mod m3_comms;
mod m3_party;
mod m3_shops;
mod m3_social_training;
mod metrics_endpoint;
mod name_check_latency;
mod queued_characters;
mod serve_in_process;
mod shutdown;
mod shutdown_barrier;
mod single_writer;
mod startup_coalescing;
mod world_load_db;
