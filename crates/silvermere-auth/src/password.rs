//! argon2id password hashes as PHC strings.
//!
//! Replaces Worldgroup's plain-text BBS passwords (and MBBSEmu's SHA-512 + salt): there
//! is no user data to import, so there is no legacy
//! variant. Verification reads the parameters from the stored string, so hashes made
//! with [`Cost::Low`] and with the defaults both verify.

use argon2::password_hash::phc::PasswordHash;
use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use rand::Rng as _;

/// How expensive a new hash is. Always argon2id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cost {
    /// The `argon2` crate's defaults (19 MiB, 2 passes, 1 lane): what the server uses.
    #[default]
    Default,
    /// 64 KiB, 1 pass: for tests that create many accounts. Still argon2id with a random
    /// salt; never configured on a server.
    Low,
}

impl Cost {
    fn hasher(self) -> Argon2<'static> {
        match self {
            Self::Default => Argon2::default(),
            Self::Low => {
                let params = Params::new(64, 1, 1, None).expect("valid argon2 parameters");
                Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            }
        }
    }
}

/// The argon2id PHC string for `plain` with the default cost and a fresh 16-byte salt.
pub fn hash(plain: &str) -> String {
    hash_with(plain, Cost::Default)
}

/// The argon2id PHC string for `plain` at `cost`.
pub fn hash_with(plain: &str, cost: Cost) -> String {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    cost.hasher()
        .hash_password_with_salt(plain.as_bytes(), &salt)
        .expect("argon2id accepts any password and a 16-byte salt")
        .to_string()
}

/// Whether `plain` matches `hash`. The comparison of the derived keys is constant-time
/// (argon2's `Output` equality); a malformed or empty `hash` never matches.
pub fn verify(plain: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

/// A hash of a password nobody knows, verified against when a username does not exist
/// so that an unknown name costs as much time as a wrong password.
pub(crate) fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| hash("silvermere unknown-user placeholder"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_cost_is_argon2id_and_verifies() {
        let h = hash_with("pw", Cost::Low);
        assert!(h.starts_with("$argon2id$v=19$m=64,t=1,p=1$"), "{h}");
        assert!(verify("pw", &h));
        assert!(!verify("pW", &h));
    }

    #[test]
    fn garbage_hash_never_matches() {
        assert!(!verify("", ""));
        assert!(!verify("pw", "not a hash"));
    }
}

/// A password as typed, held only until it is hashed or checked. `Debug` prints
/// `Secret(***)`, so a `tracing::debug!(?dialogue)` or a failing `assert_eq!` never logs
/// it (T-013 review, silvermere#17).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a password.
    pub fn new(password: impl Into<String>) -> Self {
        Self(password.into())
    }

    /// The password, for hashing and verifying only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[cfg(test)]
mod secret_tests {
    use super::Secret;

    #[test]
    fn debug_redacts_the_password() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(s.expose(), "hunter2");
    }
}
