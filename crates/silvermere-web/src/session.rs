//! Who is signed in. A session is a signed cookie, so it outlives a restart of the server:
//! the cookie holds the sysop's name, when the session began, when it was last used, when
//! the password was last given, a fingerprint of the account's password, and a random
//! name, all under an HMAC-SHA256 with the server's key ([`Sessions::with_key`]; `bin`
//! keeps it in the data directory).
//!
//! A session ends after [`LIFETIME`], after [`IDLE`] without a request, on sign-out, and as
//! soon as the account is no longer a sysop or its password changes (the fingerprint no
//! longer matches what [`SysopLogin::fingerprint`] answers). Actions that cannot be undone
//! want the password given within [`FRESH`] ([`Session::fresh`]).
//!
//! Sign-out also remembers the session's name until its cookie would have expired, so a
//! copy of the cookie is refused too (in memory: a restart forgets it, and the cookie's own
//! limits still hold).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng as _;
use sha2::{Digest, Sha256};

/// The cookie that names the session.
pub const COOKIE: &str = "silvermere_session";
/// How long a session lasts at most.
pub const LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
/// How long a session lasts without a request.
pub const IDLE: Duration = Duration::from_secs(30 * 60);
/// How long the password, once given, covers the actions that want it.
pub const FRESH: Duration = Duration::from_secs(10 * 60);

/// What the binary answers a sign-in with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// A sysop: the account's name as stored.
    Sysop(String),
    /// Not signed in, and what to tell the person.
    Refused(String),
}

/// The sign-in check: the binary reads the account and verifies the password.
pub trait SysopLogin: Send + Sync + 'static {
    fn login(
        &self,
        username: String,
        password: String,
    ) -> Pin<Box<dyn Future<Output = LoginOutcome> + Send + '_>>;

    /// A short fingerprint of the account's password, while it is a sysop; `Ok(None)` when it
    /// is gone or no longer a sysop. A session made under another fingerprint has ended.
    /// `Err` when the accounts cannot be read just now (the server restarting): the session
    /// stands.
    fn fingerprint(
        &self,
        username: String,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ()>> + Send + '_>>;
}

/// A signed-in sysop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub user: String,
    /// The token every form and console call of this session carries; one without it is
    /// refused.
    pub csrf: String,
    /// The account's password fingerprint when the session was made.
    pub fingerprint: String,
    /// Unix seconds: when the session began, when it was last used, when the password was
    /// last given.
    pub issued: u64,
    pub seen: u64,
    pub auth_at: u64,
    /// The random name.
    nonce: String,
}

impl Session {
    /// The session as last used at `now`.
    pub fn used_at(self, now: u64) -> Self {
        Self { seen: now, ..self }
    }

    /// The session with the password given again at `now`.
    pub fn reauthenticated(self, now: u64) -> Self {
        Self {
            seen: now,
            auth_at: now,
            ..self
        }
    }

    /// Whether the password was given within [`FRESH`] of `now` (Unix seconds).
    pub fn fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.auth_at) <= FRESH.as_secs()
    }
}

/// The current time, Unix seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Makes and checks session cookies.
pub struct Sessions {
    key: [u8; 32],
    /// Signed-out session names and when their cookies would have expired.
    revoked: Mutex<HashMap<String, u64>>,
}

