//! WCCTEXT texts shown whole: `display_long_text` with its `%s` argument (a monster's
//! answer names the monster; `None` for every other caller) and `display_long_text_to_room`
//! (the `roomtext` clause).

use silvermere_text::Msg;
use silvermere_types::ids::{RoomKey, SessionId, TextBlockNo};

use crate::movement::exits::{Arg, fmt_msg};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// The parts of `textnum` in order (at most 100, until a part is missing), each
/// printf-formatted with `arg` when given, and the first part's link.
fn parts(world: &World, textnum: TextBlockNo, arg: Option<&str>) -> (Vec<String>, TextBlockNo) {
    let mut out = Vec::new();
    let mut link = TextBlockNo(0);
    for part in 0..100i16 {
        let Some(tb) = world.text_blocks.get(&(part, textnum)) else {
            break;
        };
        if link.0 == 0 {
            link = tb.link;
        }
        out.push(match arg {
            Some(a) => fmt_msg(&tb.text, &[Arg::S(a)]),
            None => tb.text.clone(),
        });
    }
    (out, link)
}

/// The text to `id`, wrapped at 79, each part used as a printf format with `arg` when
/// given; a line end is added when the last part does not end with `\r`. Returns the first
/// part's link (0 for none, or when the text does not exist).
pub fn display_long_text(
    world: &mut World,
    id: SessionId,
    textnum: TextBlockNo,
    arg: Option<&str>,
) -> TextBlockNo {
    let (parts, link) = parts(world, textnum, arg);
    let endcr = parts.last().is_some_and(|t| t.ends_with('\r'));
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::LongTextStart);
        for text in parts {
            s.output.push(Msg::LongTextPart { text });
        }
        if !endcr {
            s.output.push(Msg::LongTextEnd);
        }
    }
    link
}

/// The text to everyone in `room` (nobody excepted, blind players too), part by part; a
/// part that ends its line (`\r` or `\n`) is followed by each listener's prompt, and a last
/// part that does not is followed by a line end and the prompt. No page mode, no wrap.
/// Returns the first part's link.
pub fn display_long_text_to_room(
    world: &mut World,
    room: RoomKey,
    textnum: TextBlockNo,
    arg: Option<&str>,
) -> TextBlockNo {
    let (parts, link) = parts(world, textnum, arg);
    let mut endcr = false;
    let send = |world: &mut World, msg: Msg, prompt: bool| {
        tell_room(
            world,
            room,
            [None, None],
            Noise::Always,
            TellBlind::All,
            prompt,
            None,
            &msg,
        );
    };
    // the colour codes (ANSI_CLRLINE, ANSI_GREEN) go out with the first part
    send(world, Msg::LongTextStart, false);
    for text in parts {
        endcr = text.ends_with('\r') || text.ends_with('\n');
        send(world, Msg::LongTextRoomPart { text }, endcr);
    }
    if !endcr {
        send(world, Msg::LongTextEnd, true);
    }
    link
}
