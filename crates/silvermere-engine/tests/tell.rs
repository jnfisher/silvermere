//! The `tell_*` filters over a four-session world: two in room A, one in B,
//! one blind in A, with message levels high, medium and low.

mod common;

use silvermere_engine::output::Item;
use silvermere_engine::tell::{Noise, TellBlind, prompt_bytes, tell_game, tell_room};
use silvermere_engine::{EngineInput, EngineOutput, SessionId, World, tell_gang, tell_group};
use silvermere_text::colour::{CLRTBL, Palette};
use silvermere_text::{Msg, render};
use silvermere_types::flags::{PlayerFlags, PlayerStatus};
use silvermere_types::ids::RoomKey;
use silvermere_types::{enums::MessageLevel, ids::MapId};

const A: u32 = 2140;
const B: u32 = 2189;

struct Four {
    world: World,
    /// In A, level High, palette 1.
    high: SessionId,
    /// In A, level Medium, hidden.
    medium: SessionId,
    /// In B, level Low.
    low: SessionId,
    /// In A, level High, blind.
    blind: SessionId,
}

fn four() -> Four {
    let mut world = common::tiny_world(8);
    let high = common::enter(&mut world, "Hilda", A, MessageLevel::High);
    let medium = common::enter(&mut world, "Mel", A, MessageLevel::Medium);
    let low = common::enter(&mut world, "Lou", B, MessageLevel::Low);
    let blind = common::enter(&mut world, "Blinky", A, MessageLevel::High);
    world
        .sessions
        .get_mut(&high)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .palette = 1;
    world
        .sessions
        .get_mut(&medium)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hidden = 1;
    world
        .sessions
        .get_mut(&blind)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .status |= PlayerStatus::BLIND;
    Four {
        world,
        high,
        medium,
        low,
        blind,
    }
}

fn msg() -> Msg {
    Msg::TournamentNotice
}

fn told(world: &World, id: SessionId) -> usize {
    world.session(id).unwrap().output.items().len()
}

fn room_a() -> RoomKey {
    RoomKey::new(1, A)
}

// The noise/msglevel rule
#[test]
fn high_noise_reaches_only_high_level_listeners_in_the_room() {
    let mut f = four();
    let n = tell_room(
        &mut f.world,
        room_a(),
        [None, None],
        Noise::High,
        TellBlind::All,
        false,
        None,
        &msg(),
    );
    assert_eq!(n, 2, "Hilda and Blinky, both at MLV_HIGH in A");
    assert_eq!(told(&f.world, f.high), 1);
    assert_eq!(told(&f.world, f.blind), 1);
    assert_eq!(
        told(&f.world, f.medium),
        0,
        "MLV_MEDIUM does not get TRN_HIGH"
    );
    assert_eq!(told(&f.world, f.low), 0, "in another room");
}

// The noise/msglevel rule
#[test]
fn medium_noise_reaches_medium_and_high_but_not_low() {
    let mut f = four();
    let mut world = f.world;
    // Move Lou (low) into A to show the level alone excludes.
    world
        .sessions
        .get_mut(&f.low)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .room = silvermere_types::ids::RoomId(A);
    let n = tell_room(
        &mut world,
        room_a(),
        [None, None],
        Noise::Medium,
        TellBlind::All,
        false,
        None,
        &msg(),
    );
    assert_eq!(
        n, 2,
        "Hilda and Blinky counted; Mel is hidden and not counted"
    );
    assert_eq!(told(&world, f.medium), 1, "hidden listeners are still told");
    assert_eq!(told(&world, f.low), 0);
    f.world = world;
    let n = tell_room(
        &mut f.world,
        room_a(),
        [None, None],
        Noise::Always,
        TellBlind::All,
        false,
        None,
        &msg(),
    );
    assert_eq!(
        n, 3,
        "TRN_ALWAYS reaches every level; the hidden one is not counted"
    );
    assert_eq!(told(&f.world, f.low), 1);
}

// A blind listener hears only lines meant for the blind too
#[test]
fn sight_messages_skip_the_blind_listener() {
    let mut f = four();
    let n = tell_room(
        &mut f.world,
        room_a(),
        [None, None],
        Noise::Always,
        TellBlind::Sight,
        false,
        None,
        &msg(),
    );
    assert_eq!(n, 1, "Hilda; Mel is hidden, Blinky is blind");
    assert_eq!(told(&f.world, f.blind), 0);
    assert_eq!(told(&f.world, f.medium), 1);
}

