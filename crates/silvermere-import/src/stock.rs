//! The stock MajorMUD 1.11p distribution: the MBBSEmu package, downloaded on demand and
//! checked against the sha256 pinned here (T-120). silvermere ships no game data;
//! `silvermere import` with no source, the first start and the tests all get the world from
//! this one file.
//!
//! [`cached`] returns the package in a cache directory, downloading it when it is missing or
//! does not match [`SHA256`]. A download goes to a temporary file in the same directory, is
//! checked, and only then renamed into place, so a partial or wrong file is never used or
//! kept. A lock file serialises processes sharing the directory (parallel tests).
//!
//! This file names no other module of its crate: silvermere-testkit includes it as its own
//! module (`#[path]`), so the URL and the sha256 live here alone.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Where the package is published.
pub const URL: &str =
    "https://download.mbbsemu.com/modules/WCCMMUD/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip";

/// The package's file name, in the cache.
pub const FILE_NAME: &str = "WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip";

/// The package's sha256, lowercase hex. Every download and every cached copy is checked.
pub const SHA256: &str = "815f7b35cdbc6c4de8deb0b686031e34905da147f360655858058a1f642d3583";

/// The package's size in bytes.
pub const SIZE: u64 = 6_673_244;

/// Overrides [`URL`] (a mirror, or a local server in tests). The sha256 is still checked.
pub const URL_ENV: &str = "SILVERMERE_STOCK_URL";

/// The URL to fetch from: [`URL_ENV`] when set, else [`URL`].
pub fn url() -> String {
    std::env::var(URL_ENV)
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| URL.to_owned())
}

/// What fetching the package can fail on.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The request failed, or the body was cut short.
    #[error("downloading {url} failed: {reason}\n{}", by_hand(dest))]
    Download {
        /// The URL.
        url: String,
        /// What went wrong.
        reason: String,
        /// Where the package belongs.
        dest: PathBuf,
    },
    /// The bytes downloaded are not the package.
    #[error(
        "{url} did not give the expected file ({got}); nothing was kept\n{}",
        by_hand(dest)
    )]
    Mismatch {
        /// The URL.
        url: String,
        /// The size and sha256 received.
        got: String,
        /// Where the package belongs.
        dest: PathBuf,
    },
    /// Reading or writing the cache directory failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
}

/// How to put the package in place without silvermere's help.
fn by_hand(dest: &Path) -> String {
    format!(
        "To fetch it by hand: download {URL} ({SIZE} bytes, sha256 {SHA256}) and save it as {}, \
         or import it with `silvermere import FILE.zip`.",
        dest.display()
    )
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> FetchError + '_ {
    move |source| FetchError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `bytes` are the package: its size and its sha256.
pub fn verify(bytes: &[u8]) -> bool {
    bytes.len() as u64 == SIZE && sha256_hex(bytes) == SHA256
}

/// Where [`cached`] found the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Already in the cache, and checked.
    Cache,
    /// Downloaded: the cache had none.
    Downloaded,
    /// Downloaded: the cached copy did not match and was removed.
    Replaced,
}

/// The package in `dir` (`dir/FILE_NAME`), checked; downloaded first when it is missing or
/// does not match, or always when `refresh` is set. Returns the path and where it came from.
pub fn cached(dir: &Path, refresh: bool) -> Result<(PathBuf, Origin), FetchError> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let lock_path = dir.join(".lock");
    let lock = std::fs::File::create(&lock_path).map_err(io(&lock_path))?;
    lock.lock().map_err(io(&lock_path))?;
    let path = dir.join(FILE_NAME);
    let origin = match std::fs::read(&path) {
        Ok(bytes) if !refresh && verify(&bytes) => return Ok((path, Origin::Cache)),
        Ok(_) => {
            std::fs::remove_file(&path).map_err(io(&path))?;
            if refresh {
                Origin::Downloaded
            } else {
                Origin::Replaced
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Origin::Downloaded,
        Err(e) => return Err(io(&path)(e)),
    };
    download(&url(), &path)?;
    Ok((path, origin))
}

/// Downloads `url` to `dest` through a temporary file beside it, which is renamed into place
/// only once its size and sha256 match the package's; on any failure it is removed.
pub fn download(url: &str, dest: &Path) -> Result<(), FetchError> {
    let tmp = dest.with_file_name(format!(".{FILE_NAME}.{}.part", std::process::id()));
    let result = fetch_into(url, dest, &tmp);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn fetch_into(url: &str, dest: &Path, tmp: &Path) -> Result<(), FetchError> {
    let failed = |reason: String| FetchError::Download {
        url: url.to_owned(),
        reason,
        dest: dest.to_path_buf(),
    };
    let response = agent().get(url).call().map_err(|e| failed(e.to_string()))?;
    let mut body = response.into_body().into_reader().take(SIZE + 1);
    let mut file = std::fs::File::create(tmp).map_err(io(tmp))?;
    let mut hasher = Sha256::new();
    let mut len = 0u64;
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = body.read(&mut buf).map_err(|e| failed(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(io(tmp))?;
        len += n as u64;
    }
    file.sync_all().map_err(io(tmp))?;
    drop(file);
    let sha = hex(&hasher.finalize());
    if len < SIZE {
        return Err(failed(format!(
            "the download stopped after {len} of {SIZE} bytes"
        )));
    }
    if len != SIZE || sha != SHA256 {
        let size = if len > SIZE {
            format!("more than {SIZE} bytes")
        } else {
            format!("{len} bytes")
        };
        return Err(FetchError::Mismatch {
            url: url.to_owned(),
            got: format!("{size}, sha256 {sha}"),
            dest: dest.to_path_buf(),
        });
    }
    std::fs::rename(tmp, dest).map_err(io(dest))
}

/// An HTTP agent: the proxy from `HTTPS_PROXY`/`NO_PROXY` (ureq's default), the webpki roots,
/// or the PEM bundle `SSL_CERT_FILE` names when it is set (a proxy with its own CA).
fn agent() -> ureq::Agent {
    use ureq::tls::{PemItem, RootCerts, TlsConfig};
    let mut tls = TlsConfig::builder();
    if let Some(pem) = std::env::var_os("SSL_CERT_FILE").and_then(|p| std::fs::read(p).ok()) {
        let certs: Vec<_> = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(PemItem::Certificate(c)) => Some(c),
                _ => None,
            })
            .collect();
        if !certs.is_empty() {
            tls = tls.root_certs(RootCerts::new_with_certs(&certs));
        }
    }
    ureq::Agent::config_builder()
        .tls_config(tls.build())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_wants_the_size_and_the_sha() {
        assert!(!verify(b""));
        assert!(!verify(&vec![0; SIZE as usize]));
        assert_eq!(SHA256.len(), 64);
        assert!(URL.ends_with(FILE_NAME));
    }

    #[test]
    fn errors_say_how_to_fetch_by_hand() {
        let e = FetchError::Mismatch {
            url: URL.into(),
            got: "1 bytes".into(),
            dest: PathBuf::from("/d/cache").join(FILE_NAME),
        };
        let s = e.to_string();
        assert!(
            s.contains(URL) && s.contains(SHA256) && s.contains("/d/cache"),
            "{s}"
        );
    }
}
