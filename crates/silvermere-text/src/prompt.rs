//! The in-game prompt that follows almost every message, in the five statline modes, and
//! the literal texts of `messages/prompt.toml` that it and the menu (`menu.rs`) print.
//!
//! The prompt is never followed by a newline: the next message starts with
//! `ANSI_CLRLINE` and wipes it. The original's `^A` terminator, which the host consumed,
//! is not sent.
//!
//! | mode | prints |
//! |---|---|
//! | `SL_OFF` | `>` |
//! | `SL_ON`, `SL_FULL`, no mana | `[HP=48]:`, the mark inside: `[HP=29 (Resting) ]:` |
//! | `SL_ON`, `SL_FULL`, mana | `[HP=32/MA=22]:`, the mark after: `[HP=7/MA=19]: (Resting) ` |
//! | above `SL_FULL` | the custom template through [`substitute`] |

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use silvermere_types::enums::{Magery, StatlineMode};

use crate::ansi::ANSI_CLRLINE;
use crate::colour::{ColRow, Colour, Palette, get_hp_colour};
use crate::cp437;
use crate::render::finish;
use crate::statline::{StatlineCtx, cut_template, substitute};

/// `messages/prompt.toml`, the literal pieces of the prompt, menu and entry screen.
pub const PROMPT_TOML: &str = include_str!("../messages/prompt.toml");

#[derive(Debug, Deserialize)]
struct TextFile {
    texts: BTreeMap<String, TextEntry>,
}

/// One entry of `messages/prompt.toml`.
#[derive(Debug, Deserialize)]
pub struct TextEntry {
    /// The bytes the original printed, `\r` line ends as written.
    pub text: String,
}

static TEXTS: LazyLock<BTreeMap<String, TextEntry>> = LazyLock::new(|| {
    toml::from_str::<TextFile>(PROMPT_TOML)
        .expect("messages/prompt.toml parses")
        .texts
});

/// Every entry of `messages/prompt.toml`, by key.
pub fn prompt_texts() -> &'static BTreeMap<String, TextEntry> {
    &TEXTS
}

/// The text of a `messages/prompt.toml` entry. The keys are fixed in this crate and a
/// test renders every one, so a missing key is a bug here, not an input error.
pub fn prompt_text(key: &str) -> &'static str {
    match TEXTS.get(key) {
        Some(entry) => &entry.text,
        None => panic!("messages/prompt.toml has no entry {key:?}"),
    }
}

/// The sysop debug suffix, shown to a sysop with debug mode on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugInfo {
    /// The character's energy.
    pub energy: i32,
    /// Sneaking.
    pub sneaking: bool,
    /// Just arrived in the room.
    pub just_arrived: bool,
    /// Hidden.
    pub hidden: bool,
}

/// What the prompt shows of the character and the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptInput<'a> {
    pub hp: i32,
    pub maxhp: i32,
    pub mana: i32,
    pub maxmana: i32,
    /// The class record's `magery`; `None` when the class has no record.
    pub magery: Option<Magery>,
    /// Resting; wins over meditating.
    pub resting: bool,
    /// Meditating.
    pub meditating: bool,
    /// The statline mode.
    pub mode: StatlineMode,
    /// The custom template; `None` reads as empty.
    pub template: Option<&'a str>,
    pub palette: Palette,
    /// `Some` when the channel is a sysop with debug mode on.
    pub sysop_debug: Option<DebugInfo>,
}

/// The prompt as the player receives it: [`prf_prompt`] through the host steps
/// (`render::finish`, no wrap), so a custom `%n` arrives as `\r\n`.
pub fn render_prompt(input: &PromptInput, ctx: &StatlineCtx) -> Vec<u8> {
    finish(&prf_prompt(input, ctx), None)
}

/// The prompt as the original wrote it, before the host's steps: CP437 with `\r` as
/// written, without the `^A` terminator. `ctx` serves the custom template only.
pub fn prf_prompt(input: &PromptInput, ctx: &StatlineCtx) -> Vec<u8> {
    let palette = input.palette;
    let prompt_colour = Colour::new(ColRow::Prompt, palette).prefix();
    let mut out = Vec::with_capacity(48);
    // wipe the line, then the prompt colour.
    out.extend_from_slice(ANSI_CLRLINE);
    out.extend_from_slice(prompt_colour);

    let mark = if input.resting {
        prompt_text("resting")
    } else if input.meditating {
        prompt_text("meditating")
    } else {
        ""
    };

    if input.mode.code() > StatlineMode::Full.code() {
        // the stored template, already cut by SET STATLINE; cut again so an over-long one from
        // anywhere else cannot get past the storage rule.
        let template = cut_template(input.template.unwrap_or(""));
        cp437::encode_into(&mut out, &substitute(template, ctx));
    } else if input.mode.code() != StatlineMode::Off.code() {
        // "on" and "full" look the same here.
        if input.maxmana > 0 {
            match input.magery {
                // "[HP=n/MA=n]:", the mark after "]:".
                Some(magery) => {
                    out.extend_from_slice(prompt_text("hp_open").as_bytes());
                    out.extend_from_slice(get_hp_colour(input.hp, input.maxhp, palette));
                    out.extend_from_slice(input.hp.to_string().as_bytes());
                    out.extend_from_slice(prompt_colour);
                    // MA or KAI is decided by the class, not by the mana value.
                    let label = if magery == Magery::Kai {
                        "kai_label"
                    } else {
                        "mana_label"
                    };
                    out.extend_from_slice(prompt_text(label).as_bytes());
                    out.extend_from_slice(Colour::new(ColRow::PromptVal, palette).prefix());
                    out.extend_from_slice(input.mana.to_string().as_bytes());
                    out.extend_from_slice(prompt_colour);
                    out.extend_from_slice(prompt_text("close").as_bytes());
                    out.extend_from_slice(mark.as_bytes());
                }
                // no class record, nothing printed for the line.
                None => tracing::warn!(
                    maxmana = input.maxmana,
                    "prompt for a mana user without a class record: hit point part left out"
                ),
            }
        } else {
            // "[HP=n]:", the mark inside the brackets.
            out.extend_from_slice(prompt_text("hp_open").as_bytes());
            out.extend_from_slice(get_hp_colour(input.hp, input.maxhp, palette));
            out.extend_from_slice(input.hp.to_string().as_bytes());
            out.extend_from_slice(prompt_colour);
            out.extend_from_slice(mark.as_bytes());
            out.extend_from_slice(prompt_text("close").as_bytes());
        }

        // sysop debug mode, energy and the stealth flags.
        if let Some(debug) = input.sysop_debug {
            out.extend_from_slice(prompt_text("debug_open").as_bytes());
            let energy = prompt_text("debug_energy").replace("{energy}", &debug.energy.to_string());
            out.extend_from_slice(energy.as_bytes());
            for (on, key) in [
                (debug.sneaking, "debug_sneaking"),
                (debug.just_arrived, "debug_just_arrived"),
                (debug.hidden, "debug_hidden"),
            ] {
                if on {
                    out.extend_from_slice(prompt_text(key).as_bytes());
                }
            }
            out.extend_from_slice(prompt_text("debug_close").as_bytes());
        }
    } else {
        // statline off.
        out.extend_from_slice(prompt_text("off").as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_text() {
        assert!(!prompt_texts().is_empty());
        for (key, entry) in prompt_texts() {
            assert!(!entry.text.is_empty(), "{key}");
        }
    }
}
