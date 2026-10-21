//! Rendering: from a `Msg` to the bytes the original's host sent for it.
//!
//! The module built its text with the host's `prf()` and handed it to the host to send
//! (`btuxmt`); the host then resolved if-ANSI strings, turned `\r` into `\r\n`,
//! word-wrapped at the `btutsw` width and escaped `0xFF`. Here [`render`] does the
//! module's part (template, arguments, colour rows) and [`finish`] the host's, minus the
//! `0xFF` doubling, which is the transport's (T-012).

use crate::ansi::{self, resolve_if_ansi};
use crate::catalogue::TemplatePiece;
use crate::colour::{Colour, Palette};
use crate::cp437;
use crate::msg::{Msg, Piece};
use crate::wrap::wrap;

/// A rendered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// CP437 bytes with ANSI, `\r\n` line ends, wrapped if the message asked.
    pub bytes: Vec<u8>,
    /// Whether the bytes were word-wrapped.
    pub wraps: bool,
}

/// Renders one message in the player's palette, wrapped at the width the catalogue
/// gives it (`Msg::wrap`).
pub fn render(msg: &Msg, palette: Palette) -> Rendered {
    render_with(msg, palette, msg.wrap())
}

/// [`render`] with the wrap width chosen by the caller (`None`: no wrapping).
pub fn render_with(msg: &Msg, palette: Palette, wrap_width: Option<u8>) -> Rendered {
    let bytes = finish(&prf(msg, palette), wrap_width);
    Rendered {
        bytes,
        wraps: wrap_width.is_some(),
    }
}

/// Appends the rendered message to `out`.
pub fn render_into(out: &mut Vec<u8>, msg: &Msg, palette: Palette) {
    out.extend_from_slice(&render(msg, palette).bytes);
}

/// The module's part: the template with its arguments, ANSI strings and colour rows,
/// as the module built it (CP437, `\r` line ends as written). Resolving if-ANSI, line ends
/// and wrapping are [`finish`]'s. A moved entry ([`crate::package::MOVED_KEYS`]) takes its
/// template from the installed package text.
pub fn prf(msg: &Msg, palette: Palette) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(pieces) = moved_pieces(msg) {
        for piece in pieces {
            match piece {
                TemplatePiece::Text(t) => cp437::encode_into(&mut out, t),
                TemplatePiece::Ansi(name) => {
                    out.extend_from_slice(ansi::by_name(name).unwrap_or_default())
                }
                TemplatePiece::Colour(row) => {
                    out.extend_from_slice(Colour::new(*row, palette).prefix())
                }
                // the package's templates take no argument (checked when loaded)
                TemplatePiece::Arg(_) => {}
            }
        }
        return out;
    }
    let mut arg = String::new();
    for piece in msg.pieces() {
        match piece {
            Piece::Text(t) => cp437::encode_into(&mut out, t),
            Piece::Ansi(a) => out.extend_from_slice(a),
            Piece::Arg(i) => {
                arg.clear();
                msg.write_arg(*i, &mut arg);
                cp437::encode_into(&mut out, &arg);
            }
            Piece::Colour(row) => out.extend_from_slice(Colour::new(*row, palette).prefix()),
        }
    }
    out
}

/// The installed package text's pieces for a moved entry.
pub(crate) fn moved_pieces(msg: &Msg) -> Option<&'static [TemplatePiece]> {
    crate::package::installed()?.catalogue_pieces(msg.key())
}

/// The host's part for the module's text: if-ANSI strings resolved to their ANSI half, every
/// line end normalised to `\r\n`, then word wrap at `wrap_width` if given. The prompt
/// (T-020) and other raw text go through this too.
pub fn finish(prf: &[u8], wrap_width: Option<u8>) -> Vec<u8> {
    let resolved = resolve_if_ansi(prf);
    let normalised = normalise_line_ends(&resolved);
    match wrap_width {
        Some(width) => wrap(&normalised, width),
        None => normalised,
    }
}

/// The host's `FormatNewLineCarriageReturn`: `\r\n` stays, `\n\r` becomes one `\r\n`, a
/// lone `\r` or `\n` becomes `\r\n` (MBBSEmu's `outprf`).
pub fn normalise_line_ends(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 8);
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'\r' | b'\n' => {
                out.extend_from_slice(b"\r\n");
                let pair = input[i] == b'\r' && input.get(i + 1) == Some(&b'\n')
                    || input[i] == b'\n' && input.get(i + 1) == Some(&b'\r');
                i += if pair { 2 } else { 1 };
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ends() {
        assert_eq!(normalise_line_ends(b"a\rb"), b"a\r\nb");
        assert_eq!(normalise_line_ends(b"a\nb"), b"a\r\nb");
        assert_eq!(normalise_line_ends(b"a\r\nb"), b"a\r\nb");
        assert_eq!(normalise_line_ends(b"a\n\rb"), b"a\r\nb");
        assert_eq!(normalise_line_ends(b"\r\r"), b"\r\n\r\n");
        assert_eq!(normalise_line_ends(b"\r\n\r\n"), b"\r\n\r\n");
    }

    // ANSI_CLRLINE, then "You gain 13 experience." and a line end
    #[test]
    fn clrline_message_bytes() {
        let r = render(&Msg::ExpGain { exp: 13 }, Palette::default());
        assert_eq!(r.bytes, b"\x1B[79D\x1B[KYou gain 13 experience.\r\n");
        assert!(!r.wraps);
    }

    // the exits label is in the COL_EXITS colour of the player's palette
    #[test]
    fn colour_rows_follow_the_palette() {
        let m = Msg::RoomExits {
            exits: "north".into(),
        };
        assert_eq!(
            render(&m, Palette::new(0).unwrap()).bytes,
            b"\x1B[0;32mObvious exits: north\r\n"
        );
        assert_eq!(
            render(&m, Palette::new(1).unwrap()).bytes,
            b"\x1B[0;33mObvious exits: north\r\n"
        );
        assert!(render(&m, Palette::default()).wraps);
    }

    #[test]
    fn finish_resolves_raw_if_ansi() {
        let raw = b"\x1B[[\x1B[1;31m| \x08 \x08 \x08 \x08]Hi\r";
        assert_eq!(finish(raw, None), b"\x1B[1;31mHi\r\n");
    }
}
