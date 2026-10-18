//! Rebuilds this crate when a migration is added or changed: `sqlx::migrate!` embeds the
//! files at compile time and does not see a new one on its own (T-109 added the first
//! migration after 0001).

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
