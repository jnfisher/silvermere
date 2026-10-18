//! etcetera 0.11's [`BaseStrategy`] and its Windows strategy, as sqlx-postgres uses them,
//! with the folders from the environment only (see Cargo.toml).

pub mod base_strategy;

pub use base_strategy::BaseStrategy;

/// The home folder: [`std::env::home_dir`].
pub fn home_dir() -> Result<std::path::PathBuf, HomeDirError> {
    std::env::home_dir().ok_or(HomeDirError)
}

/// The home folder could not be found.
#[derive(Debug)]
pub struct HomeDirError;

impl std::fmt::Display for HomeDirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not locate home directory")
    }
}

impl std::error::Error for HomeDirError {}
