//! The stock distribution is pinned once, in `silvermere_import::stock` (T-120). install.sh runs
//! on its own, before any silvermere binary is there, so it carries the same pins; this keeps
//! them equal. (`tools/differential/fetch-stock.sh` reads them from the source.)

use silvermere_import::stock::{FILE_NAME, SHA256, URL};

#[test]
fn install_sh_pins_the_same_package() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/install.sh");
    let script = std::fs::read_to_string(path).unwrap();
    let value = |var: &str| {
        script
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{var}=")))
            .unwrap_or_else(|| panic!("install.sh sets no {var}"))
            .to_owned()
    };
    assert_eq!(value("stock_name"), FILE_NAME);
    assert_eq!(value("stock_sha"), SHA256);
    assert_eq!(
        value("stock_url").replace("$stock_name", FILE_NAME),
        format!("${{SILVERMERE_STOCK_URL:-{URL}}}")
    );
}
