//! The exits: the live parameters over the content, `exit_is_available`, the lock states
//! and the exit-type gate of `move_user`, one function per type.
//!
//! A door opened, a hidden exit found or a timed exit swapped changes an exit's
//! `exitpar1s`, `exitpar2s` or `exitpar3s`. Here the content `Exit` is never touched:
//! [`ExitOverride`] in `RoomState` holds the live value of each parameter and [`live_exit`]
//! lays it over the content, so a reload of the content keeps the doors as they stand.

use silvermere_rules::economy;
use silvermere_text::Msg;
use silvermere_text::ansi;
use silvermere_types::enums::{Currency, Direction, ExitType, RoomType};
use silvermere_types::flags::{ExitFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ItemNo, MapId, MessageNo, RoomKey, SessionId};
use silvermere_types::records::{Exit, Room};
use silvermere_types::sizes::NDIRS;

use super::move_user::{clear_search_flags, set_user_location, user_allowed_in_room};
use super::{HOOKS, dirname};
use crate::display::{RoomMode, display_room};
use crate::scheduler::Rng;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// A door, gate or key exit that can be walked through.
pub const LOCK_OPEN: u32 = 0;
/// Shut but not locked (also "unlocked" after PICKLOCK).
pub const LOCK_CLOSED: u32 = 1;
/// Shut and locked.
pub const LOCK_LOCKED: u32 = 2;

/// The trap kinds in a Trap or Spell Trap exit's second parameter.
pub mod trap {
    /// Armed; a failed disarm hurts.
    pub const HURT: i16 = 0;
    /// Disarmed (was `HURT`).
    pub const HURTOFF: i16 = 1;
    /// Not among the visible exits; springs only when walked into on purpose.
    pub const HIDDEN: i16 = 2;
    /// Armed; a failed disarm throws the player through.
    pub const THROW: i16 = 3;
    /// Disarmed (was `THROW`).
    pub const THROWOFF: i16 = 4;
}

/// The live values of an exit's parameters where they differ from the content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExitOverride {
    /// `exitpar1s`: a Door's or Gate's lock state, a Hidden exit's flags.
    pub par1: Option<u32>,
    /// `exitpar2s`: a Key exit's lock state, a trap's kind.
    pub par2: Option<i16>,
    /// `exitpar3s`: a Timed exit's phase (0 open).
    pub par3: Option<i32>,
}

impl ExitOverride {
    /// The content exit with the live values laid over it.
    pub fn apply(self, mut exit: Exit) -> Exit {
        if let Some(p) = self.par1 {
            exit.par1 = p;
        }
        if let Some(p) = self.par2 {
            exit.par2 = p;
        }
        if let Some(p) = self.par3 {
            exit.par3 = p;
        }
        exit
    }
}

/// The exit `dir` of `room` as it stands now (the content exit with the live parameters),
/// or `None` for a room not in the content.
pub fn live_exit(world: &World, room: RoomKey, dir: Direction) -> Option<Exit> {
    let d = usize::from(dir.code());
    let content = world.room(room)?.exits[d];
    Some(
        world
            .room_state
            .get(&room)
            .map_or(content, |s| s.exit_overrides[d].apply(content)),
    )
}

/// All ten exits of `room` as they stand now.
pub fn live_exits(world: &World, room: RoomKey) -> Option<[Exit; NDIRS]> {
    let content = world.room(room)?.exits;
    let Some(state) = world.room_state.get(&room) else {
        return Some(content);
    };
    Some(std::array::from_fn(|d| {
        state.exit_overrides[d].apply(content[d])
    }))
}

/// The room record with its exits as they stand now: what every display reads.
pub fn live_room(world: &World, room: RoomKey) -> Option<Room> {
    let mut rm = world.room(room)?.clone();
    if let Some(exits) = live_exits(world, room) {
        rm.exits = exits;
    }
    Some(rm)
}

/// Sets the live `exitpar1s` of the exit and marks the room changed.
pub fn set_par1(world: &mut World, room: RoomKey, dir: Direction, v: u32) {
    if let Some(s) = world.room_state.get_mut(&room) {
        s.exit_overrides[usize::from(dir.code())].par1 = Some(v);
    }
}

