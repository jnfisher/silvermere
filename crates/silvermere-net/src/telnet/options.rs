//! The option table: what the server offers on accept and how it answers the client.
//!
//! The server offers `WILL ECHO`, `WILL SGA`, `WILL BINARY` and `DO NAWS`, and `WILL GMCP`
//! and `WILL CHARSET` when they are on; it agrees to `DO` only for those options and to `WILL`
//! only for NAWS, and refuses every other option. Refusals from the client (`WONT`, `DONT`)
//! are accepted silently and never retried.

use super::parser::{DO, DONT, IAC, Verb, WILL, WONT};

pub const BINARY: u8 = 0;
pub const ECHO: u8 = 1;
pub const SGA: u8 = 3;
pub const TTYPE: u8 = 24;
pub const NAWS: u8 = 31;
pub const LINEMODE: u8 = 34;
/// Character set negotiation (RFC 2066): tells the client which encoding the text is in.
pub const CHARSET: u8 = 42;
/// Generic MUD Communication Protocol (<https://tintin.mudhalla.net/protocols/gmcp/>).
pub const GMCP: u8 = 201;

/// Sent on accept, in this order. MBBSEmu's `TelnetSession` also sends `DO BINARY`, `DONT
/// ECHO`, `DO SGA` and `WONT NAWS`; those are its own noise (MegaMud ignores them) and are not
/// carried.
pub const OFFERS: [u8; 12] = [
    IAC, WILL, ECHO, IAC, WILL, SGA, IAC, WILL, BINARY, IAC, DO, NAWS,
];

/// Sent after [`OFFERS`] when GMCP is on.
pub const GMCP_OFFER: [u8; 3] = [IAC, WILL, GMCP];
/// Sent after [`OFFERS`] (and [`GMCP_OFFER`]) when CHARSET is on.
pub const CHARSET_OFFER: [u8; 3] = [IAC, WILL, CHARSET];

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
    /// GMCP is offered.
    gmcp: bool,
    /// CHARSET is offered.
    charset: bool,
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
            gmcp: false,
            charset: false,
        }
    }

    /// The table with GMCP offered (`IAC WILL GMCP` goes out on accept, so the client's
    /// `DO GMCP` is its acknowledgement and is not answered again).
    pub fn with_gmcp() -> Self {
        let mut o = Self::new();
        o.offer_gmcp();
        o
    }

    /// Offers GMCP ([`Self::with_gmcp`]).
    pub fn offer_gmcp(&mut self) {
        self.gmcp = true;
        self.last_reply[usize::from(GMCP)] = Some(WILL);
    }

    /// Offers CHARSET, as [`Self::offer_gmcp`] offers GMCP.
    pub fn offer_charset(&mut self) {
        self.charset = true;
        self.last_reply[usize::from(CHARSET)] = Some(WILL);
    }

    /// Whether the server performs `option` itself when the client asks (`DO option`).
    fn server_supports(&self, option: u8) -> bool {
        matches!(option, ECHO | SGA | BINARY)
            || (self.gmcp && option == GMCP)
            || (self.charset && option == CHARSET)
    }

    /// The reply to the client's `IAC <verb> <option>`, if one is due.
    pub fn respond(&mut self, verb: Verb, option: u8) -> Option<[u8; 3]> {
        let supported = self.server_supports(option);
        let slot = &mut self.last_reply[usize::from(option)];
        let reply = match verb {
            Verb::Do if supported => WILL,
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

    #[test]
    fn gmcp_is_refused_unless_offered() {
        let mut o = Options::new();
        assert_eq!(o.respond(Verb::Do, GMCP), Some([IAC, WONT, GMCP]));
    }

    #[test]
    fn offered_gmcp_acknowledgement_is_not_answered() {
        let mut o = Options::with_gmcp();
        assert_eq!(o.respond(Verb::Do, GMCP), None);
        // switched off and on again by the client: answered once more
        assert_eq!(o.respond(Verb::Dont, GMCP), None);
        assert_eq!(o.respond(Verb::Do, GMCP), Some([IAC, WILL, GMCP]));
    }
}
