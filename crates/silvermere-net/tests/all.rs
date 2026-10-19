//! Every integration test of silvermere-net is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod line_editor;
mod session_task;
mod smoke;
mod telnet_negotiation;