/// Sets the live `exitpar2s` of the exit and marks the room changed.
pub fn set_par2(world: &mut World, room: RoomKey, dir: Direction, v: i16) {
    if let Some(s) = world.room_state.get_mut(&room) {
        s.exit_overrides[usize::from(dir.code())].par2 = Some(v);
    }
}

/// Sets the live `exitpar3s` of the exit and marks the room changed.
pub fn set_par3(world: &mut World, room: RoomKey, dir: Direction, v: i32) {
    if let Some(s) = world.room_state.get_mut(&room) {
        s.exit_overrides[usize::from(dir.code())].par3 = Some(v);
    }
}

/// `"door"` or `"gate"` for the messages: everything that is not a Gate is a door, as the
/// original has it.
pub fn door_word(kind: ExitType) -> &'static str {
    if kind == ExitType::Gate {
        "gate"
    } else {
        "door"
    }
}

/// Is there a way out that way for the player standing in `room`? Spell, Text and Remote
/// exits never count; a Hidden one counts once passable, found or visible-but-blocked; a
/// Trap counts unless its kind is 2; everything else counts.
pub fn exit_is_available(world: &World, room: RoomKey, dir: Direction) -> bool {
    let Some(exit) = live_exit(world, room, dir) else {
        return false;
    };
    if exit.target.0 == 0 {
        return false;
    }
    match exit.kind {
        ExitType::Spell | ExitType::Text | ExitType::RemoteAction => false,
        ExitType::Hidden => hidden_passable(exit.par1),
        ExitType::Trap => matches!(exit.par2, 0 | 1 | 3 | 4),
        _ => true,
    }
}

/// A Hidden exit that is passable, found or visible-but-blocked.
pub fn hidden_passable(par1: u32) -> bool {
    ExitFlags::from_bits_retain(par1)
        .intersects(ExitFlags::PASSABLE | ExitFlags::VISSEARCHED | ExitFlags::VISBLOCKED)
}

/// The exit types that refuse a dragged player.
pub fn exit_allows_drag(kind: ExitType) -> bool {
    !matches!(
        kind,
        ExitType::Spell
            | ExitType::Key
            | ExitType::Item
            | ExitType::Toll
            | ExitType::Class
            | ExitType::Race
            | ExitType::Level
            | ExitType::Alignment
    )
}

/// One format argument of a WCCMSG text.
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    /// `%s`.
    S(&'a str),
    /// `%d`, `%ld`, `%u`.
    D(i64),
}

/// A WCCMSG text filled in: each `%s`, `%d`, `%ld`, `%u` or `%lu` takes the next argument
/// in order, `%%` is a percent sign; anything else is left as written, and a conversion
/// past the last argument prints nothing.
pub fn fmt_msg(fmt: &str, args: &[Arg<'_>]) -> String {
    let mut out = String::with_capacity(fmt.len() + 32);
    let mut next = 0;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&p) = chars.peek() {
            if p == 'l' || p == 'h' {
                spec.push(p);
                chars.next();
            } else {
                break;
            }
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some(conv @ ('s' | 'd' | 'u' | 'i')) => {
                chars.next();
                if let Some(a) = args.get(next) {
                    match a {
                        Arg::S(s) => out.push_str(s),
                        Arg::D(d) => out.push_str(&d.to_string()),
                    }
                    next += 1;
                } else if conv == 's' {
                    // nothing
                }
            }
            _ => {
                out.push('%');
                out.push_str(&spec);
            }
        }
    }
    out
}