// The two exclusions
#[test]
fn two_sessions_can_be_left_out() {
    let mut f = four();
    let n = tell_room(
        &mut f.world,
        room_a(),
        [Some(f.high), Some(f.blind)],
        Noise::Always,
        TellBlind::All,
        false,
        None,
        &msg(),
    );
    assert_eq!(
        n, 0,
        "only the hidden Mel is told, and hidden is not counted"
    );
    assert_eq!(told(&f.world, f.medium), 1);
    assert_eq!(told(&f.world, f.high), 0);
}

// The gossip cap lets a line through while the count is at most `gossmax` (sic: one too many)
#[test]
fn gossip_cap_lets_gossmax_plus_one_lines_through() {
    let mut f = four();
    for id in [f.high, f.medium, f.low, f.blind] {
        f.world
            .sessions
            .get_mut(&id)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .gossmax = 1;
    }
    f.world
        .sessions
        .get_mut(&f.low)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .flags |= PlayerFlags::NOGOSSIP;
    let counts: Vec<usize> = (0..4)
        .map(|_| {
            tell_game(
                &mut f.world,
                None,
                PlayerFlags::NOGOSSIP,
                false,
                None,
                &msg(),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [3, 3, 0, 0],
        "gossmax=1 admits two lines (the <= test), then none"
    );
    assert_eq!(told(&f.world, f.high), 2);
    assert_eq!(told(&f.world, f.low), 0, "PF_NOGOSSIP blocks it");
    assert_eq!(
        f.world.session(f.high).unwrap().player().unwrap().gosscnt,
        2,
        "the counter goes up only for delivered lines"
    );
    // The speaker can be left out.
    f.world
        .sessions
        .get_mut(&f.high)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .gosscnt = 0;
    let n = tell_game(
        &mut f.world,
        Some(f.high),
        PlayerFlags::NOGOSSIP,
        false,
        None,
        &msg(),
    );
    assert_eq!(n, 0);
    assert_eq!(told(&f.world, f.high), 2);
}

// Colour prefix then text then prompt
#[test]
fn colour_prefix_and_prompt_are_per_listener() {
    let (mut engine, _clock, rx) = common::engine(8);
    let high = common::enter(&mut engine.world, "Hilda", A, MessageLevel::High);
    let other = common::enter(&mut engine.world, "Olaf", A, MessageLevel::High);
    engine
        .world
        .sessions
        .get_mut(&high)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .palette = 1;
    engine
        .world
        .sessions
        .get_mut(&other)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hp = 3;
    let row = 2u8; // COL_PROMPT
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            let n = tell_room(
                w,
                room_a(),
                [None, None],
                Noise::Always,
                TellBlind::All,
                true,
                Some(row),
                &msg(),
            );
            assert_eq!(n, 2);
        }),
    });
    let writes: Vec<(SessionId, Vec<u8>)> = common::drain(&rx)
        .into_iter()
        .filter_map(|o| match o {
            EngineOutput::Write { id, bytes } => Some((id, bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 2);
    for (id, bytes) in writes {
        let palette = if id == high { 1 } else { 0 };
        let mut expected = CLRTBL[usize::from(row)][palette].to_vec();
        expected.extend_from_slice(&render(&msg(), Palette::new(palette as u8).unwrap()).bytes);
        let prompt = prompt_bytes(&engine.world, id);
        assert!(
            prompt.windows(4).any(|w| w == b"[HP="),
            "a prompt: {prompt:?}"
        );
        expected.extend_from_slice(&prompt);
        assert_eq!(bytes, expected, "listener {id}");
    }
    // The two listeners' prompts differ (hp 10 vs 3), so each got its own.
    assert_ne!(
        prompt_bytes(&engine.world, high),
        prompt_bytes(&engine.world, other)
    );
}

// TELL_NOCOLOR; rows above the last colour row
#[test]
fn no_colour_above_maxclr_or_for_tell_nocolor() {
    let mut f = four();
    tell_room(
        &mut f.world,
        room_a(),
        [None, None],
        Noise::Always,
        TellBlind::All,
        false,
        Some(0xFF),
        &msg(),
    );
    tell_room(
        &mut f.world,
        room_a(),
        [None, None],
        Noise::Always,
        TellBlind::All,
        false,
        Some(15),
        &msg(),
    );
    let items = f.world.session(f.high).unwrap().output.items().to_vec();
    assert!(matches!(items[0], Item::Msg { colour: None, .. }));
    assert!(matches!(
        items[1],
        Item::Msg {
            colour: Some(15),
            ..
        }
    ));
    let bytes = f
        .world
        .session(f.high)
        .unwrap()
        .output
        .render(Palette::new(1).unwrap(), b"");
    let one = render(&msg(), Palette::new(1).unwrap()).bytes;
    assert_eq!(
        bytes,
        [one.clone(), one].concat(),
        "no prefix bytes for either"
    );
}

// `tell_gang` (same gang), `tell_group` (always prompted)
#[test]
fn gang_and_group_always_prompt_and_match_case_insensitively() {
    let mut f = four();
    for id in [f.high, f.low] {
        let c = f
            .world
            .sessions
            .get_mut(&id)
            .unwrap()
            .character
            .as_mut()
            .unwrap();
        c.gang = "Rogues".into();
        c.bchannel = 7;
    }
    assert_eq!(tell_gang(&mut f.world, "ROGUES", &msg()), 2);
    assert_eq!(tell_group(&mut f.world, 7, Some(f.low), &msg()), 1);
    let items = f.world.session(f.high).unwrap().output.items().to_vec();
    assert_eq!(items.len(), 4);
    assert!(matches!(items[1], Item::Prompt(_)));
    assert!(matches!(items[3], Item::Prompt(_)));
    assert_eq!(told(&f.world, f.low), 2, "left out of the group tell");
    assert_eq!(told(&f.world, f.medium), 0);
    // Sessions that are not in the game never hear anything.
    let outside = f.world.ids.alloc().unwrap();
    f.world.sessions.insert(
        outside,
        silvermere_engine::PlayerSession::new(
            outside,
            Default::default(),
            std::time::Instant::now(),
        ),
    );
    assert_eq!(tell_gang(&mut f.world, "rogues", &msg()), 2);
    assert_eq!(told(&f.world, outside), 0);
    let _ = MapId(1);
}

/// What `id` was queued, rendered in his palette with prompts left out.
fn bytes_of(world: &World, id: SessionId) -> Vec<u8> {
    let palette = silvermere_engine::tell::palette_of(world, id);
    world.session(id).unwrap().output.render(palette, b"")
}

// `tell_game` and `tell_room`: the colour goes out on its own before `tell_user` sends the
// text buffer, so a command's still-queued ANSI_NORMAL
// comes after the listener's colour, not before it. No command reaches this today (the
// coloured social lines run after the buffer has been cleared).
#[test]
fn the_listeners_colour_goes_ahead_of_the_pending_reset() {
    const RESET: &[u8] = b"\x1b[0;37;40m";
    let row = 2u8; // COL_PROMPT
    let pending = |f: &mut Four| {
        f.world.prf_owner = Some(f.high);
        f.world
            .session_mut(f.high)
            .unwrap()
            .output
            .push(Msg::CmdNormal);
    };
    let expected = |palette: usize| {
        let mut b = CLRTBL[usize::from(row)][palette].to_vec();
        b.extend_from_slice(RESET);
        b.extend_from_slice(&render(&msg(), Palette::new(palette as u8).unwrap()).bytes);
        b
    };
    // tell_room, the player left out: Mel and Blinky get colour, reset, line; the player
    // loses the reset
    let mut f = four();
    pending(&mut f);
    tell_room(
        &mut f.world,
        room_a(),
        [Some(f.high), None],
        Noise::Always,
        TellBlind::All,
        false,
        Some(row),
        &msg(),
    );
    assert_eq!(bytes_of(&f.world, f.medium), expected(0));
    assert_eq!(bytes_of(&f.world, f.blind), expected(0));
    assert!(bytes_of(&f.world, f.high).is_empty());
    // tell_game, the player among the listeners: his own reset follows his colour too
    let mut f = four();
    pending(&mut f);
    tell_game(
        &mut f.world,
        None,
        PlayerFlags::empty(),
        false,
        Some(row),
        &msg(),
    );
    assert_eq!(bytes_of(&f.world, f.high), expected(1), "palette 1");
    for id in [f.medium, f.low, f.blind] {
        assert_eq!(bytes_of(&f.world, id), expected(0), "listener {id}");
    }
}
