//! Every integration test of silvermere-fsd is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod ansi_entry;
mod answers;
mod display;
mod line_entry;
mod properties;
mod support;
mod template;
mod verify;
