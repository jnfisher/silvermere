//! The typed message enum, generated from `messages/catalogue.toml` by `build.rs`.
//!
//! Every message the original printed becomes a catalogue entry and so a `Msg` variant
//! with named fields for its arguments. The
//! engine appends `Msg` values to a session's `OutputBatch` (T-015); `render` turns them
//! into the bytes the original sent.

use crate::colour::ColRow;

/// One piece of a message template (`Msg::pieces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// Literal text (UTF-8; encoded to CP437 at render time).
    Text(&'static str),
    /// An ANSI string, already resolved.
    Ansi(&'static [u8]),
    /// The argument with this index (`Msg::write_arg`).
    Arg(usize),
    /// A colour-table row, resolved with the player's palette at render time.
    Colour(ColRow),
}

include!(concat!(env!("OUT_DIR"), "/msg_generated.rs"));

/// The catalogue the enum was generated from, for tooling and tests.
pub const CATALOGUE_TOML: &str = include_str!("../messages/catalogue.toml");

impl Msg {
    /// The template with arguments substituted, before the host-side steps (if-ANSI,
    /// line ends, CP437, wrap) and without colour rows. For logs and tests.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if let Some(pieces) = crate::render::moved_pieces(self) {
            for piece in pieces {
                match piece {
                    crate::catalogue::TemplatePiece::Text(t) => out.push_str(t),
                    crate::catalogue::TemplatePiece::Ansi(name) => out.push_str(
                        std::str::from_utf8(crate::ansi::by_name(name).unwrap_or_default())
                            .expect("ANSI strings are ASCII"),
                    ),
                    _ => {}
                }
            }
            return out;
        }
        for piece in self.pieces() {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Ansi(a) => {
                    out.push_str(std::str::from_utf8(a).expect("ANSI strings are ASCII"))
                }
                Piece::Arg(i) => self.write_arg(*i, &mut out),
                Piece::Colour(_) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_catalogue() {
        let catalogue = crate::catalogue::parse(CATALOGUE_TOML).unwrap();
        let keys: Vec<&str> = catalogue.messages.keys().map(String::as_str).collect();
        assert_eq!(Msg::KEYS, keys.as_slice());
        let examples = Msg::examples();
        assert_eq!(examples.len(), keys.len());
        for (msg, key) in examples.iter().zip(keys) {
            assert_eq!(msg.key(), key);
        }
    }

    #[test]
    fn text_substitutes_arguments() {
        let m = Msg::ExpGain { exp: 13 };
        assert_eq!(m.text(), "\x1B[79D\x1B[KYou gain 13 experience.\r");
        assert_eq!(m.pattern(), Some("exp_gain"));
        assert_eq!(m.wrap(), None);
    }
}
