//! The IAC parser: a pure byte-at-a-time state machine, so it can be tested without sockets.

/// Interpret As Command: every telnet command starts with this byte; doubled, it is a data 0xFF.
pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
/// Subnegotiation begin.
pub const SB: u8 = 250;
/// No operation (the optional keepalive).
pub const NOP: u8 = 241;
/// Subnegotiation end.
pub const SE: u8 = 240;

/// Longest subnegotiation payload kept. NAWS needs four bytes; anything longer is an option we
/// refused anyway, so the excess is discarded rather than buffered without bound.
const SB_MAX: usize = 64;

/// The four negotiation verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Will,
    Wont,
    Do,
    Dont,
}

impl Verb {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            WILL => Some(Self::Will),
            WONT => Some(Self::Wont),
            DO => Some(Self::Do),
            DONT => Some(Self::Dont),
            _ => None,
        }
    }

    /// The command byte for this verb.
    pub fn byte(self) -> u8 {
        match self {
            Self::Will => WILL,
            Self::Wont => WONT,
            Self::Do => DO,
            Self::Dont => DONT,
        }
    }
}

/// What one input byte amounted to, once it completes something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A data byte for the line editor (`IAC IAC` yields `Data(0xFF)`).
    Data(u8),
    /// `IAC <verb> <option>`.
    Negotiate(Verb, u8),
    /// `IAC SB <option> <payload> IAC SE`, with `IAC IAC` in the payload undoubled.
    Subnegotiation(u8, Vec<u8>),
    /// Any other two-byte command (`IAC NOP`, `IAC AYT`, ...): ignored by the transport.
    Command(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    Verb(Verb),
    SbOption,
    SbData,
    SbIac,
}

/// Telnet input parser. Feed it every received byte in order.
#[derive(Debug, Clone)]
pub struct Parser {
    state: State,
    sb_option: u8,
    sb_payload: Vec<u8>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Data,
            sb_option: 0,
            sb_payload: Vec::new(),
        }
    }

    /// Consumes one byte; returns the action it completes, if any.
    pub fn feed(&mut self, byte: u8) -> Option<Action> {
        match self.state {
            State::Data => {
                if byte == IAC {
                    self.state = State::Iac;
                    None
                } else {
                    Some(Action::Data(byte))
                }
            }
            State::Iac => {
                self.state = State::Data;
                if byte == IAC {
                    Some(Action::Data(IAC))
                } else if let Some(verb) = Verb::from_byte(byte) {
                    self.state = State::Verb(verb);
                    None
                } else if byte == SB {
                    self.state = State::SbOption;
                    None
                } else {
                    Some(Action::Command(byte))
                }
            }
            State::Verb(verb) => {
                self.state = State::Data;
                Some(Action::Negotiate(verb, byte))
            }
            State::SbOption => {
                self.sb_option = byte;
                self.sb_payload.clear();
                self.state = State::SbData;
                None
            }
            State::SbData => {
                if byte == IAC {
                    self.state = State::SbIac;
                } else {
                    self.push_sb(byte);
                }
                None
            }
            State::SbIac => match byte {
                SE => {
                    self.state = State::Data;
                    Some(Action::Subnegotiation(
                        self.sb_option,
                        std::mem::take(&mut self.sb_payload),
                    ))
                }
                IAC => {
                    self.push_sb(IAC);
                    self.state = State::SbData;
                    None
                }
                // A malformed subnegotiation: drop it and resynchronise on plain data.
                _ => {
                    self.sb_payload.clear();
                    self.state = State::Data;
                    None
                }
            },
        }
    }

    fn push_sb(&mut self, byte: u8) {
        if self.sb_payload.len() < SB_MAX {
            self.sb_payload.push(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8]) -> Vec<Action> {
        let mut p = Parser::new();
        bytes.iter().filter_map(|&b| p.feed(b)).collect()
    }

    #[test]
    fn plain_data_passes_through() {
        assert_eq!(run(b"ab"), vec![Action::Data(b'a'), Action::Data(b'b')]);
    }

    #[test]
    fn doubled_iac_is_one_data_byte() {
        assert_eq!(run(&[IAC, IAC]), vec![Action::Data(0xFF)]);
    }

    #[test]
    fn negotiation_is_recognised() {
        assert_eq!(run(&[IAC, DO, 1]), vec![Action::Negotiate(Verb::Do, 1)]);
    }

    #[test]
    fn subnegotiation_collects_payload_and_undoubles_iac() {
        assert_eq!(
            run(&[IAC, SB, 31, 0, IAC, IAC, 0, 24, IAC, SE, b'x']),
            vec![
                Action::Subnegotiation(31, vec![0, 0xFF, 0, 24]),
                Action::Data(b'x')
            ]
        );
    }

    #[test]
    fn other_commands_are_reported() {
        assert_eq!(run(&[IAC, NOP]), vec![Action::Command(NOP)]);
    }

    #[test]
    fn state_survives_chunk_boundaries() {
        let mut p = Parser::new();
        assert_eq!(p.feed(IAC), None);
        assert_eq!(p.feed(WILL), None);
        assert_eq!(p.feed(31), Some(Action::Negotiate(Verb::Will, 31)));
    }
}