fn ansi_str(a: &'static [u8]) -> &'static str {
    std::str::from_utf8(a).unwrap_or("")
}

/// The name in green.
pub fn green_name(name: &str) -> String {
    format!("{}{name}", ansi_str(ansi::ANSI_GREEN))
}

/// The name in yellow, then back to green.
pub fn yellow_name(name: &str) -> String {
    format!(
        "{}{name}{}",
        ansi_str(ansi::ANSI_YELLOW),
        ansi_str(ansi::ANSI_GREEN)
    )
}

/// To everyone in the room but `except`, blind players too, with a prompt: how every exit
/// text goes out.
pub fn tell_room_all(world: &mut World, room: RoomKey, except: [Option<SessionId>; 2], msg: Msg) {
    tell_room(
        world,
        room,
        except,
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &msg,
    );
}

/// What the player's room, exit and move look like to the gate.
#[derive(Debug, Clone)]
pub struct GateCtx {
    /// The player.
    pub id: SessionId,
    /// His name.
    pub name: String,
    /// The room he stands in.
    pub room: RoomKey,
    /// Its type (for `user_allowed_in_room`).
    pub room_type: RoomType,
    /// The exit taken.
    pub dir: Direction,
    /// The exit as it stands now.
    pub exit: Exit,
    /// The `how` of `move_user`.
    pub how: i32,
    /// The player being dragged.
    pub drag: Option<SessionId>,
    /// The drag is still on (the victim was down and light enough).
    pub movedrag: bool,
    /// `" (Dragging NAME)"` or empty.
    pub drag_suffix: String,
    /// Sneaking and not noticed.
    pub unseen: bool,
}

/// What the gate decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// The way is open: on to `user_allowed_in_room` and the move, into `newmap`.
    Pass {
        /// The map of the room he arrives in (a Map Change exit's `exitpar1s`).
        newmap: MapId,
        /// The toll was paid, which gives a sneaker away.
        seen: bool,
    },
    /// Refused; the player has been told. The move's ending follows.
    Refused,
    /// The exit moved the player itself and showed him the room (Item, Action, Text, Trap,
    /// Spell Trap). The move's ending follows, except after an Item exit.
    Carried {
        /// No Cast epilogue and no prompt (the Item exit).
        out: bool,
    },
    /// Nothing more happens and nothing was said (a trap kind that does nothing, a Cast
    /// exit that felled the player, an Ability refusal without a text). The move's ending
    /// follows.
    Stay,
}

/// The exit-type check of `move_user`. Each type is its own function below.
pub fn exit_gate(world: &mut World, c: &GateCtx) -> Gate {
    match c.exit.kind {
        ExitType::Spell => spell(world, c),
        ExitType::Key => key(world, c),
        ExitType::Item => item(world, c),
        ExitType::Toll => toll(world, c),
        ExitType::Action => action(world, c),
        ExitType::Gate => gate_or_door(world, c, Msg::GateClosedLocked, Msg::GateClosed),
        ExitType::Door => gate_or_door(world, c, Msg::DoorClosedLocked, Msg::DoorClosed),
        ExitType::Text => text(world, c),
        ExitType::Trap => trap_exit(world, c),
        ExitType::SpellTrap => spell_trap(world, c),
        ExitType::Hidden => hidden(world, c),
        ExitType::RemoteAction => remote(world, c),
        ExitType::Class => class_or_race(world, c, true),
        ExitType::Race => class_or_race(world, c, false),
        ExitType::Level => level(world, c),
        ExitType::Alignment => alignment(world, c),
        ExitType::MapChange => Gate::Pass {
            newmap: MapId(c.exit.par1),
            seen: false,
        },
        ExitType::Timed => timed(world, c),
        ExitType::Cast => cast(world, c),
        ExitType::Ability => ability(world, c),
        // Normal, and Ticket, User Count, Block Guard and Delay (17-19, 21), which have no
        // case and pass like ordinary exits.
        ExitType::Normal
        | ExitType::Ticket
        | ExitType::UserCount
        | ExitType::BlockGuard
        | ExitType::Delay
        | ExitType::Unknown(_) => pass(c),
    }
}

fn pass(c: &GateCtx) -> Gate {
    Gate::Pass {
        newmap: c.room.map,
        seen: false,
    }
}

fn refuse(world: &mut World, id: SessionId, msg: Msg) -> Gate {
    tell_user(world, id, msg);
    Gate::Refused
}

/// A dragging player is refused: "You may not drag anyone through this exit.".
fn no_drag(world: &mut World, c: &GateCtx) -> Option<Gate> {
    c.drag.map(|_| refuse(world, c.id, Msg::ExitNoDrag))
}

/// Refused during tournament play (the same check for several exit types).
fn tournament(world: &mut World, c: &GateCtx) -> Option<Gate> {
    world
        .config
        .tournament_play
        .then(|| refuse(world, c.id, Msg::ExitForbiddenTournament))
}

/// The message record `n`, if any.
fn message(world: &World, n: i32) -> Option<[String; 3]> {
    world.messages.get(&MessageNo(n)).map(|m| m.msgs.clone())
}

