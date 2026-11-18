//! Output helpers: blanks, cursor moves, colours and answers formatted for a field.

use crate::MAX_ANSWER;
use crate::template::{Kind, Place};

/// Attribute of the field the cursor is on (black on white, as the library's `fsdapr`).
pub(crate) const CURSOR_ATTR: u8 = 0x70;
/// The terminal bell.
pub(crate) const BEL: u8 = 0x07;

/// Outputs `n` blanks, at most `MAX_ANSWER + 1`.
pub(crate) fn spaces(n: usize, out: &mut Vec<u8>) {
    out.resize(out.len() + n.min(MAX_ANSWER + 1), b' ');
}

/// Outputs `n` copies of backspace-blank-backspace.
pub(crate) fn rubout(n: usize, out: &mut Vec<u8>) {
    for _ in 0..n {
        out.extend_from_slice(b"\x08 \x08");
    }
}

/// Outputs an ANSI cursor move of 1 to 99 places: `A` up, `B` down, `C` right, `D` left;
/// nothing for other distances (as the library's `ansmov`).
pub(crate) fn cursor_move(n: usize, cmd: u8, out: &mut Vec<u8>) {
    if !(1..100).contains(&n) {
        return;
    }
    out.extend_from_slice(b"\x1B[");
    if n > 1 {
        out.extend_from_slice(n.to_string().as_bytes());
    }
    out.push(cmd);
}

/// Outputs the ANSI colour sequence for a PC attribute byte (as the host's `ibm2ans`).
pub(crate) fn colour(attr: u8, out: &mut Vec<u8>) {
    const ANSI_COLOUR: [u8; 8] = *b"04261537";
    out.extend_from_slice(b"\x1B[0");
    if attr == 0x70 {
        out.extend_from_slice(b";7");
    } else {
        if attr & 0x80 != 0 {
            out.extend_from_slice(b";5");
        }
        if attr & 0x08 != 0 {
            out.extend_from_slice(b";1");
        }
        let fg = usize::from(attr & 0x07);
        if fg != 0 {
            out.extend_from_slice(&[b';', b'3', ANSI_COLOUR[fg]]);
        }
        let bg = usize::from((attr & 0x70) >> 4);
        if bg != 0x07 {
            out.extend_from_slice(&[b';', b'4', ANSI_COLOUR[bg]]);
        }
    }
    out.push(b'm');
}

/// How an answer fills its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Justify {
    /// Padded to the field width; a number right-justified.
    Field,
    /// Padded to the field width, left-justified.
    Left,
}

/// Outputs `answer` formatted for the field at `place` (as the library's `dspans`).
pub(crate) fn answer_in(place: &Place, answer: &[u8], justify: Justify, out: &mut Vec<u8>) {
    let pad = place.width.saturating_sub(answer.len());
    match justify {
        Justify::Field if place.kind == Kind::Number => {
            spaces(pad, out);
            out.extend_from_slice(answer);
        }
        Justify::Field | Justify::Left => {
            out.extend_from_slice(answer);
            spaces(pad, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Justify, answer_in, colour, cursor_move};
    use crate::template::{Kind, Place};

    #[test]
    fn cursor_moves_of_one_to_ninety_nine() {
        let mut out = Vec::new();
        for (n, cmd) in [(1, b'C'), (7, b'D'), (42, b'A'), (0, b'D'), (100, b'D')] {
            cursor_move(n, cmd, &mut out);
        }
        assert_eq!(out, b"\x1B[C\x1B[7D\x1B[42A");
    }

    #[test]
    fn colours() {
        let mut out = Vec::new();
        for attr in [0x70, 0x07, 0x1E, 0x8C] {
            colour(attr, &mut out);
        }
        assert_eq!(
            out,
            b"\x1B[0;7m\x1B[0;37;40m\x1B[0;1;33;44m\x1B[0;5;1;31;40m"
        );
    }

    #[test]
    fn numbers_are_right_justified_unless_left() {
        let number = Place {
            kind: Kind::Number,
            width: 4,
            ..Place::default()
        };
        let mut out = Vec::new();
        answer_in(&number, b"42", Justify::Field, &mut out);
        answer_in(&number, b"42", Justify::Left, &mut out);
        assert_eq!(out, b"  4242  ");
    }
}
