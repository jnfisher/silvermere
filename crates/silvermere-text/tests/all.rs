//! Every integration test of silvermere-text is a module of this one test binary (T-129), so the
//! crate's dependency graph is linked once rather than once per file. A new test file
//! goes in the list below.

mod ansi_macros;
mod component;
mod cp437;
mod hp_colour;
mod menu;
mod msg_catalogue;
mod package_text;
mod patterns_snapshot;
mod prompt;
mod smoke;
mod statline;
mod wrap;

// The stock-download helper, declared once for every test module here (`crate::stock`).
#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock;

/// The game's long text from the stock package, installed for this process.
pub fn package() -> &'static silvermere_text::package::PackageText {
    silvermere_text::package::load_and_install(&stock::assets_dir())
        .unwrap_or_else(|e| panic!("the stock package's text: {e}"))
}