/// A Spell exit.
fn spell(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    if c.how != 3 && c.how != 4 {
        return refuse(world, c.id, Msg::ExitNeedSpell);
    }
    pass(c)
}

/// A Key exit: `exitpar2s` is the lock; only a bash (how 5) gets through a shut one.
fn key(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    if c.exit.par2 != 0 && c.how != 5 {
        return refuse(world, c.id, Msg::ExitKeyClosed);
    }
    pass(c)
}

/// An Item exit: the item `exitpar1s` carried moves the player through with message
/// `exitpar3s` and spends one use; otherwise message `exitpar2s` or the standard refusal.
fn item(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    let wanted = ItemNo(c.exit.par1 as i32);
    let slot = world
        .session(c.id)
        .and_then(|s| s.player())
        .and_then(|p| p.items.iter().position(|&i| i == wanted && i.0 != 0));
    if let Some(slot) = slot {
        let msg = message(world, c.exit.par3);
        let target = RoomKey {
            map: c.room.map,
            room: c.exit.target,
        };
        if let Some(m) = &msg {
            if !m[1].is_empty() {
                let text = fmt_msg(
                    &m[1],
                    &[Arg::S(&green_name(&c.name)), Arg::S(&dirname(world, c.dir))],
                );
                tell_room_all(
                    world,
                    c.room,
                    [Some(c.id), None],
                    Msg::ExitTextYellow { text },
                );
            }
            if !m[2].is_empty() {
                let back = c.dir.reverse().map_or(String::new(), |r| dirname(world, r));
                let text = fmt_msg(&m[2], &[Arg::S(&green_name(&c.name)), Arg::S(&back)]);
                tell_room_all(
                    world,
                    target,
                    [Some(c.id), c.drag],
                    Msg::ExitTextYellow { text },
                );
            }
        }
        if let Some(p) = world.session_mut(c.id).and_then(|s| s.character.as_mut()) {
            p.status |= PlayerStatus::JUSTARRIVED;
        }
        set_user_location(world, c.id, target, false);
        if let Some(m) = &msg
            && !m[0].is_empty()
        {
            tell_user(world, c.id, Msg::ExitTextGreen { text: m[0].clone() });
        }
        show_room(world, c.id);
        HOOKS.items.deduct_item_charge(world, c.id, slot);
        return Gate::Carried { out: true };
    }
    match message(world, i32::from(c.exit.par2)) {
        Some(m) => {
            if !m[0].is_empty() {
                tell_user(world, c.id, Msg::ExitTextYellow { text: m[0].clone() });
            }
            if !m[1].is_empty() {
                let text = fmt_msg(&m[1], &[Arg::S(&green_name(&c.name))]);
                tell_room_all(
                    world,
                    c.room,
                    [Some(c.id), None],
                    Msg::ExitTextYellow { text },
                );
            }
        }
        None => tell_user(world, c.id, Msg::ExitNeedItem),
    }
    Gate::Refused
}

/// The room description, in the player's display mode.
pub fn show_room(world: &mut World, id: SessionId) {
    let verbose = world
        .session(id)
        .and_then(|s| s.player())
        .map_or(0, |p| p.verbose);
    display_room(world, id, RoomMode::verbose(verbose));
}

/// A Toll exit: `exitpar1s` gold crowns, paid by value, listed after "You just paid";
/// paying gives a sneaker away.
fn toll(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    let gold = c.exit.par1;
    let price = economy::amount_to_copper(gold, Currency::Gold);
    let Some(coins) = world
        .session(c.id)
        .and_then(|s| s.player())
        .map(|p| p.coins)
    else {
        return Gate::Refused;
    };
    if let Some(payment) = economy::pay(coins, price) {
        if let Some(p) = world.session_mut(c.id).and_then(|s| s.character.as_mut()) {
            p.coins = payment.left;
        }
        let paid = coin_list(world, payment.paid);
        tell_user(world, c.id, Msg::TollPaid { paid });
        return Gate::Pass {
            newmap: c.room.map,
            seen: true,
        };
    }
    // `exitpar1s` read as a signed int: the low word
    let n = i64::from(gold as u16 as i16);
    let cur = currency_name(world, i64::from(gold), Currency::Gold);
    refuse(world, c.id, Msg::TollShort { n, cur })
}

