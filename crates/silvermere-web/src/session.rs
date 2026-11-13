//! Who is signed in. A session lives in memory and ends with the server, after
//! [`LIFETIME`], or on sign-out; the cookie holds its random name only.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::Rng as _;

/// The cookie that names the session.
pub const COOKIE: &str = "silvermere_session";
/// How long a session lasts.
pub const LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
/// The most sessions kept; the oldest goes first.
const MOST: usize = 64;

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
}

/// A signed-in sysop.
#[derive(Debug, Clone)]
pub struct Session {
    pub user: String,
    /// The token every form of this session carries; a post without it is refused.
    pub csrf: String,
    until: Instant,
}

/// The sessions by cookie value.
#[derive(Debug, Default)]
pub struct Sessions {
    map: Mutex<HashMap<String, Session>>,
}

/// 32 random bytes as hex.
fn random_name() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Sessions {
    /// A new session for `user`; the cookie value.
    pub fn open(&self, user: &str) -> String {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        map.retain(|_, s| s.until > now);
        if map.len() >= MOST
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, s)| s.until)
                .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
        let name = random_name();
        map.insert(
            name.clone(),
            Session {
                user: user.to_owned(),
                csrf: random_name(),
                until: now + LIFETIME,
            },
        );
        name
    }

    /// The live session the request's `Cookie` header names.
    pub fn find(&self, cookie_header: Option<&str>) -> Option<Session> {
        let name = cookie_value(cookie_header?)?;
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(name).filter(|s| s.until > Instant::now()).cloned()
    }

    /// Ends the session the header names.
    pub fn close(&self, cookie_header: Option<&str>) {
        if let Some(name) = cookie_header.and_then(cookie_value) {
            let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
            map.remove(name);
        }
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
pub fn set_cookie(name: &str) -> String {
    format!(
        "{COOKIE}={name}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict",
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

    #[test]
    fn a_session_is_found_by_its_cookie_until_closed() {
        let s = Sessions::default();
        let name = s.open("Bob");
        assert_eq!(name.len(), 64);
        let header = format!("theme=dark; {COOKIE}={name}");
        let found = s.find(Some(&header)).expect("open");
        assert_eq!(found.user, "Bob");
        assert_ne!(found.csrf, name);
        assert!(s.find(Some(&format!("{COOKIE}=nope"))).is_none());
        assert!(s.find(None).is_none());
        s.close(Some(&header));
        assert!(s.find(Some(&header)).is_none());
    }

    #[test]
    fn the_oldest_session_makes_room() {
        let s = Sessions::default();
        let first = s.open("A");
        for _ in 0..MOST {
            s.open("B");
        }
        assert!(s.find(Some(&format!("{COOKIE}={first}"))).is_none());
    }
}
