//! Bearer-token auth. Tokens are configured in TOML (`[[admin.tokens]] name = "editor",
//! sha256 = "<hex>", scopes = ["read", "write"]`); only the SHA-256 of a token is kept in
//! memory and the presented token's hash is compared with [`subtle::ConstantTimeEq`].
//! Argon2 would be overkill for machine tokens.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// What a token may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// GET routes: `/v1/who` (without hidden sysops) and `/v1/events`.
    Read,
    /// Routes that change the running game: `/v1/broadcast`, `/v1/reload-world`.
    #[serde(alias = "broadcast")]
    Write,
    /// The reserved world-editing routes, and everything else; `/v1/who` includes hidden
    /// sysops.
    #[serde(alias = "sysop")]
    Admin,
}

impl Scope {
    /// Every scope.
    pub const ALL: [Scope; 3] = [Scope::Read, Scope::Write, Scope::Admin];

    /// The name used in the config file and the docs.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Admin => "admin",
        }
    }

    /// The scope a config or CLI word names (`broadcast` and `sysop` are the T-018 names).
    pub fn parse(word: &str) -> Option<Scope> {
        match word.trim() {
            "read" => Some(Scope::Read),
            "write" | "broadcast" => Some(Scope::Write),
            "admin" | "sysop" => Some(Scope::Admin),
            _ => None,
        }
    }
}

/// The SHA-256 of `secret`.
pub fn sha256(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// The lower-case hex SHA-256 of `secret`, as written in `sha256 = "..."`.
pub fn sha256_hex(secret: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(secret.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Parses 64 hex digits (either case) into a hash.
pub fn parse_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// A configured token: the name for the audit trail, the hash, the scopes.
#[derive(Debug, Clone)]
pub struct Token {
    pub name: String,
    hash: [u8; 32],
    pub scopes: Vec<Scope>,
}

impl Token {
    /// A token named `name` whose secret is `secret`.
    pub fn new(name: &str, secret: &str, scopes: &[Scope]) -> Self {
        Self::from_hash(name, Sha256::digest(secret.as_bytes()).into(), scopes)
    }

    /// A token named `name` known only by the SHA-256 of its secret.
    pub fn from_hash(name: &str, hash: [u8; 32], scopes: &[Scope]) -> Self {
        Self {
            name: name.to_string(),
            hash,
            scopes: scopes.to_vec(),
        }
    }

    /// Whether the token may do something that needs `needed`; `admin` may do anything.
    pub fn allows(&self, needed: Scope) -> bool {
        scope_allows(&self.scopes, needed)
    }

    /// Whether `presented` is this token's secret (constant-time on the hash).
    pub fn matches(&self, presented: &str) -> bool {
        let hash: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
        bool::from(self.hash.ct_eq(&hash))
    }
}

fn scope_allows(scopes: &[Scope], needed: Scope) -> bool {
    scopes.contains(&Scope::Admin) || scopes.contains(&needed)
}

/// Who is calling: the matched token's name and scopes.
#[derive(Debug, Clone)]
pub struct Caller {
    pub name: String,
    pub scopes: Vec<Scope>,
}

impl Caller {
    /// See [`Token::allows`].
    pub fn allows(&self, needed: Scope) -> bool {
        scope_allows(&self.scopes, needed)
    }
}

/// The token matching the `Authorization` header value, if any. Every token is compared, so
/// the time does not depend on which one matched.
pub fn authenticate(tokens: &[Token], header: Option<&str>) -> Option<Caller> {
    let presented = header?.strip_prefix("Bearer ")?.trim();
    let hash: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
    let mut found = None;
    for t in tokens {
        if bool::from(t.hash.ct_eq(&hash)) {
            found = Some(Caller {
                name: t.name.clone(),
                scopes: t.scopes.clone(),
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_the_right_secret() {
        let tokens = [
            Token::new("ops", "s3cret", &[Scope::Read]),
            Token::new("bot", "other", &[Scope::Write]),
        ];
        assert_eq!(
            authenticate(&tokens, Some("Bearer other")).unwrap().name,
            "bot"
        );
        assert!(authenticate(&tokens, Some("Bearer nope")).is_none());
        assert!(authenticate(&tokens, Some("s3cret")).is_none());
        assert!(authenticate(&tokens, None).is_none());
    }

    #[test]
    fn admin_scope_allows_everything_and_write_does_not_imply_read() {
        let t = Token::new("a", "x", &[Scope::Admin]);
        assert!(t.allows(Scope::Read) && t.allows(Scope::Write));
        let t = Token::new("b", "y", &[Scope::Write]);
        assert!(t.allows(Scope::Write) && !t.allows(Scope::Read) && !t.allows(Scope::Admin));
    }

    #[test]
    fn hash_helpers_round_trip() {
        // sha256("abc")
        let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex("abc"), hex);
        let t = Token::from_hash("n", parse_sha256_hex(hex).unwrap(), &[Scope::Read]);
        assert!(t.matches("abc") && !t.matches("abd"));
        assert!(parse_sha256_hex(&hex[1..]).is_none());
        assert!(parse_sha256_hex(&hex.replace('b', "z")).is_none());
        assert_eq!(parse_sha256_hex(&hex.to_uppercase()), parse_sha256_hex(hex));
    }

    #[test]
    fn scope_words_include_the_t018_names() {
        assert_eq!(Scope::parse("broadcast"), Some(Scope::Write));
        assert_eq!(Scope::parse("sysop"), Some(Scope::Admin));
        assert_eq!(Scope::parse("read"), Some(Scope::Read));
        assert_eq!(Scope::parse("root"), None);
    }
}