/// A currency's name: singular for exactly one, else plural; the fifth currency is the
/// sysop's `USRCURNM`.
pub fn currency_name(world: &World, amount: i64, cur: Currency) -> String {
    if cur == Currency::Runic {
        return world.config.usr_cur_name.clone();
    }
    let name = if amount == 1 {
        cur.singular_name()
    } else {
        cur.plural_name()
    };
    name.unwrap_or("").to_owned()
}

/// The coin list a payment shows: ` N name` per type handed over, largest first,
/// comma-separated, or ` nothing`.
pub fn coin_list(world: &World, paid: silvermere_types::records::Coins) -> String {
    let mut out = String::new();
    let mut any = false;
    let parts = [
        (paid.runic, Currency::Runic),
        (paid.platinum, Currency::Platinum),
        (paid.gold, Currency::Gold),
        (paid.silver, Currency::Silver),
        (paid.copper, Currency::Copper),
    ];
    for (n, cur) in parts {
        if n == 0 {
            continue;
        }
        if any {
            out.push(',');
        }
        any = true;
        out.push_str(&format!(" {n} {}", currency_name(world, i64::from(n), cur)));
    }
    if !any {
        out.push_str(" nothing");
    }
    out
}

/// An Action exit: message `exitpar1s` (0: a plain exit) moves a seen player itself, same
/// map, and tells both rooms.
fn action(world: &mut World, c: &GateCtx) -> Gate {
    if c.exit.par1 == 0 || c.unseen {
        return pass(c);
    }
    let target = RoomKey {
        map: c.room.map,
        room: c.exit.target,
    };
    if !user_allowed_in_room(world, c.id, target, c.room_type, false) {
        return refuse(world, c.id, Msg::NotPermitted);
    }
    let Some(m) = message(world, c.exit.par1 as i32) else {
        return pass(c);
    };
    set_user_location(world, c.id, target, false);
    if c.movedrag
        && let Some(v) = c.drag
    {
        clear_search_flags(world, v, c.room);
        set_user_location(world, v, target, false);
    }
    let buf = c.drag_suffix.as_str();
    if !m[1].is_empty() {
        let text = fmt_msg(&m[1], &[Arg::S(&green_name(&c.name)), Arg::S(buf)]);
        tell_room_all(
            world,
            c.room,
            [Some(c.id), c.drag],
            Msg::ExitTextYellow { text },
        );
    }
    if !m[2].is_empty() {
        let text = fmt_msg(&m[2], &[Arg::S(&green_name(&c.name)), Arg::S(buf)]);
        tell_room_all(
            world,
            target,
            [Some(c.id), c.drag],
            Msg::ExitTextYellow { text },
        );
    }
    if !m[0].is_empty() {
        let text = fmt_msg(&m[0], &[Arg::S(buf)]);
        tell_user(world, c.id, Msg::ExitTextGreen { text });
    }
    show_room(world, c.id);
    if c.movedrag
        && let Some(v) = c.drag
    {
        tell_user(
            world,
            v,
            Msg::DraggingYou {
                who: c.name.clone(),
            },
        );
        show_room(world, v);
    }
    Gate::Carried { out: false }
}

/// A Gate or Door exit: `exitpar1s` 1 is `Closed!`, 2 is `closed!`.
fn gate_or_door(world: &mut World, c: &GateCtx, one: Msg, two: Msg) -> Gate {
    match c.exit.par1 {
        1 => refuse(world, c.id, one),
        2 => refuse(world, c.id, two),
        _ => pass(c),
    }
}

