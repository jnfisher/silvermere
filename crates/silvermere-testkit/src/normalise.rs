//! Event-sequence normalisation and comparison (layer 4 of the test plan).
//!
//! The random fields of a capture (experience and damage numbers, coin counts, the build
//! stamp, `N game minutes left`, user counts) are masked **by pattern field**: the event
//! keeps its name and its other fields, so a wording change still shows up in a diff.
//! The mask list is data (`tests/fixtures/masks.json`) so the Python harness reads the
//! same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::patterns::Event;

/// What a masked field's value becomes.
pub const MASKED: &str = "*";

/// Which fields to mask.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MaskSpec {
    /// Event name -> fields to mask in it; `"*"` masks every field of that event.
    #[serde(default)]
    pub events: BTreeMap<String, Vec<String>>,
    /// Fields masked in every event (the damage numbers of any family).
    #[serde(default)]
    pub fields: Vec<String>,
    /// Event -> fields masked only when a script asks for it (`Lives/CP`); see
    /// [`MaskSpec::with_script_events`].
    #[serde(default)]
    pub script_events: BTreeMap<String, Vec<String>>,
}

/// Errors loading the mask file.
#[derive(Debug, thiserror::Error)]
pub enum MaskError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing masks.json: {0}")]
    Json(#[from] serde_json::Error),
}

impl MaskSpec {
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/masks.json")
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, MaskError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| MaskError::Io {
            path: path.to_owned(),
            source,
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn load_default() -> Result<Self, MaskError> {
        Self::load(Self::default_path())
    }

    /// Adds the `script_events` masks (a script that says `Lives/CP` are random).
    pub fn with_script_events(mut self) -> Self {
        for (name, fields) in std::mem::take(&mut self.script_events) {
            self.events.entry(name).or_default().extend(fields);
        }
        self
    }

    fn masks(&self, event: &str, field: &str) -> bool {
        self.fields.iter().any(|f| f == field)
            || self
                .events
                .get(event)
                .is_some_and(|fs| fs.iter().any(|f| f == "*" || f == field))
    }
}

/// Masks the random fields of `events`: a masked field keeps its key, its value becomes
/// [`MASKED`], and its name is recorded in `Event::masked`.
pub fn normalise(events: &[Event], masks: &MaskSpec) -> Vec<Event> {
    events
        .iter()
        .map(|e| {
            let mut e = e.clone();
            for (field, value) in &mut e.fields {
                if masks.masks(&e.name, field) {
                    *value = MASKED.to_owned();
                    e.masked.insert(field.clone());
                }
            }
            e
        })
        .collect()
}

/// How two event sequences differ at one position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// The events have different names.
    Name {
        index: usize,
        left: String,
        right: String,
    },
    /// Same event, an unmasked field differs (`None` = field absent).
    Field {
        index: usize,
        event: String,
        field: String,
        left: Option<String>,
        right: Option<String>,
    },
    /// One sequence ends first.
    Missing {
        index: usize,
        /// Which side has the extra event: `"left"` or `"right"`.
        extra_in: &'static str,
        event: String,
    },
}

/// Compares two normalised sequences pairwise: names equal, and every unmasked field
/// equal. Empty result = equal.
pub fn diff_events(a: &[Event], b: &[Event]) -> Vec<Difference> {
    let mut out = Vec::new();
    for index in 0..a.len().max(b.len()) {
        match (a.get(index), b.get(index)) {
            (Some(l), Some(r)) => {
                if l.name != r.name {
                    out.push(Difference::Name {
                        index,
                        left: l.name.clone(),
                        right: r.name.clone(),
                    });
                    continue;
                }
                let keys: std::collections::BTreeSet<&String> =
                    l.fields.keys().chain(r.fields.keys()).collect();
                for key in keys {
                    let masked = l.masked.contains(key) && r.masked.contains(key);
                    let (lv, rv) = (l.fields.get(key), r.fields.get(key));
                    if !masked && lv != rv {
                        out.push(Difference::Field {
                            index,
                            event: l.name.clone(),
                            field: key.clone(),
                            left: lv.cloned(),
                            right: rv.cloned(),
                        });
                    }
                }
            }
            (Some(l), None) => out.push(Difference::Missing {
                index,
                extra_in: "left",
                event: l.name.clone(),
            }),
            (None, Some(r)) => out.push(Difference::Missing {
                index,
                extra_in: "right",
                event: r.name.clone(),
            }),
            (None, None) => unreachable!("index is below one of the lengths"),
        }
    }
    out
}
