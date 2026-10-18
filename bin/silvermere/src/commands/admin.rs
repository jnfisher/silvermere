//! `admin token new|list`: bearer tokens for the admin API (T-060). Revoking a token is
//! deleting its `[[admin.tokens]]` block and restarting (`docs/admin-api.md#tokens`).

use std::process::ExitCode;

use anyhow::bail;
use rand::Rng as _;
use silvermere_api::Scope;
use silvermere_api::auth::sha256_hex;

use crate::cli::{AdminCmd, TokenCmd};
use crate::config::Config;

/// The prefix that marks a silvermere token in a log or a leaked file.
pub const TOKEN_PREFIX: &str = "sm_";

/// Base64url without padding.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// A fresh token: `sm_` and 32 random bytes as base64url.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", base64url(&bytes))
}

/// The `[[admin.tokens]]` block for `token`.
pub fn config_block(name: &str, token: &str, scopes: &[Scope]) -> String {
    let scopes: Vec<String> = scopes.iter().map(|s| format!("\"{}\"", s.name())).collect();
    format!(
        "[[admin.tokens]]\nname = {}\nsha256 = \"{}\"\nscopes = [{}]\n",
        toml::Value::String(name.to_string()),
        sha256_hex(token),
        scopes.join(", ")
    )
}

/// Parses `--scopes read,write` (the T-018 names `broadcast` and `sysop` are accepted).
pub fn parse_scopes(words: &[String]) -> anyhow::Result<Vec<Scope>> {
    let mut out = Vec::new();
    for w in words {
        let Some(s) = Scope::parse(w) else {
            bail!("unknown scope `{w}`; expected read, write or admin");
        };
        if !out.contains(&s) {
            out.push(s);
        }
    }
    if out.is_empty() {
        bail!("at least one scope is needed");
    }
    Ok(out)
}

pub fn run(cfg: &Config, action: &AdminCmd) -> anyhow::Result<ExitCode> {
    let AdminCmd::Token { action } = action;
    match action {
        TokenCmd::New { name, scopes } => {
            if name.trim().is_empty() {
                bail!("the token needs a name");
            }
            let scopes = parse_scopes(scopes)?;
            let token = generate_token();
            println!("Token (shown once, store it now): {token}");
            println!();
            println!("Add to silvermere.toml, then restart:");
            println!();
            print!("{}", config_block(name, &token, &scopes));
        }
        TokenCmd::List => {
            if cfg.admin.tokens.is_empty() {
                println!("no tokens configured");
            }
            for t in &cfg.admin.tokens {
                let scopes: Vec<_> = t.scopes.iter().map(|s| s.to_scope().name()).collect();
                let prefix = t.hash().map_or_else(
                    || "invalid".to_string(),
                    |h| sha256_hex_of(&h)[..8].to_string(),
                );
                println!("{}\t{}\tsha256:{}", t.name, scopes.join(","), prefix);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn sha256_hex_of(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc4648_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn tokens_are_prefixed_unique_and_43_chars_of_payload() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(a.len(), 3 + 43);
        assert!(
            a[3..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn scopes_parse_and_reject() {
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_scopes(&w(&["read", "write", "read"])).unwrap(),
            [Scope::Read, Scope::Write]
        );
        assert!(parse_scopes(&w(&["root"])).is_err());
    }
}