/// A Text exit: by typed words (how 4) or behind a leader (how 3), with message
/// `exitpar3s`; otherwise there is no exit.
fn text(world: &mut World, c: &GateCtx) -> Gate {
    if c.how != 4 && c.how != 3 {
        return refuse(world, c.id, Msg::NoExit);
    }
    if c.exit.par3 == 0 {
        return pass(c);
    }
    let Some(m) = message(world, c.exit.par3) else {
        return pass(c);
    };
    let target = RoomKey {
        map: c.room.map,
        room: c.exit.target,
    };
    if !user_allowed_in_room(world, c.id, target, c.room_type, false) {
        return refuse(world, c.id, Msg::NotPermitted);
    }
    set_user_location(world, c.id, target, false);
    if c.movedrag
        && let Some(v) = c.drag
    {
        set_user_location(world, v, target, false);
    }
    let buf = c.drag_suffix.as_str();
    if !m[1].is_empty() {
        let text = fmt_msg(&m[1], &[Arg::S(&yellow_name(&c.name)), Arg::S(buf)]);
        tell_room_all(
            world,
            c.room,
            [Some(c.id), c.drag],
            Msg::ExitTextPlain { text },
        );
    }
    if !m[2].is_empty() {
        let text = fmt_msg(&m[2], &[Arg::S(&yellow_name(&c.name)), Arg::S(buf)]);
        tell_room_all(
            world,
            target,
            [Some(c.id), c.drag],
            Msg::ExitTextGreen { text },
        );
    }
    if !m[0].is_empty() {
        let text = fmt_msg(&m[0], &[Arg::S(buf)]);
        tell_user(world, c.id, Msg::ExitTextYellow { text });
    }
    show_room(world, c.id);
    prompt(world, c.id);
    if c.movedrag
        && let Some(v) = c.drag
    {
        tell_user(
            world,
            v,
            Msg::DraggingYou {
                who: c.name.clone(),
            },
        );
        show_room(world, v);
        prompt(world, v);
    }
    Gate::Carried { out: false }
}

/// The prompt to the player.
pub fn prompt(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
}

/// The trap kind decides: `Some(true)` springs, `Some(false)` is an ordinary exit, `None`
/// refused as no exit or nothing.
fn trap_springs(world: &mut World, c: &GateCtx) -> Result<bool, Gate> {
    match c.exit.par2 {
        trap::HURTOFF | trap::THROWOFF => Ok(false),
        trap::HIDDEN if c.how != 7 => Err(refuse(world, c.id, Msg::NoExit)),
        trap::HIDDEN | trap::HURT | trap::THROW => Ok(true),
        _ => Err(Gate::Stay),
    }
}

/// A Trap exit: `exitpar1s / 2` to `exitpar1s + 1` damage, message `exitpar3s`, and through
/// the exit he goes.
fn trap_exit(world: &mut World, c: &GateCtx) -> Gate {
    match trap_springs(world, c) {
        Ok(false) => return pass(c),
        Err(g) => return g,
        Ok(true) => {}
    }
    let base = c.exit.par1 as i32;
    let dmg = world.rng.range(base / 2, base + 1);
    if let Some(p) = world.session_mut(c.id).and_then(|s| s.character.as_mut()) {
        p.hp = p.hp.wrapping_sub(dmg as i16);
    }
    let target = RoomKey {
        map: c.room.map,
        room: c.exit.target,
    };
    let msg = message(world, c.exit.par3);
    if let Some(m) = &msg {
        if !m[1].is_empty() {
            let text = fmt_msg(&m[1], &[Arg::S(&green_name(&c.name))]);
            tell_room_all(
                world,
                c.room,
                [Some(c.id), None],
                Msg::ExitTextYellow { text },
            );
        }
        if !m[2].is_empty() {
            let text = fmt_msg(&m[2], &[Arg::S(&green_name(&c.name))]);
            tell_room_all(
                world,
                target,
                [Some(c.id), c.drag],
                Msg::ExitTextYellow { text },
            );
        }
    }
    // (sic) the entry check runs for its side effects; its answer is not used
    let _ = user_allowed_in_room(world, c.id, target, c.room_type, false);
    if let Some(p) = world.session_mut(c.id).and_then(|s| s.character.as_mut()) {
        p.status |= PlayerStatus::JUSTARRIVED;
    }
    set_user_location(world, c.id, target, false);
    if let Some(m) = &msg {
        if !m[0].is_empty() {
            let text = fmt_msg(&m[0], &[Arg::D(i64::from(dmg))]);
            tell_user(world, c.id, Msg::ExitTextGreen { text });
        }
        show_room(world, c.id);
    }
    Gate::Carried { out: false }
}

