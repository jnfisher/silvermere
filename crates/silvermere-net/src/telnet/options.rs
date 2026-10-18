//! The option table: what the server offers on accept and how it answers the client.
//!
//! The server offers `WILL ECHO`,
//! `WILL SGA`, `WILL BINARY` and `DO NAWS`; it agrees to `DO` only for those three options and
//! to `WILL` only for NAWS, and refuses every other option. Refusals from the client (`WONT`,
//! `DONT`) are accepted silently and never retried.

use super::parser::{DO, DONT, IAC, Verb, WILL, WONT};

pub const BINARY: u8 = 0;
pub const ECHO: u8 = 1;
pub const SGA: u8 = 3;
pub const TTYPE: u8 = 24;
pub const NAWS: u8 = 31;
pub const LINEMODE: u8 = 34;

/// Sent on accept, in this order. MBBSEmu's `TelnetSession` also sends `DO BINARY`, `DONT
/// ECHO`, `DO SGA` and `WONT NAWS`; those are its own noise (MegaMud ignores them) and are not
/// carried.
pub const OFFERS: [u8; 12] = [
    IAC, WILL, ECHO, IAC, WILL, SGA, IAC, WILL, BINARY, IAC, DO, NAWS,
];

/// Whether the server performs `option` itself when the client asks (`DO option`).
fn server_supports(option: u8) -> bool {
    matches!(option, ECHO | SGA | BINARY)
}

/// Whether the server lets the client perform `option` (`WILL option`).
fn client_may(option: u8) -> bool {
    option == NAWS
}

/// Per-connection negotiation state.
///
/// Every `DO`/`WILL` is answered per the table, but the same answer is not repeated for an
/// option until a `WONT`/`DONT` for it resets the state. That answers the first `DO ECHO` with
/// `WILL ECHO` as MBBSEmu does, while a client that acknowledges acknowledgements cannot start
/// a negotiation loop.
#[derive(Debug, Clone)]
pub struct Options {
    last_reply: [Option<u8>; 256],
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    pub fn new() -> Self {
        Self {
            last_reply: [None; 256],
        }
    }

    /// The reply to the client's `IAC <verb> <option>`, if one is due.
    pub fn respond(&mut self, verb: Verb, option: u8) -> Option<[u8; 3]> {
        let slot = &mut self.last_reply[usize::from(option)];
        let reply = match verb {
            Verb::Do if server_supports(option) => WILL,
            Verb::Do => WONT,
            Verb::Will if client_may(option) => DO,
            Verb::Will => DONT,
            Verb::Wont | Verb::Dont => {
                *slot = None;
                return None;
            }
        };
        if *slot == Some(reply) {
            return None;
        }
        *slot = Some(reply);
        Some([IAC, reply, option])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_do_is_accepted_once() {
        let mut o = Options::new();
        assert_eq!(o.respond(Verb::Do, ECHO), Some([IAC, WILL, ECHO]));
        assert_eq!(o.respond(Verb::Do, ECHO), None);
    }

    #[test]
    fn unsupported_do_is_refused() {
        let mut o = Options::new();
        assert_eq!(o.respond(Verb::Do, TTYPE), Some([IAC, WONT, TTYPE]));
    }

    #[test]
    fn will_is_refused_except_naws() {
        let mut o = Options::new();
        assert_eq!(o.respond(Verb::Will, LINEMODE), Some([IAC, DONT, LINEMODE]));
        assert_eq!(o.respond(Verb::Will, NAWS), Some([IAC, DO, NAWS]));
    }

    #[test]
    fn refusals_are_accepted_silently_and_reset_state() {
        let mut o = Options::new();
        assert_eq!(o.respond(Verb::Wont, NAWS), None);
        assert_eq!(o.respond(Verb::Do, ECHO), Some([IAC, WILL, ECHO]));
        assert_eq!(o.respond(Verb::Dont, ECHO), None);
        assert_eq!(o.respond(Verb::Do, ECHO), Some([IAC, WILL, ECHO]));
    }
}
