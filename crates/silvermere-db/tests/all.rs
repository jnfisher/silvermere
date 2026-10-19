//! Every integration test of silvermere-db is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod common;

mod backup;
mod connectivity;
mod events;
mod guard;
mod keys;
mod migrate;
mod persist;
mod persist_batch;
mod persist_latency;
mod schema_roundtrip;
