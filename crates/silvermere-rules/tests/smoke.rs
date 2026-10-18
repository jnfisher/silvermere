//! Placeholder so `cargo test -p silvermere-rules` has a target. Later cards add their
//! tests next to this file.

use silvermere_rules as _;

#[test]
fn crate_builds_and_links() {
    assert_eq!(env!("CARGO_PKG_NAME"), "silvermere-rules");
}
