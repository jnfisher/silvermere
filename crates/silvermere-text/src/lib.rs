//! Message catalogue (from WCC*.MSG), prompt/statline rendering, ANSI.
//!
//! Owns the game's text: the message catalogue and its constructors, rendering with the
//! a/b/c alternative picker, the colour table and palettes, ANSI sequences, the prompt and
//! statline, the MajorMUD menu, 79-column wrapping and the CP437 table.
//!
//! The rendering contract is `docs/text.md`. In short: [`render`] turns a [`Msg`] into
//! the bytes the original's host sent for it, which is `ESC[79D ESC[K` where the
//! original printed `ANSI_CLRLINE`, CP437 text, `\r\n` line ends, and 79-column word
//! wrap only for the messages the original wrapped.

#![forbid(unsafe_code)]

pub mod ansi;
pub mod catalogue;
pub mod colour;
pub mod component;
pub mod cp437;
pub mod editor_screen;
pub mod menu;
pub mod msg;
pub mod prompt;
pub mod render;
pub mod statline;
pub mod strip;
pub mod wrap;

pub use colour::{
    CLRTBL, ColRow, Colour, MAXCLR, Palette, TELL_NOCOLOR, get_hp_colour, hp_colour_row,
};
pub use component::{MAXALTS, Picked, Which, pick};
pub use menu::{MenuInput, RaceChange, render_entry_banner, render_login_greeting, render_menu};
pub use msg::{Msg, Piece};
pub use prompt::{DebugInfo, PromptInput, render_prompt};
pub use render::{Rendered, finish, render, render_into, render_with};
pub use statline::{STATLINELEN, SUBSTMAX, StatlineCtx, substitute};
pub use strip::{strip_ansi, strip_ansi_str};
pub use wrap::{WRAPCOLS, wrap};
