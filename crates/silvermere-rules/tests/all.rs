//! Every integration test of silvermere-rules is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod combat_distributions;
mod economy;
mod progression;
mod rng;
mod smoke;