/// A Spell Trap exit: message `exitpar4s` (room texts with a blank line after), spell
/// `exitpar1s` on the player, then through unless the spell moved him.
fn spell_trap(world: &mut World, c: &GateCtx) -> Gate {
    match trap_springs(world, c) {
        Ok(false) => return pass(c),
        Err(g) => return g,
        Ok(true) => {}
    }
    let target = RoomKey {
        map: c.room.map,
        room: c.exit.target,
    };
    let msg = message(world, c.exit.par4);
    if let Some(m) = &msg {
        if !m[1].is_empty() {
            let text = fmt_msg(&m[1], &[Arg::S(&green_name(&c.name))]);
            tell_room_all(
                world,
                c.room,
                [Some(c.id), None],
                Msg::ExitTextBareDouble { text },
            );
        }
        if !m[2].is_empty() {
            let text = fmt_msg(&m[2], &[Arg::S(&green_name(&c.name))]);
            tell_room_all(
                world,
                target,
                [Some(c.id), c.drag],
                Msg::ExitTextBareDouble { text },
            );
        }
        // (sic) msgs[0] takes `dmg`, which only a Trap exit sets; 0 here
        if !m[0].is_empty() {
            let text = fmt_msg(&m[0], &[Arg::D(0)]);
            tell_user(world, c.id, Msg::ExitTextGreen { text });
        }
    }
    HOOKS
        .spells
        .room_cast_on_user(world, c.id, c.exit.par1 as i32);
    let still_here = world
        .session(c.id)
        .and_then(|s| s.room_key())
        .is_some_and(|k| k == c.room);
    if still_here {
        if let Some(p) = world.session_mut(c.id).and_then(|s| s.character.as_mut()) {
            p.status |= PlayerStatus::JUSTARRIVED;
        }
        set_user_location(world, c.id, target, false);
    }
    show_room(world, c.id);
    Gate::Carried { out: false }
}

/// A Hidden exit: passable, found or visible-but-blocked; otherwise there is no exit.
fn hidden(world: &mut World, c: &GateCtx) -> Gate {
    if hidden_passable(c.exit.par1) {
        return pass(c);
    }
    refuse(world, c.id, Msg::NoExit)
}

/// A Remote exit: never walked.
fn remote(world: &mut World, c: &GateCtx) -> Gate {
    refuse(world, c.id, Msg::NoExit)
}

/// A Class or Race exit: `exitpar1s` the only class (race) allowed, else `exitpar2s` the
/// one barred.
fn class_or_race(world: &mut World, c: &GateCtx, class: bool) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    if let Some(g) = tournament(world, c) {
        return g;
    }
    let Some(plr) = world.session(c.id).and_then(|s| s.player()) else {
        return Gate::Refused;
    };
    let mine = if class {
        u32::from(plr.class.0)
    } else {
        u32::from(plr.race.0)
    };
    let barred = if c.exit.par1 != 0 {
        mine != c.exit.par1
    } else {
        c.exit.par2 != 0 && mine == c.exit.par2 as u32
    };
    if barred {
        return refuse(world, c.id, Msg::ExitForbidden);
    }
    pass(c)
}

/// A Level exit: `exitpar1s` the lowest level, `exitpar2s` the highest (0: no limit).
fn level(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    if let Some(g) = tournament(world, c) {
        return g;
    }
    let Some(level) = world
        .session(c.id)
        .and_then(|s| s.player())
        .map(|p| i64::from(p.level))
    else {
        return Gate::Refused;
    };
    if c.exit.par1 != 0 && level < i64::from(c.exit.par1) {
        return refuse(world, c.id, Msg::ExitLevelLow);
    }
    if c.exit.par2 != 0 && level > i64::from(c.exit.par2) {
        return refuse(world, c.id, Msg::ExitLevelHigh);
    }
    pass(c)
}

/// An Alignment exit: evil points between `exitpar1s` (read as signed) and `exitpar2s`.
fn alignment(world: &mut World, c: &GateCtx) -> Gate {
    if let Some(g) = no_drag(world, c) {
        return g;
    }
    if let Some(g) = tournament(world, c) {
        return g;
    }
    let Some(evil) = world.session(c.id).and_then(|s| s.player()).map(|p| p.evil) else {
        return Gate::Refused;
    };
    // `exitpar1s` read as a signed int: the low word
    if evil < c.exit.par1 as u16 as i16 {
        return refuse(world, c.id, Msg::ExitTooGood);
    }
    if evil > c.exit.par2 {
        return refuse(world, c.id, Msg::ExitTooEvil);
    }
    pass(c)
}

