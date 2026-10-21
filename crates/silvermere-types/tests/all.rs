//! Every integration test of silvermere-types is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod facts;

mod enum_codes;
mod fixed_str;
mod names_match_facts;
mod record_sizes;
mod smoke;
mod worldgroup_layouts;
