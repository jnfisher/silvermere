//! A custom statline template with percent codes, expanded the way the original did.
//! The prompt (`SET STATLINE CUSTOM`) and the social actions (T-046, with `%t`) both use
//! it.
//!
//! The template is read one character at a time in one of four states: 0 plain text,
//! 1 after `%`, 2 after `%f`, 3 after `%b`. A code that applies expands into a piece; a
//! character that is no code at all is written after the part of the code read so far
//! (`%z` gives `%z`); a code letter where it does not apply is written without the
//! pending part (`%5` gives `5`, `%fh` gives `h`). Expansion stops once [`SUBSTMAX`]
//! characters are written, but the piece being written at that moment finishes, so the
//! result can be longer.

use crate::ansi::ANSI_RESET;
use crate::colour::{ColRow, Colour, Palette, get_hp_colour};

/// Expansion stops once this many characters are written.
pub const SUBSTMAX: usize = 196;

/// `SET STATLINE [FULL] CUSTOM` cuts the template to this many characters before storing
/// it.
pub const STATLINELEN: usize = 60;

/// What the codes of a template describe: the character, its channel's settings and
/// the caller's `%t` text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatlineCtx<'a> {
    /// `%h`, hit points.
    pub hp: i32,
    /// `%H`, maximum hit points.
    pub maxhp: i32,
    /// `%m`, mana or kai.
    pub mana: i32,
    /// `%M`, maximum mana or kai.
    pub maxmana: i32,
    /// `%x`, experience: the 32-bit running total, printed unsigned.
    pub exp: u32,
    /// The experience needed for the next level, for `%X`: the older formula that takes
    /// the race and class percentages as one sum; `None` when the race or class record
    /// is missing, which makes `%X` print a bare `X`.
    pub exp_needed: Option<i32>,
    /// `%c`: all coins carried, in copper farthings.
    pub wealth_copper: u32,
    /// `%i`, the first name.
    pub first_name: &'a str,
    /// `%w`, the evil warning setting.
    pub evil_warn: bool,
    /// `%r`, resting; wins over meditating.
    pub resting: bool,
    /// `%r`, meditating.
    pub meditating: bool,
    /// `%p`, the possessive pronoun: "his", "her" or "its".
    pub pronoun_possessive: &'a str,
    /// `%P`, the object pronoun: "him", "her" or "it".
    pub pronoun_object: &'a str,
    /// `%t`, the caller's title text; `None` prints "nothing".
    pub title: Option<&'a str>,
    /// The player's palette, for `%d` and the coloured `%h`.
    pub palette: Palette,
}

/// The template as `SET STATLINE` stores it: cut to [`STATLINELEN`] characters
/// Characters, not bytes: the original's template is CP437, one byte per character.
pub fn cut_template(template: &str) -> &str {
    match template.char_indices().nth(STATLINELEN) {
        Some((at, _)) => &template[..at],
        None => template,
    }
}

/// What one template character does.
enum Step {
    /// A code that applies: append the piece.
    Copy(String),
    /// Part of a code (`%`, `%f`, `%b`) or `%r` with nothing to show: the new state, and
    /// nothing written.
    Continue(u8),
    /// A code letter where it does not apply: the character alone is written.
    Plain,
    /// No code letter: the pending `%`, `%f` or `%b`, then the character.
    Default,
}