/// A Timed exit: closed while `exitpar3s` is not 0; the refusal is `msgs[2]` of message
/// `exitpar1s` with the direction, or the standard text.
fn timed(world: &mut World, c: &GateCtx) -> Gate {
    if c.exit.par3 == 0 {
        return pass(c);
    }
    let own = message(world, c.exit.par1 as i32).filter(|m| !m[2].is_empty());
    let msg = match own {
        Some(m) => Msg::ExitTextPlain {
            text: fmt_msg(&m[2], &[Arg::S(&dirname(world, c.dir))]),
        },
        None => Msg::ExitTimedClosed,
    };
    refuse(world, c.id, msg)
}

/// A Cast exit: spell `exitpar1s` on the player first; dead, held or stunned he stays; else
/// message `exitpar3s` and on.
fn cast(world: &mut World, c: &GateCtx) -> Gate {
    if c.exit.par1 == 0 {
        return pass(c);
    }
    HOOKS
        .spells
        .room_cast_on_user(world, c.id, c.exit.par1 as i32);
    let felled = world
        .session(c.id)
        .and_then(|s| s.player())
        .is_none_or(|p| {
            p.hp < 0
                || p.status2
                    .intersects(PlayerStatus2::HELD | PlayerStatus2::KNOCKEDDOWN)
        });
    if felled {
        return Gate::Stay;
    }
    if c.exit.par3 == 0 {
        return pass(c);
    }
    let Some(m) = message(world, c.exit.par3) else {
        return pass(c);
    };
    if !m[0].is_empty() {
        tell_user(world, c.id, Msg::ExitTextPlain { text: m[0].clone() });
    }
    if m[1].is_empty() {
        return pass(c);
    }
    let here = world
        .session(c.id)
        .and_then(|s| s.room_key())
        .unwrap_or(c.room);
    let text = fmt_msg(&m[1], &[Arg::S(&c.name)]);
    tell_room_all(world, here, [Some(c.id), None], Msg::ExitTextPlain { text });
    pass(c)
}

/// An Ability exit: ability `exitpar1s` between `exitpar2s` and `exitpar3s` (the top
/// compared unsigned); else message `exitpar4s`, if any.
fn ability(world: &mut World, c: &GateCtx) -> Gate {
    let abl = silvermere_types::enums::Ability::from_code(c.exit.par1 as u16);
    if c.exit.par1 != 0 && HOOKS.abilities.has(world, c.id, abl) {
        let value = HOOKS.abilities.value(world, c.id, abl).unwrap_or(0);
        if i64::from(c.exit.par2) <= i64::from(value)
            && (value as u32) <= c.exit.par3 as u32
            && value >= 0
        {
            return pass(c);
        }
    }
    if c.exit.par4 == 0 {
        return Gate::Stay;
    }
    let Some(m) = message(world, c.exit.par4) else {
        return Gate::Stay;
    };
    if !m[0].is_empty() {
        tell_user(world, c.id, Msg::ExitTextPlain { text: m[0].clone() });
    }
    if m[1].is_empty() {
        return Gate::Stay;
    }
    let text = fmt_msg(&m[1], &[Arg::S(&c.name)]);
    tell_room_all(
        world,
        c.room,
        [Some(c.id), None],
        Msg::ExitTextPlain { text },
    );
    Gate::Stay
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_subset() {
        assert_eq!(
            fmt_msg("%s walks %s; 100%% sure", &[Arg::S("Bob"), Arg::S("north")]),
            "Bob walks north; 100% sure"
        );
        assert_eq!(fmt_msg("hurt for %d", &[Arg::D(7)]), "hurt for 7");
        assert_eq!(fmt_msg("%ld and %s", &[Arg::D(3)]), "3 and ");
        assert_eq!(fmt_msg("odd %q", &[]), "odd %q");
    }

    #[test]
    fn drag_gates() {
        assert!(!exit_allows_drag(ExitType::Toll));
        assert!(exit_allows_drag(ExitType::Door));
        assert!(exit_allows_drag(ExitType::Normal));
    }
}
