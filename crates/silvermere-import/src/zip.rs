//! A PKZIP reader for the distribution zips (T-107).
//!
//! The MBBSEmu package (`WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`) and the archives inside it
//! (`WCCHSE.ZIP` and the other description files `WCCMISC.BAT` unpacks) hold only stored
//! (method 0) and deflated (method 8) members. This reads the central directory, inflates
//! each member with `miniz_oxide` and checks its CRC-32; anything else (encryption, other
//! methods, ZIP64, multi-disk archives) is refused with [`ZipError`].

use std::path::{Path, PathBuf};

/// End of central directory record.
const EOCD: u32 = 0x0605_4b50;
/// Central directory file header.
const CENTRAL: u32 = 0x0201_4b50;
/// Local file header.
const LOCAL: u32 = 0x0403_4b50;

/// What can be wrong with an archive.
#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    /// Not a zip, or a damaged one.
    #[error("{name}: not a readable zip archive: {reason}")]
    Format {
        /// The archive.
        name: String,
        /// What was wrong.
        reason: String,
    },
    /// A member this reader does not unpack.
    #[error("{name}: {member}: {reason}")]
    Unsupported {
        /// The archive.
        name: String,
        /// The member.
        member: String,
        /// Why.
        reason: String,
    },
    /// Writing an unpacked member failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file being written.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
}

/// One member of an archive.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The member's name as stored (DOS archives: upper case, no directories).
    pub name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: usize,
    size: usize,
    local_offset: usize,
}

impl Entry {
    /// Whether this is a directory entry.
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }

    /// The name without any directory part.
    pub fn file_name(&self) -> &str {
        self.name.rsplit(['/', '\\']).next().unwrap_or(&self.name)
    }
}

/// An archive in memory.
pub struct Archive<'a> {
    name: String,
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl<'a> Archive<'a> {
    /// Reads the central directory of `bytes`; `name` is used in errors.
    pub fn new(name: impl Into<String>, bytes: &'a [u8]) -> Result<Self, ZipError> {
        let name = name.into();
        let bad = |reason: &str| ZipError::Format {
            name: name.clone(),
            reason: reason.to_string(),
        };
        // The EOCD is 22 bytes plus a comment of at most 65,535.
        let low = bytes.len().saturating_sub(22 + 0xFFFF);
        let eocd = (low..=bytes.len().saturating_sub(22))
            .rev()
            .find(|&at| u32_at(bytes, at) == Some(EOCD))
            .ok_or_else(|| bad("no end of central directory record"))?;
        let field = |off: usize| u16_at(bytes, eocd + off).ok_or_else(|| bad("short EOCD"));
        if field(4)? != 0 || field(6)? != 0 {
            return Err(bad("multi-disk archives are not supported"));
        }
        let count = usize::from(field(10)?);
        let cd_offset = u32_at(bytes, eocd + 16).ok_or_else(|| bad("short EOCD"))? as usize;
        if cd_offset == 0xFFFF_FFFF {
            return Err(bad("ZIP64 archives are not supported"));
        }
        let mut entries = Vec::with_capacity(count);
        let mut at = cd_offset;
        for _ in 0..count {
            if u32_at(bytes, at) != Some(CENTRAL) {
                return Err(bad("central directory entry signature missing"));
            }
            let h = |off: usize| u16_at(bytes, at + off).ok_or_else(|| bad("short entry"));
            let w = |off: usize| u32_at(bytes, at + off).ok_or_else(|| bad("short entry"));
            let (name_len, extra_len, comment_len) = (
                usize::from(h(28)?),
                usize::from(h(30)?),
                usize::from(h(32)?),
            );
            let raw = bytes
                .get(at + 46..at + 46 + name_len)
                .ok_or_else(|| bad("entry name past the end"))?;
            entries.push(Entry {
                // CP437 names; the DOS archives' names are ASCII
                name: raw.iter().map(|&b| char::from(b)).collect(),
                flags: h(8)?,
                method: h(10)?,
                crc: w(16)?,
                compressed: w(20)? as usize,
                size: w(24)? as usize,
                local_offset: w(42)? as usize,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self {
            name,
            bytes,
            entries,
        })
    }

    /// Every member, in central-directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The member named `name`, any case (DOS names).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.file_name().eq_ignore_ascii_case(name))
    }

    /// A member's bytes, inflated and checked against its CRC-32.
    pub fn read(&self, e: &Entry) -> Result<Vec<u8>, ZipError> {
        let unsupported = |reason: String| ZipError::Unsupported {
            name: self.name.clone(),
            member: e.name.clone(),
            reason,
        };
        if e.flags & 1 != 0 {
            return Err(unsupported("encrypted".into()));
        }
        let at = e.local_offset;
        if u32_at(self.bytes, at) != Some(LOCAL) {
            return Err(unsupported("local header signature missing".into()));
        }
        let name_len = usize::from(u16_at(self.bytes, at + 26).unwrap_or(0));
        let extra_len = usize::from(u16_at(self.bytes, at + 28).unwrap_or(0));
        let start = at + 30 + name_len + extra_len;
        let data = self
            .bytes
            .get(start..start + e.compressed)
            .ok_or_else(|| unsupported("data past the end of the archive".into()))?;
        let out = match e.method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, e.size)
                .map_err(|err| unsupported(format!("inflate: {err:?}")))?,
            m => return Err(unsupported(format!("compression method {m}"))),
        };
        if out.len() != e.size {
            return Err(unsupported(format!(
                "{} bytes unpacked, the directory says {}",
                out.len(),
                e.size
            )));
        }
        if crc32(&out) != e.crc {
            return Err(unsupported("CRC-32 mismatch".into()));
        }
        Ok(out)
    }

    /// Writes every file member into `dir` (flat, by its file name), which is created.
    pub fn unpack_to(&self, dir: &Path) -> Result<usize, ZipError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| ZipError::Io { path, source }
        };
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let mut n = 0;
        for e in self.entries.iter().filter(|e| !e.is_dir()) {
            let path = dir.join(e.file_name());
            std::fs::write(&path, self.read(e)?).map_err(io(&path))?;
            n += 1;
        }
        Ok(n)
    }
}

/// CRC-32 (IEEE, reflected), as PKZIP stores it.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            (0..8).fold(i as u32, |c, _| {
                if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                }
            })
        })
    });
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn not_a_zip() {
        assert!(matches!(
            Archive::new("x", b"hello"),
            Err(ZipError::Format { .. })
        ));
    }
}
