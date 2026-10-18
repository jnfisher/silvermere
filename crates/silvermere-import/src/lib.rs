//! One-time importer for legacy WCC*.DAT/.VIR/.MSG (the only crate that knows Btrieve layouts).
//!
//! Reads the legacy Btrieve data files (the virgin `.VIR` set, or a running realm's `.DAT`
//! set with `--live`), unpacks the runtime text files into the assets directory, decodes the record layouts, applies the WCCUPDAT.DAT
//! patches, converts the message files and writes the result into the normalised schema
//! through silvermere-db. It depends on silvermere-types and silvermere-db only.

#![forbid(unsafe_code)]

pub mod assets;
pub mod btrieve;
pub mod decode;
pub mod dos_time;
pub mod importer;
pub mod json;
pub mod patch;
pub mod stock;
pub mod zip;

pub use assets::{AssetCount, unpack_assets};
pub use importer::{Content, Count, ImportError, ImportOptions, Importer, Note, Report};
pub use patch::{PatchReport, apply_patches};
