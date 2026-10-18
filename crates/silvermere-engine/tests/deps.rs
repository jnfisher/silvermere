//! The crate dependency rule (`CONTRIBUTING.md`): the engine never depends on the network,
//! database, auth or API crates.

#[test]
fn engine_has_no_upward_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "silvermere-net",
        "silvermere-db",
        "silvermere-auth",
        "silvermere-api",
        "silvermere-import",
        "silvermere-testkit",
        "tokio",
        "sqlx",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "crates/silvermere-engine/Cargo.toml names {forbidden}"
        );
    }
}