impl Default for Sessions {
    /// A key of its own: sessions end with the process.
    fn default() -> Self {
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        Self::with_key(key)
    }
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions").finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// HMAC-SHA256 (RFC 2104) of `msg` under `key`.
pub fn hmac(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    block[..32].copy_from_slice(key);
    let pad = |b: u8| block.map(|k| k ^ b);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// Equal without a timing difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

impl Sessions {
    /// Sessions signed with `key`: they survive a restart that keeps it.
    pub fn with_key(key: [u8; 32]) -> Self {
        Self {
            key,
            revoked: Mutex::new(HashMap::new()),
        }
    }

    fn revoked(&self) -> std::sync::MutexGuard<'_, HashMap<String, u64>> {
        self.revoked.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn csrf(&self, nonce: &str) -> String {
        hex(&hmac(&self.key, format!("csrf|{nonce}").as_bytes()))
    }

    /// A new session for `user`, whose password has just been given at `now`.
    pub fn open(&self, user: &str, fingerprint: &str, now: u64) -> Session {
        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let nonce = hex(&nonce);
        Session {
            user: user.to_owned(),
            csrf: self.csrf(&nonce),
            fingerprint: fingerprint.to_owned(),
            issued: now,
            seen: now,
            auth_at: now,
            nonce,
        }
    }

    /// The cookie value of `s`.
    pub fn cookie_value(&self, s: &Session) -> String {
        let body = format!(
            "v1.{}.{}.{}.{}.{}.{}",
            hex(s.user.as_bytes()),
            hex(s.fingerprint.as_bytes()),
            s.issued,
            s.seen,
            s.auth_at,
            s.nonce
        );
        let mac = hex(&hmac(&self.key, body.as_bytes()));
        format!("{body}.{mac}")
    }

    /// The session a cookie value names, if it is genuine and has not ended by `now`.
    pub fn read(&self, value: &str, now: u64) -> Option<Session> {
        let (body, mac) = value.rsplit_once('.')?;
        if !same(&unhex(mac)?, &hmac(&self.key, body.as_bytes())) {
            return None;
        }
        let mut parts = body.split('.');
        if parts.next()? != "v1" {
            return None;
        }
        let user = String::from_utf8(unhex(parts.next()?)?).ok()?;
        let fingerprint = String::from_utf8(unhex(parts.next()?)?).ok()?;
        let issued: u64 = parts.next()?.parse().ok()?;
        let seen: u64 = parts.next()?.parse().ok()?;
        let auth_at: u64 = parts.next()?.parse().ok()?;
        let nonce = parts.next()?.to_owned();
        if parts.next().is_some()
            || now.saturating_sub(issued) > LIFETIME.as_secs()
            || now.saturating_sub(seen) > IDLE.as_secs()
            || self.revoked().contains_key(&nonce)
        {
            return None;
        }
        Some(Session {
            user,
            csrf: self.csrf(&nonce),
            fingerprint,
            issued,
            seen,
            auth_at,
            nonce,
        })
    }

    /// The live session the request's `Cookie` header names.
    pub fn find(&self, cookie_header: Option<&str>) -> Option<Session> {
        self.read(cookie_value(cookie_header?)?, now())
    }

    /// Ends `s`: its cookie is refused from now on.
    pub fn close(&self, s: &Session) {
        let until = s.issued + LIFETIME.as_secs();
        let now = now();
        let mut revoked = self.revoked();
        revoked.retain(|_, u| *u > now);
        revoked.insert(s.nonce.clone(), until);
    }

    /// The `Set-Cookie` value for `s`.
    pub fn set_cookie(&self, s: &Session) -> String {
        set_cookie(&self.cookie_value(s))
    }
}

/// The session cookie's value in a `Cookie` header.
fn cookie_value(header: &str) -> Option<&str> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == COOKIE).then_some(v)
    })
}

/// The `Set-Cookie` value for a session: not readable by scripts, sent only on requests
/// from these pages.
pub fn set_cookie(value: &str) -> String {
    format!(
        "{COOKIE}={value}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict",
        LIFETIME.as_secs()
    )
}

/// The `Set-Cookie` value that removes the cookie.
pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000;

    #[test]
    fn hmac_matches_rfc_4231_case_2_shape() {
        // RFC 4231 test case 1 uses a 20-byte key; ours is always 32 bytes, so check the
        // construction against a key padded the same way the function pads it.
        let mut key = [0u8; 32];
        key[..20].copy_from_slice(&[0x0b; 20]);
        assert_eq!(
            hex(&hmac(&key, b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn a_cookie_reads_back_until_it_ends() {
        let s = Sessions::with_key([7; 32]);
        let made = s.open("Bob", "fp1", T0);
        let value = s.cookie_value(&made);
        assert_eq!(s.read(&value, T0 + 60), Some(made.clone()));
        // another server with the same key reads it too: a restart keeps the session
        assert_eq!(
            Sessions::with_key([7; 32]).read(&value, T0 + 60),
            Some(made.clone())
        );
        assert_eq!(Sessions::with_key([8; 32]).read(&value, T0 + 60), None);
        // idle, and too old even when used
        assert_eq!(s.read(&value, T0 + IDLE.as_secs() + 1), None);
        let mut used = made.clone();
        used.seen = T0 + LIFETIME.as_secs();
        assert_eq!(
            s.read(&s.cookie_value(&used), T0 + LIFETIME.as_secs() + 1),
            None
        );
        // a changed byte breaks it
        let forged = value.replacen("426f62", "426f63", 1);
        assert_eq!(s.read(&forged, T0 + 60), None);
        assert_eq!(s.read("nonsense", T0), None);
    }

    #[test]
    fn signing_out_refuses_the_cookie_and_fresh_follows_the_password() {
        let s = Sessions::with_key([1; 32]);
        let made = s.open("Bob", "fp", now());
        assert!(made.fresh(made.auth_at + FRESH.as_secs()));
        assert!(!made.fresh(made.auth_at + FRESH.as_secs() + 1));
        let value = s.cookie_value(&made);
        let header = format!("theme=dark; {COOKIE}={value}");
        assert_eq!(s.find(Some(&header)), Some(made.clone()));
        s.close(&made);
        assert_eq!(s.find(Some(&header)), None);
        assert_ne!(made.csrf, value);
        assert_eq!(s.find(None), None);
    }
}
