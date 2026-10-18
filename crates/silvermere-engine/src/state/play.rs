//! `Playing`: `execute_input` and the command pipeline.
//!
//! T-021 brought in the three stages the login flow needs (the exit wait, the empty line,
//! `x`); T-024 brings the whole of `execute_input` in `commands::input`, which this
//! state hands every line to.

use crate::commands::input::execute_input;
use crate::state::{Input, Outcome};
use crate::world::World;

/// One input line in a state this module owns: the line typed now (`queued = 0`).
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    execute_input(world, input.id, input.line, false)
}
