//! Answer strings: `NAME=value` entries, each ending in a NUL, then a final NUL.

/// The value of the first entry `name=` in an answer string, any case; empty if none (as
/// `FSD.C`'s `fsdxan`).
pub(crate) fn lookup<'a>(answers: &'a [u8], name: &[u8]) -> &'a [u8] {
    answers
        .split(|&b| b == 0)
        .take_while(|entry| !entry.is_empty())
        .find_map(|entry| {
            let (head, rest) = entry.split_at_checked(name.len())?;
            (head.eq_ignore_ascii_case(name) && rest.first() == Some(&b'=')).then(|| &rest[1..])
        })
        .unwrap_or_default()
}

/// The answer string of `entries`: name and value pairs.
pub(crate) fn join<'a>(entries: impl Iterator<Item = (&'a [u8], &'a [u8])>) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in entries {
        out.extend_from_slice(name);
        out.push(b'=');
        out.extend_from_slice(value);
        out.push(0);
    }
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::{join, lookup};

    #[test]
    fn lookup_by_name_any_case() {
        let ans = b"NAME=Fred\0RANK=MAJOR\0RANK=X\0\0LATE=1\0";
        assert_eq!(lookup(ans, b"RANK"), b"MAJOR");
        assert_eq!(lookup(ans, b"name"), b"Fred");
        assert_eq!(lookup(ans, b"NAM"), b"");
        // the answer string ends at the double NUL
        assert_eq!(lookup(ans, b"LATE"), b"");
    }

    #[test]
    fn join_ends_in_a_double_nul() {
        let entries = [(&b"A"[..], &b"1"[..]), (b"B", b"")];
        assert_eq!(join(entries.into_iter()), b"A=1\0B=\0\0");
        assert_eq!(join([].into_iter()), b"\0");
    }
}
