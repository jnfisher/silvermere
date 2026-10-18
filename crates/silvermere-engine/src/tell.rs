//! The `tell_*` family: fan-out of one message to a filtered set of sessions.
//!
//! Each function picks its listeners as the original does for that kind of message, in
//! channel order, appending to each listener's [`OutputBatch`] the colour prefix, the
//! message and the prompt. The message is a `Msg` value here; nothing is formatted twice.

use silvermere_text::colour::{Palette, TELL_NOCOLOR};
use silvermere_text::prompt::{DebugInfo, PromptInput};
use silvermere_text::{Msg, StatlineCtx, render_prompt};
use silvermere_types::enums::{DebugLevel, MessageLevel, trb, trn};
use silvermere_types::flags::{PlayerFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};

use crate::output::Item;
use crate::world::World;

/// `tell_room` noise: how minor a room message is (`TRN_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Noise {
    /// `TRN_ALWAYS`: every listener gets it.
    Always,
    /// `TRN_MEDIUM`: not for players at `MLV_LOW`.
    Medium,
    /// `TRN_HIGH`: only for players at `MLV_HIGH`.
    High,
}

impl Noise {
    /// The `TRN_` value.
    pub const fn code(self) -> u16 {
        match self {
            Self::Always => trn::ALWAYS,
            Self::Medium => trn::MEDIUM,
            Self::High => trn::HIGH,
        }
    }
}

/// `tell_room` blindness filter (`TRB_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TellBlind {
    /// `TRB_SIGHT`: the message describes something seen; `ST_BLIND` listeners skip it.
    Sight,
    /// `TRB_ALL`: blinded players get it too.
    All,
}

impl TellBlind {
    /// The `TRB_` value.
    pub const fn code(self) -> u16 {
        match self {
            Self::Sight => trb::SIGHT,
            Self::All => trb::ALL,
        }
    }
}

/// The `sound` argument: a `clrtbl` row, or `None` for `TELL_NOCOLOR`.
pub type Sound = Option<u8>;

/// The message goes to one session, with no
/// prompt and no colour. The split-screen and client/server paths are not carried.
///
/// There is one output buffer: told to another player inside a command, the line takes
/// the command's still-queued `ANSI_NORMAL` along ([`World::prf_owner`]).
pub fn tell_user(world: &mut World, id: SessionId, msg: Msg) {
    let pending = pending_buffer_owner(world);
    if let Some(owner) = pending.filter(|&o| o != id) {
        clear_pending(world, owner);
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::CmdNormal);
        }
    }
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// The player of the running command when the dispatcher's `ANSI_NORMAL` is still the last
/// thing queued for him: nothing has been told yet, so it is still in the output buffer.
/// [`tell_room`], [`tell_group`], [`tell_gang`], [`tell_game`] and [`tell_user`] to another player
/// send the whole buffer to each listener and empty it: the listeners get the reset ahead
/// of the line and the player's own later line comes without it.
fn pending_buffer_owner(world: &World) -> Option<SessionId> {
    let owner = world.prf_owner?;
    let last = world.session(owner)?.output.items().last();
    matches!(
        last,
        Some(Item::Msg {
            msg: Msg::CmdNormal,
            colour: None
        })
    )
    .then_some(owner)
}

/// The buffer is cleared at the end of a `tell_*` (the buffer went to the listeners).
fn clear_pending(world: &mut World, owner: SessionId) {
    if let Some(s) = world.session_mut(owner) {
        s.output.discard_last(&Msg::CmdNormal);
    }
}

/// One listener of a fan-out: whether the pending reset leads his line. The player's own,
/// when he is a listener, is taken back so that [`deliver`] puts it after his colour.
fn lead_pending(world: &mut World, pending: Option<SessionId>, id: SessionId) -> bool {
    if pending == Some(id) {
        clear_pending(world, id);
    }
    pending.is_some()
}

/// The end of a fan-out: the buffer is cleared, so a player who was not among the
/// listeners loses the reset.
fn end_pending(world: &mut World, pending: Option<SessionId>, told_owner: bool) {
    if let Some(owner) = pending.filter(|_| !told_owner) {
        clear_pending(world, owner);
    }
}

/// Every player in the game standing in `room`, except up to two sessions, who can see it (or
/// `blind` is `All`) and whose message level admits `noise`: noise at or below ALWAYS, or at or
/// below MEDIUM when the listener's level is MEDIUM or lower, or at or below HIGH when the
/// listener's level is HIGH or lower. With `prompt`, each listener's prompt follows the message;
/// `sound` picks the colour prefix in the listener's palette.
/// Returns the listeners told who are not hidden: hidden listeners
/// receive the text but are not counted.
// Eight arguments; the 188 call sites keep this order.
#[allow(clippy::too_many_arguments)]
pub fn tell_room(
    world: &mut World,
    room: RoomKey,
    except: [Option<SessionId>; 2],
    noise: Noise,
    blind: TellBlind,
    prompt: bool,
    sound: Sound,
    msg: &Msg,
) -> usize {
    tell_room_led(world, room, except, noise, blind, prompt, sound, None, msg)
}

