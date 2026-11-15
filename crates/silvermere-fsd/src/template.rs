//! Scanning a template for its fields and help area.

use crate::screen::Screen;
use crate::{MAX_ANSWER, MAX_HELP};

/// What a field holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `?`: any text, or a choice.
    #[default]
    Text,
    /// `$`: a whole number, shown right-justified.
    Number,
}

/// Where a field is in the template and on screen. A field the template lacks has the
/// default: no width.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) kind: Kind,
    /// Offset of the field's first marker in the template.
    pub(crate) offset: usize,
    pub(crate) width: usize,
    /// ANSI command to go to the field (full screen only).
    pub(crate) goto: Vec<u8>,
    /// Attribute of the field when the cursor is not on it.
    pub(crate) attr: u8,
}

/// The help area, where messages appear.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Help {
    pub(crate) offset: usize,
    pub(crate) width: usize,
    pub(crate) goto: Vec<u8>,
    pub(crate) attr: u8,
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    /// The places of the first `fields` field runs; later runs are plain text.
    pub(crate) places: Vec<Place>,
    /// The last `!` run.
    pub(crate) help: Option<Help>,
    /// The bottom screen row the fields and the text reach (full screen only).
    pub(crate) bottom_row: u8,
}

/// Scans a template (as `FSD.C`'s `tmpscn`): a run of two or more `?`, `$` or `!` is a
/// field or the help area; a single one is text. With `on_screen` the text is "printed" to
/// learn each field's position and colour.
pub(crate) fn scan(template: &[u8], fields: usize, on_screen: bool) -> Layout {
    let mut layout = Layout::default();
    let mut screen = on_screen.then(Screen::new);
    let mut printed = 0;
    let mut i = 0;
    while i < template.len() {
        let marker = template[i];
        if !matches!(marker, b'?' | b'$' | b'!') || template.get(i + 1) != Some(&marker) {
            i += 1;
            continue;
        }
        let end = i + template[i..].iter().take_while(|&&b| b == marker).count();
        let (goto, attr) = match &mut screen {
            Some(screen) => {
                screen.print(&template[printed..i]);
                printed = i;
                (goto(screen), Some(screen.attr))
            }
            None => (Vec::new(), None),
        };
        if marker == b'!' {
            layout.help = Some(Help {
                offset: i,
                width: (end - i).min(MAX_HELP),
                goto,
                attr: attr.unwrap_or(0),
            });
        } else if layout.places.len() < fields {
            layout.places.push(Place {
                kind: if marker == b'$' {
                    Kind::Number
                } else {
                    Kind::Text
                },
                offset: i,
                width: (end - i).min(MAX_ANSWER),
                goto,
                attr: attr.unwrap_or(0x07),
            });
            if let Some(screen) = &screen {
                layout.bottom_row = layout.bottom_row.max(screen.row());
            }
        }
        i = end;
    }
    if let Some(screen) = &mut screen {
        screen.print(&template[printed..]);
        layout.bottom_row = layout.bottom_row.max(screen.row());
    }
    layout
}

/// The ANSI command that puts the cursor where the screen's is.
fn goto(screen: &Screen) -> Vec<u8> {
    format!("\x1B[{};{}f", screen.row() + 1, screen.col() + 1).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{Kind, scan};

    #[test]
    fn runs_are_fields_and_single_markers_text() {
        let layout = scan(b"? $ ! # Y/N\x1B[1;33m\rA: ???? $$\r!!!!! ??", 2, true);
        let found: Vec<_> = layout
            .places
            .iter()
            .map(|p| (p.kind, p.offset, p.width, p.goto.clone(), p.attr))
            .collect();
        assert_eq!(
            found,
            [
                (Kind::Text, 22, 4, b"\x1B[2;4f".to_vec(), 0x0E),
                (Kind::Number, 27, 2, b"\x1B[2;9f".to_vec(), 0x0E),
            ]
        );
        let help = layout.help.unwrap();
        assert_eq!((help.offset, help.width), (30, 5));
        assert_eq!(help.goto, b"\x1B[3;1f");
        // the third run is past the specification's fields
        assert_eq!(layout.places.len(), 2);
        assert_eq!(layout.bottom_row, 2);
    }

    #[test]
    fn line_mode_learns_no_positions() {
        let layout = scan(b"\rA: ??", 1, false);
        assert_eq!(layout.places[0].goto, b"");
        assert_eq!(layout.places[0].attr, 0x07);
        assert_eq!(layout.bottom_row, 0);
    }
}
