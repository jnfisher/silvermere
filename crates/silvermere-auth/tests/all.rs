//! Every integration test of silvermere-auth is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod cli;
mod lockout;
mod login_art;
mod login_dialogue;
mod password;
mod roles;
mod username;
