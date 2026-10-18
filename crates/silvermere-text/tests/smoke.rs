//! The crate builds and links; the behaviour tests live beside this file.

use silvermere_text as _;

#[test]
fn crate_builds_and_links() {
    assert_eq!(env!("CARGO_PKG_NAME"), "silvermere-text");
}
