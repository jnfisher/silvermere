//! `MainMenu` and `Help`: the `[MAJORMUD]:` menu and the help topic prompt.

use silvermere_text::Msg;
use silvermere_types::enums::SessionState;
use silvermere_types::ids::SessionId;

use crate::commands::parse::eq_ignore_case;
use crate::help::help_text;
use crate::state::login::{enter, key, leave, show_menu};
use crate::state::newchar::race_change;
use crate::state::{Input, Outcome};
use crate::world::World;

/// One input line in a state this module owns.
pub fn on_line(world: &mut World, input: Input<'_>, today: i64) -> Outcome {
    match world.session(input.id).map(|s| s.state) {
        Some(SessionState::Help) => {
            help_line(world, input);
            Outcome::Continue
        }
        Some(_) => main_menu_line(world, input, today),
        None => Outcome::Logoff,
    }
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// `MainMenu`: only the first character of the first word counts, in either case.
///
/// | Key | Here |
/// |---|---|
/// | `P`, `E` | [`enter::enter_realm`] |
/// | `X` | [`leave::leave_module`] |
/// | `H` | the help topic list and prompt, then `Help` |
/// | `T`, `G`, `S`/`W` | the lists (T-027), then `\r[MAJORMUD]: ` |
/// | `R` | the release notes (`WCCMMUD.NOT`), then `\r[MAJORMUD]: ` |
/// | `I`, `C`, `A` | the screens (T-027), then `[MAJORMUD]: ` |
/// | empty line | the menu again |
/// | anything else | `\rInvalid Option.` and `\r[MAJORMUD]: ` |
///
/// The hidden words, in the original's order: `sysop` opens the sysop menu for a sysop
/// (T-055); `changerace` starts the race change while `can_user_change_race` allows it
/// ([`race_change::start`]); otherwise it is the `C` it starts with. Not carried: `V`
/// vacation, the add-on command hooks.
fn main_menu_line(world: &mut World, input: Input<'_>, today: i64) -> Outcome {
    let id = input.id;
    let first = input.line.split_whitespace().next().unwrap_or("");
    // `sysop` (T-055)
    if eq_ignore_case(first, "sysop") && world.session(id).is_some_and(|s| s.flags.sysop) {
        crate::state::sysop_menu::open(world, id);
        return Outcome::Continue;
    }
    if eq_ignore_case(first, "changerace") && race_change::start(world, id) {
        return Outcome::Continue;
    }
    match key(input.line) {
        Some('P' | 'E') => enter::enter_realm(world, id, today),
        Some('X') => return leave::leave_module(world, id),
        Some('H') => {
            push(world, id, Msg::AnsiNormal);
            help_text(world, id, None, 1, "");
            push(world, id, Msg::HelpPrompt);
            if let Some(s) = world.session_mut(id) {
                s.state = SessionState::Help;
            }
        }
        Some('T') => {
            // the TOPTEN list, then the menu prompt
            crate::display::topten::display_only_topten(world, id);
            push(world, id, Msg::MudMenuPromptCr);
        }
        Some('G') => {
            crate::display::topten::display_top_gangs(world, id, 10);
            push(world, id, Msg::MudMenuPromptCr);
        }
        Some('S' | 'W') => {
            crate::display::who::display_scan_of_users(world, id);
            push(world, id, Msg::MudMenuPromptCr);
        }
        Some('R') => release_notes(world, id),
        Some(c @ ('I' | 'C' | 'A')) => {
            // the ANSI screen or the plain one, then the bare prompt
            let ansi = world.session(id).is_none_or(|s| s.caps.ansi);
            let screen = match (c, ansi) {
                ('I', true) => Msg::MenuInfoAnsi,
                ('I', false) => Msg::MenuInfoPlain,
                ('C', true) => Msg::MenuCountsAnsi,
                ('C', false) => Msg::MenuCountsPlain,
                (_, true) => Msg::MenuAboutAnsi,
                (_, false) => Msg::MenuAboutPlain,
            };
            push(world, id, screen);
            push(world, id, Msg::MudMenuPromptNormal);
        }
        None => show_menu(world, id),
        Some(_) => {
            push(world, id, Msg::MenuInvalidOption);
            push(world, id, Msg::MudMenuPromptCr);
        }
    }
    Outcome::Continue
}

/// `Help`: `X` back to the menu; an empty line or `?` the topic list; anything else (or
/// `HELP topic`) one topic; then the prompt.
fn help_line(world: &mut World, input: Input<'_>) {
    let id = input.id;
    let words: Vec<&str> = input.line.split_whitespace().collect();
    if key(input.line) == Some('X') {
        show_menu(world, id);
        if let Some(s) = world.session_mut(id) {
            s.state = SessionState::MainMenu;
        }
        return;
    }
    push(world, id, Msg::AnsiNormal);
    let class_word = words.get(2).copied().unwrap_or("");
    if words.is_empty() || (words.len() == 1 && words[0].starts_with('?')) {
        help_text(world, id, None, words.len(), class_word);
    } else if words.len() > 1 && words[0].eq_ignore_ascii_case("help") {
        let topic = input.line.trim_start()[words[0].len()..].trim_start();
        help_text(world, id, Some(topic), words.len(), class_word);
    } else {
        help_text(world, id, Some(input.line), words.len(), class_word);
    }
    push(world, id, Msg::HelpPrompt);
}

/// `R`: the release notes `WCCMMUD.NOT` as the host's paged listing, then
/// `\r[MAJORMUD]: `. The file is read from the assets directory; without it only the
/// prompt is shown. The page mode of the listing is the host's and is not carried.
fn release_notes(world: &mut World, id: SessionId) {
    let notes = world
        .display
        .assets
        .get("WCCMMUD.NOT")
        .map(silvermere_text::render::normalise_line_ends);
    if let Some(s) = world.session_mut(id) {
        if let Some(bytes) = notes {
            s.output.push_raw(bytes);
        }
        s.output.push(Msg::MudMenuPromptCr);
    }
}
