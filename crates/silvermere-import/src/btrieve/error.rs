//! Errors raised while reading a Btrieve file.

use std::io;
use std::path::{Path, PathBuf};

/// Everything that can go wrong opening or enumerating a Btrieve file. Every variant names
/// the file; faults found on a data page also name the page.
#[derive(Debug, thiserror::Error)]
pub enum BtrieveError {
    #[error("{}: {source}", file.display())]
    Io {
        file: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}: not a Btrieve 5.x/6.x file: {reason}", file.display())]
    NotBtrieve { file: PathBuf, reason: String },
    #[error("{}: compressed records are not supported", file.display())]
    Compressed { file: PathBuf },
    #[error("{}: marked inconsistent, needs Btrieve recovery before import", file.display())]
    NeedsRecovery { file: PathBuf },
    #[error(
        "{}: variable-length records (open with OpenOptions::variable_length to read them)",
        file.display()
    )]
    VariableLength { file: PathBuf },
    #[error("{}: record length is {found}, expected {expected}", file.display())]
    RecordLengthMismatch {
        file: PathBuf,
        expected: usize,
        found: usize,
    },
    #[error("{}: page {page}: {reason}", file.display())]
    Corrupt {
        file: PathBuf,
        page: u32,
        reason: String,
    },
    #[error("{}: alternate collating sequence: {reason}", file.display())]
    InvalidAcs { file: PathBuf, reason: String },
    #[error("{}: key {key}: {reason}", file.display())]
    BadKey {
        file: PathBuf,
        key: u16,
        reason: String,
    },
}

/// A format fault found by one of the pure decoders, before the file name is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fault {
    NotBtrieve(String),
    Compressed,
    NeedsRecovery,
    VariableLength,
    Corrupt { page: u32, reason: String },
    InvalidAcs(String),
    BadKey { key: u16, reason: String },
}

impl Fault {
    pub(crate) fn not_btrieve(reason: impl Into<String>) -> Self {
        Fault::NotBtrieve(reason.into())
    }

    pub(crate) fn corrupt(page: u32, reason: impl Into<String>) -> Self {
        Fault::Corrupt {
            page,
            reason: reason.into(),
        }
    }

    pub(crate) fn at(self, file: &Path) -> BtrieveError {
        let file = file.to_path_buf();
        match self {
            Fault::NotBtrieve(reason) => BtrieveError::NotBtrieve { file, reason },
            Fault::Compressed => BtrieveError::Compressed { file },
            Fault::NeedsRecovery => BtrieveError::NeedsRecovery { file },
            Fault::VariableLength => BtrieveError::VariableLength { file },
            Fault::Corrupt { page, reason } => BtrieveError::Corrupt { file, page, reason },
            Fault::InvalidAcs(reason) => BtrieveError::InvalidAcs { file, reason },
            Fault::BadKey { key, reason } => BtrieveError::BadKey { file, key, reason },
        }
    }
}

/// Either a decoder fault or an I/O failure; the reader attaches the file name to both.
#[derive(Debug)]
pub(crate) enum ReadError {
    Fault(Fault),
    Io(io::Error),
}

impl ReadError {
    pub(crate) fn at(self, file: &Path) -> BtrieveError {
        match self {
            ReadError::Fault(fault) => fault.at(file),
            ReadError::Io(source) => BtrieveError::Io {
                file: file.to_path_buf(),
                source,
            },
        }
    }
}

impl From<Fault> for ReadError {
    fn from(fault: Fault) -> Self {
        ReadError::Fault(fault)
    }
}

impl From<io::Error> for ReadError {
    fn from(err: io::Error) -> Self {
        ReadError::Io(err)
    }
}
