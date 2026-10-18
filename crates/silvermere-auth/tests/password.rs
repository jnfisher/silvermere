//! argon2id hashing: every stored password is an argon2id hash.

use silvermere_auth::password::{self, Cost};

#[test]
fn hash_verify_round_trip() {
    let h = password::hash("kangtest1");
    assert!(h.starts_with("$argon2id$v=19$"), "{h}");
    assert!(password::verify("kangtest1", &h));
}

#[test]
fn wrong_password_is_false() {
    let h = password::hash_with("kangtest1", Cost::Low);
    assert!(!password::verify("kangtest2", &h));
    assert!(!password::verify("KANGTEST1", &h));
    assert!(!password::verify("", &h));
}

#[test]
fn two_hashes_of_one_password_differ() {
    let a = password::hash("same");
    let b = password::hash("same");
    assert_ne!(a, b, "a fresh salt each time");
    assert!(password::verify("same", &a) && password::verify("same", &b));
}

/// The server's cost is the argon2 crate's defaults (19 MiB, 2 passes, 1 lane); only
/// `Cost::Low`, used by tests that create many accounts, is cheaper. Read from the hash's
/// PHC parameters, not from how long a verify takes: a timing floor failed on a fast runner
/// (20.88 ms against `> 20`, T-119).
#[test]
fn verify_is_not_cheap() {
    let h = password::hash("kangtest1");
    assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
    assert!(password::verify("kangtest1", &h));
    let low = password::hash_with("kangtest1", Cost::Low);
    assert!(low.starts_with("$argon2id$v=19$m=64,t=1,p=1$"), "{low}");
}

#[test]
fn low_cost_hashes_are_still_argon2id() {
    let h = password::hash_with("pw", Cost::Low);
    assert!(h.starts_with("$argon2id$"), "{h}");
    assert!(password::verify("pw", &h));
}