/// Expands `template` for `ctx`.
///
/// `%n` gives a bare `\r`, as the original wrote it; the renderer's `finish` makes
/// it `\r\n`. The template is not cut here: [`cut_template`] is the storage rule, and
/// the social actions pass their own texts.
pub fn substitute(template: &str, ctx: &StatlineCtx) -> String {
    let prompt_colour = ansi_str(Colour::new(ColRow::Prompt, ctx.palette).prefix());
    let mut out = String::with_capacity(template.len() * 2);
    let mut state: u8 = 0;
    let mut len: usize = 0;
    let mut hpcol = false;

    // the bound is checked before every template character.
    for c in template.chars() {
        if c == '\0' || len >= SUBSTMAX {
            break;
        }
        let step = match c {
            // digits only mean something after %f or %b; a colour
            // chosen by hand switches the %h colouring off again.
            '0'..='7' if state == 2 || state == 3 => {
                hpcol = false;
                let n = c as u8 - b'0';
                let code = match (state, n) {
                    (2, n) => 30 + n,
                    // %b1 gives green, 42, where red would be 41: the original's quirk, kept.
                    (_, 1) => 42,
                    (_, n) => 40 + n,
                };
                Step::Copy(format!("\x1B[{code}m"))
            }
            '0'..='7' => Step::Plain,
            // "%%", and "%f%"/"%b%", give two percent signs.
            '%' if state == 0 => Step::Continue(1),
            '%' => Step::Copy("%%".to_string()),
            // hit points, coloured by health after %d.
            'h' if state == 1 => Step::Copy(if hpcol {
                format!(
                    "{}{}{}",
                    ansi_str(get_hp_colour(ctx.hp, ctx.maxhp, ctx.palette)),
                    ctx.hp,
                    prompt_colour
                )
            } else {
                ctx.hp.to_string()
            }),
            // attributes.
            'B' if state == 1 => Step::Copy("\x1B[1m".to_string()),
            'N' if state == 1 => Step::Copy(ansi_str(ANSI_RESET).to_string()),
            'U' if state == 1 => Step::Copy("\x1B[4m".to_string()),
            'L' if state == 1 => Step::Copy("\x1B[5m".to_string()),
            'R' if state == 1 => Step::Copy("\x1B[7m".to_string()),
            'H' if state == 1 => Step::Copy(ctx.maxhp.to_string()),
            // a colour digit follows.
            'f' if state == 1 => Step::Continue(2),
            'b' if state == 1 => Step::Continue(3),
            // neither resting nor meditating shows nothing at all.
            'r' if state == 1 => {
                if ctx.resting {
                    Step::Copy(" (Resting) ".to_string())
                } else if ctx.meditating {
                    Step::Copy(" (Meditating) ".to_string())
                } else {
                    Step::Continue(0)
                }
            }
            'm' if state == 1 => Step::Copy(ctx.mana.to_string()),
            'M' if state == 1 => Step::Copy(ctx.maxmana.to_string()),
            // all coins carried, printed unsigned.
            'c' if state == 1 => Step::Copy(ctx.wealth_copper.to_string()),
            'p' if state == 1 => Step::Copy(ctx.pronoun_possessive.to_string()),
            'P' if state == 1 => Step::Copy(ctx.pronoun_object.to_string()),
            'i' if state == 1 => Step::Copy(ctx.first_name.to_string()),
            't' if state == 1 => Step::Copy(ctx.title.unwrap_or("nothing").to_string()),
            // the prompt colour, and colour %h from now on.
            'd' if state == 1 => {
                hpcol = true;
                Step::Copy(prompt_colour.to_string())
            }
            'w' if state == 1 => Step::Copy(if ctx.evil_warn { "On" } else { "Off" }.into()),
            // a bare \r, as the original wrote it.
            'n' if state == 1 => Step::Copy("\r".to_string()),
            // printed unsigned.
            'x' if state == 1 => Step::Copy(ctx.exp.to_string()),
            // needed minus exp, floored at 0. Without a race or class record the X is
            // written alone.
            'X' if state == 1 => match ctx.exp_needed {
                Some(needed) => Step::Copy(exp_left(needed, ctx.exp).to_string()),
                None => Step::Plain,
            },
            'h' | 'B' | 'N' | 'U' | 'L' | 'R' | 'H' | 'f' | 'b' | 'r' | 'm' | 'M' | 'c' | 'p'
            | 'P' | 'i' | 't' | 'd' | 'w' | 'n' | 'x' | 'X' => Step::Plain,
            _ => Step::Default,
        };
        match step {
            // every character of the piece counts.
            Step::Copy(piece) => {
                len += piece.chars().count();
                out.push_str(&piece);
                state = 0;
            }
            Step::Continue(next) => state = next,
            Step::Plain | Step::Default => {
                // the pending part is written but not counted.
                if matches!(step, Step::Default) {
                    match state {
                        1 => out.push('%'),
                        2 => out.push_str("%f"),
                        3 => out.push_str("%b"),
                        _ => {}
                    }
                }
                state = 0;
                len += 1;
                out.push(c);
            }
        }
    }
    out
}

/// `%X`: needed minus experience, floored at 0, printed unsigned. The difference wraps
/// in 32 bits and is then read as signed, as the original shows it; that wrap is
/// reproduced.
fn exp_left(needed: i32, exp: u32) -> u32 {
    let left = (needed as u32).wrapping_sub(exp) as i32;
    left.max(0) as u32
}

/// The ANSI byte strings are ASCII.
fn ansi_str(bytes: &'static [u8]) -> &'static str {
    std::str::from_utf8(bytes).expect("ANSI strings are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_left_wraps_like_the_original() {
        assert_eq!(exp_left(1000, 400), 600);
        assert_eq!(exp_left(1000, 1000), 0);
        assert_eq!(exp_left(1000, 5000), 0);
    }

    #[test]
    fn cut_counts_characters() {
        let long = "é".repeat(70);
        assert_eq!(cut_template(&long).chars().count(), 60);
        assert_eq!(cut_template("short"), "short");
    }
}