/// [`tell_room`] for a command that left text in the output buffer (the dispatcher's
/// `ANSI_NORMAL`) before the call: `tell_room` sends the whole buffer, so each
/// listener gets `lead` ahead of `msg`.
#[allow(clippy::too_many_arguments)]
pub fn tell_room_led(
    world: &mut World,
    room: RoomKey,
    except: [Option<SessionId>; 2],
    noise: Noise,
    blind: TellBlind,
    prompt: bool,
    sound: Sound,
    lead: Option<&Msg>,
    msg: &Msg,
) -> usize {
    let mut cnt = 0;
    let pending = pending_buffer_owner(world);
    let mut told_owner = false;
    for id in world.session_ids() {
        if except.contains(&Some(id)) || !world.is_in_game(id) {
            continue;
        }
        let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
            continue;
        };
        let here = plr.room == room.room && plr.map == room.map;
        let sees = blind == TellBlind::All || !plr.status.contains(PlayerStatus::BLIND);
        let n = noise.code();
        let lvl = plr.msglevel.code();
        // `noise <= TRN_ALWAYS` with TRN_ALWAYS 0 is equality.
        let loud = n == trn::ALWAYS
            || (n <= trn::MEDIUM && lvl <= MessageLevel::Medium.code())
            || (n <= trn::HIGH && lvl <= MessageLevel::High.code());
        if !(here && sees && loud) {
            continue;
        }
        let hidden = plr.hidden != 0;
        told_owner |= pending == Some(id);
        let reset = lead_pending(world, pending, id);
        deliver(world, id, reset, lead, msg, prompt, sound);
        if !hidden {
            cnt += 1;
        }
    }
    end_pending(world, pending, told_owner);
    cnt
}

/// Every player in the game but `except` whose
/// `player.flags` do not include `mask`. For `PF_NOGOSSIP` and `PF_NOAUCTION` each
/// listener also has a per-period cap: the line is delivered and counted while
/// `gosscnt <= gossmax` (`auctcnt <= auctmax`); the test is `<=`, so a listener hears
/// `gossmax + 1` lines a period (sic). Returns the listeners told.
pub fn tell_game(
    world: &mut World,
    except: Option<SessionId>,
    mask: PlayerFlags,
    prompt: bool,
    sound: Sound,
    msg: &Msg,
) -> usize {
    let mut cnt = 0;
    let pending = pending_buffer_owner(world);
    let mut told_owner = false;
    for id in world.session_ids() {
        if except == Some(id) || !world.is_in_game(id) {
            continue;
        }
        let Some(plr) = world
            .sessions
            .get_mut(&id)
            .and_then(|s| s.character.as_mut())
        else {
            continue;
        };
        if plr.flags.intersects(mask) {
            continue;
        }
        let allowed = if mask == PlayerFlags::NOGOSSIP {
            if plr.gosscnt <= plr.gossmax {
                plr.gosscnt = plr.gosscnt.wrapping_add(1);
                true
            } else {
                false
            }
        } else if mask == PlayerFlags::NOAUCTION {
            if plr.auctcnt <= plr.auctmax {
                plr.auctcnt = plr.auctcnt.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            true
        };
        if allowed {
            told_owner |= pending == Some(id);
            let reset = lead_pending(world, pending, id);
            deliver(world, id, reset, None, msg, prompt, sound);
            cnt += 1;
        }
    }
    end_pending(world, pending, told_owner);
    cnt
}

/// Every player in the game whose `player.gang` equals `gang` without regard to case
/// (`eq_ignore_case`), the speaker included; each gets the message followed by a fresh prompt
/// (always).
pub fn tell_gang(world: &mut World, gang: &str, msg: &Msg) -> usize {
    let mut cnt = 0;
    let pending = pending_buffer_owner(world);
    let mut told_owner = false;
    for id in world.session_ids() {
        if !world.is_in_game(id) {
            continue;
        }
        let same = world
            .sessions
            .get(&id)
            .and_then(|s| s.player())
            .is_some_and(|p| p.gang.eq_ignore_ascii_case(gang));
        if same {
            told_owner |= pending == Some(id);
            let reset = lead_pending(world, pending, id);
            deliver(world, id, reset, None, msg, true, None);
            cnt += 1;
        }
    }
    end_pending(world, pending, told_owner);
    cnt
}

/// Every player in the game but `except` tuned to broadcast channel `channel`
/// (`player.bchannel`); each gets the message followed by a fresh prompt (always).
pub fn tell_group(world: &mut World, channel: i16, except: Option<SessionId>, msg: &Msg) -> usize {
    let mut cnt = 0;
    let pending = pending_buffer_owner(world);
    let mut told_owner = false;
    for id in world.session_ids() {
        if except == Some(id) || !world.is_in_game(id) {
            continue;
        }
        let tuned = world
            .sessions
            .get(&id)
            .and_then(|s| s.player())
            .is_some_and(|p| p.bchannel == channel);
        if tuned {
            told_owner |= pending == Some(id);
            let reset = lead_pending(world, pending, id);
            deliver(world, id, reset, None, msg, true, None);
            cnt += 1;
        }
    }
    end_pending(world, pending, told_owner);
    cnt
}

/// One listener's share (the bytes one listener receives): the colour first; then the
/// pending reset (`reset`) and `lead` when there are any, the message and the
/// prompt if asked.
fn deliver(
    world: &mut World,
    id: SessionId,
    reset: bool,
    lead: Option<&Msg>,
    msg: &Msg,
    prompt: bool,
    sound: Sound,
) {
    let mut colour = sound.filter(|&row| row != TELL_NOCOLOR);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    for m in reset.then_some(&Msg::CmdNormal).into_iter().chain(lead) {
        s.output.push_coloured(m.clone(), colour.take());
    }
    s.output.push_coloured(msg.clone(), colour);
    if prompt {
        s.push_prompt();
    }
}

/// The listener's palette: `player.palette`, repaired to 0 when out of range; palette 0
/// for a session with no character.
pub fn palette_of(world: &World, id: SessionId) -> Palette {
    world
        .sessions
        .get(&id)
        .and_then(|s| s.player())
        .map(|p| Palette::repaired(p.palette))
        .unwrap_or_default()
}

/// The listener's prompt as `prf_prompt(i)` produced it (T-020's renderer), or nothing
/// for a session without a character.
pub fn prompt_bytes(world: &World, id: SessionId) -> Vec<u8> {
    world
        .sessions
        .get(&id)
        .and_then(PromptState::capture)
        .map(|p| p.render(world))
        .unwrap_or_default()
}

/// What `prf_prompt` reads from the character and the channel, taken when the prompt is
/// queued, so two prompts in one batch show the hit points of their own moments, as the
/// original's do. The class's magery is
/// looked up when it is rendered (classes do not
/// change mid-event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptState {
    hp: i32,
    maxhp: i32,
    mana: i32,
    maxmana: i32,
    class: silvermere_types::ids::ClassNo,
    resting: bool,
    meditating: bool,
    mode: silvermere_types::enums::StatlineMode,
    template: String,
    palette: Palette,
    sysop_debug: Option<DebugInfo>,
    exp: u32,
    first_name: String,
    evil_warn: bool,
    sex: u8,
    title: String,
}

