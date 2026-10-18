//! The base strategies; only Windows' is here.

use std::path::{Path, PathBuf};

use crate::HomeDirError;

/// The standard folders of a platform.
pub trait BaseStrategy {
    /// The home folder.
    fn home_dir(&self) -> &Path;
    /// Configuration files.
    fn config_dir(&self) -> PathBuf;
    /// Data files.
    fn data_dir(&self) -> PathBuf;
    /// Cache files.
    fn cache_dir(&self) -> PathBuf;
    /// State files, where the platform has a folder for them.
    fn state_dir(&self) -> Option<PathBuf>;
    /// Runtime files, where the platform has a folder for them.
    fn runtime_dir(&self) -> Option<PathBuf>;
}

/// Windows' folders: `%APPDATA%` and `%LOCALAPPDATA%`, else their defaults under the home
/// folder. etcetera asks SHGetKnownFolderPath when the variables are missing; this does not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Windows {
    home_dir: PathBuf,
}

impl Windows {
    /// The strategy for the current user.
    pub fn new() -> Result<Self, HomeDirError> {
        Ok(Self {
            home_dir: crate::home_dir()?,
        })
    }

    fn var(name: &str) -> Option<PathBuf> {
        std::env::var_os(name)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
}

impl BaseStrategy for Windows {
    fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    fn config_dir(&self) -> PathBuf {
        self.data_dir()
    }

    fn data_dir(&self) -> PathBuf {
        Self::var("APPDATA").unwrap_or_else(|| self.home_dir.join("AppData").join("Roaming"))
    }

    fn cache_dir(&self) -> PathBuf {
        Self::var("LOCALAPPDATA").unwrap_or_else(|| self.home_dir.join("AppData").join("Local"))
    }

    fn state_dir(&self) -> Option<PathBuf> {
        None
    }

    fn runtime_dir(&self) -> Option<PathBuf> {
        None
    }
}
