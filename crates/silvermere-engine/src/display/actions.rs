//! The ACTION verb list.

use silvermere_text::Msg;

/// The ACTION list line: every verb followed by one space, wrapped at 79 columns by the
/// catalogue entry. `verbs` is `World::action_order`: WCCACTS in physical record order (as
/// Btrieve Step First/Next reads it), so the stock list starts
/// `hug kiss wave grin bow bleed nod laugh shake`, not alphabetically.
pub fn list_of_actions(verbs: &[String]) -> Msg {
    let mut line = String::new();
    for v in verbs {
        line.push_str(v);
        line.push(' ');
    }
    Msg::ActionList { verbs: line }
}