impl PromptState {
    /// The session's prompt state now; `None` without a character.
    pub fn capture(s: &crate::player_session::PlayerSession) -> Option<Self> {
        let plr = s.player()?;
        Some(Self {
            hp: i32::from(plr.hp),
            maxhp: i32::from(plr.maxhp),
            mana: i32::from(plr.mana),
            maxmana: i32::from(plr.maxmana),
            class: plr.class,
            resting: plr.status.contains(PlayerStatus::RESTING),
            meditating: plr.status2.contains(PlayerStatus2::MEDITATING),
            mode: plr.statline,
            template: s.ust.statline.clone(),
            palette: Palette::repaired(plr.palette),
            sysop_debug: (s.flags.sysop && s.flags.debug != DebugLevel::Off).then_some(DebugInfo {
                energy: i32::from(plr.energy),
                sneaking: plr.status.contains(PlayerStatus::SNEAKING),
                just_arrived: plr.status.contains(PlayerStatus::JUSTARRIVED),
                hidden: plr.hidden != 0,
            }),
            exp: plr.exp,
            first_name: plr.firstname.clone(),
            evil_warn: plr
                .settings
                .contains(silvermere_types::flags::PlayerSettings::EVILWARN),
            sex: plr.sex,
            title: plr.title.clone(),
        })
    }

    /// The prompt bytes (`prf_prompt`), with the class record from `world`.
    pub fn render(&self, world: &World) -> Vec<u8> {
        let input = PromptInput {
            hp: self.hp,
            maxhp: self.maxhp,
            mana: self.mana,
            maxmana: self.maxmana,
            magery: world.classes.get(&self.class).map(|c| c.magery),
            resting: self.resting,
            meditating: self.meditating,
            mode: self.mode,
            template: Some(self.template.as_str()),
            palette: self.palette,
            sysop_debug: self.sysop_debug,
        };
        let (possessive, object) = pronouns(self.sex);
        let ctx = StatlineCtx {
            hp: self.hp,
            maxhp: self.maxhp,
            mana: self.mana,
            maxmana: self.maxmana,
            exp: self.exp,
            // The experience needed is T-016's; until it lands `%X` prints a bare `X`.
            exp_needed: None,
            // The currency total is T-030's; the custom statline's `%c` shows 0 until then.
            wealth_copper: 0,
            first_name: &self.first_name,
            evil_warn: self.evil_warn,
            resting: self.resting,
            meditating: self.meditating,
            pronoun_possessive: possessive,
            pronoun_object: object,
            title: (!self.title.is_empty()).then_some(self.title.as_str()),
            palette: self.palette,
        };
        render_prompt(&input, &ctx)
    }
}

/// The possessive and object pronouns by `player.sex`: `'F'` her/her, `'M'` his/him,
/// anything else its/it.
pub(crate) fn pronouns(sex: u8) -> (&'static str, &'static str) {
    match sex {
        b'F' => ("her", "her"),
        b'M' => ("his", "him"),
        _ => ("its", "it"),
    }
}
