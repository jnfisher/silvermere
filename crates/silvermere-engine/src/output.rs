//! `OutputBatch`: what one session will receive at the end of the current event.
//!
//! A player receives an event's messages in the order the handler produced them. The
//! engine never writes to sockets: a handler appends
//! messages here, `tell_*` appends to other sessions' batches, and the event loop renders
//! each touched batch with `silvermere-text` and sends it as one write.

use silvermere_text::colour::{Palette, tell_colour};
use silvermere_text::{Msg, render};
use silvermere_types::ids::SessionId;

use crate::tell::PromptState;
use crate::world::World;

/// The output buffer is cleared inside the command of `id` (ending a multiple-match list, a
/// non-ambiguous player lookup, using a scroll): the dispatcher's colour reset
/// (`Msg::CmdNormal`) never reaches the player when nothing was told before the clear. Text
/// the command already told stays: it had been sent. `true` when the reset was dropped.
pub fn clear_output(world: &mut World, id: SessionId) -> bool {
    world
        .session_mut(id)
        .is_some_and(|s| s.output.discard_last(&Msg::CmdNormal))
}

/// One queued item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A catalogue message, with the `tell_*` colour row (by sound and the listener's
    /// palette) sent before it, or none.
    Msg {
        msg: Msg,
        /// The row as a `clrtbl` index; `None` is `TELL_NOCOLOR`.
        colour: Option<u8>,
    },
    /// Bytes already rendered (a banner, a menu, the login dialogue's lines).
    Raw(Vec<u8>),
    /// The listener's prompt (the prompt before the send), with the state it was
    /// queued with, so each prompt shows the hit points of its own line, as the original's
    /// prompts do. `None` for a session without a
    /// character (an empty prompt).
    Prompt(Option<Box<PromptState>>),
}

/// The batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputBatch {
    items: Vec<Item>,
    /// Append the session's own prompt after everything else at flush time.
    pub reprompt: bool,
}

impl OutputBatch {
    /// Appends a message with no colour prefix.
    pub fn push(&mut self, msg: Msg) {
        self.items.push(Item::Msg { msg, colour: None });
    }

    /// Appends a message preceded by `clrtbl[row][palette]` (`None`: no prefix).
    pub fn push_coloured(&mut self, msg: Msg, colour: Option<u8>) {
        self.items.push(Item::Msg { msg, colour });
    }

    /// Appends bytes that are already rendered.
    pub fn push_raw(&mut self, bytes: impl Into<Vec<u8>>) {
        self.items.push(Item::Raw(bytes.into()));
    }

    /// Appends a prompt captured by [`PromptState::capture`]; handlers call
    /// [`PlayerSession::push_prompt`](crate::player_session::PlayerSession::push_prompt).
    pub fn push_prompt(&mut self, state: Option<PromptState>) {
        self.items.push(Item::Prompt(state.map(Box::new)));
    }

    /// Keeps the first `len` items (clearing back to a mark); captured prompts stay as
    /// they were queued.
    pub fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }

    /// Drops everything queued.
    pub fn clear(&mut self) {
        self.items.clear();
        self.reprompt = false;
    }

    /// Removes the item at `index` when it is `msg` without a colour prefix (a message
    /// that was queued and then dropped from the prf buffer); anything else stays.
    pub fn remove_msg_at(&mut self, index: usize, msg: &Msg) {
        if matches!(self.items.get(index), Some(Item::Msg { msg: m, colour: None }) if m == msg) {
            self.items.remove(index);
        }
    }

    /// Drops the last item when it is `msg` with no colour: what a [`clear_output`] (or a
    /// `tell_room` that sends the output buffer to others) takes back of a line the command had
    /// only queued so far, such as the dispatcher's `ANSI_NORMAL`. `true` when dropped.
    pub fn discard_last(&mut self, msg: &Msg) -> bool {
        let last =
            matches!(self.items.last(), Some(Item::Msg { msg: m, colour: None }) if m == msg);
        if last {
            self.items.pop();
        }
        last
    }

    /// Nothing queued and no re-prompt wanted.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && !self.reprompt
    }

    /// The queued items.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Takes the queued items, leaving the batch empty.
    pub fn take(&mut self) -> (Vec<Item>, bool) {
        let reprompt = std::mem::take(&mut self.reprompt);
        (std::mem::take(&mut self.items), reprompt)
    }

    /// Renders the batch as the bytes one write carries: each message through
    /// `silvermere-text` in `palette`, its colour prefix first, and each queued prompt as
    /// it was captured (the session's palette and class record from `world`); the
    /// session's prompt now once more at the end when `reprompt` is set.
    pub fn render_in(&self, world: &World, id: SessionId, palette: Palette) -> Vec<u8> {
        let mut out = Vec::new();
        render_items(&mut out, &self.items, palette, PromptText::Captured(world));
        if self.reprompt {
            out.extend_from_slice(&crate::tell::prompt_bytes(world, id));
        }
        out
    }

    /// Renders the batch with `marker` standing for every prompt (queued or `reprompt`):
    /// for tests that pin the order of the lines rather than the prompt text.
    pub fn render(&self, palette: Palette, marker: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        render_items(&mut out, &self.items, palette, PromptText::Marker(marker));
        if self.reprompt {
            out.extend_from_slice(marker);
        }
        out
    }
}

/// How [`render_items`] writes a queued prompt.
#[derive(Debug, Clone, Copy)]
pub enum PromptText<'a> {
    /// As captured when it was queued; `world` supplies the class record.
    Captured(&'a World),
    /// These bytes for every prompt.
    Marker(&'a [u8]),
}

/// Renders `items` into `out` (see [`OutputBatch::render_in`]).
pub fn render_items(out: &mut Vec<u8>, items: &[Item], palette: Palette, prompt: PromptText<'_>) {
    for item in items {
        match item {
            Item::Msg { msg, colour } => {
                if let Some(row) = colour {
                    out.extend_from_slice(tell_colour(*row, palette));
                }
                out.extend_from_slice(&render(msg, palette).bytes);
            }
            Item::Raw(bytes) => out.extend_from_slice(bytes),
            Item::Prompt(state) => match prompt {
                PromptText::Captured(world) => {
                    if let Some(state) = state {
                        out.extend_from_slice(&state.render(world));
                    }
                }
                PromptText::Marker(marker) => out.extend_from_slice(marker),
            },
        }
    }
}
