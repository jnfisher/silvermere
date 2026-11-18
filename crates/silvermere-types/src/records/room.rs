//! The `room` record (WCCMP001) and the `roomkey` record.

use serde::{Deserialize, Serialize};

use crate::enums::{ExitType, GangHouse, RoomType};
use crate::flags::{ExitFlags, RoomFlags};
use crate::ids::{
    ActiveMonsterId, ItemNo, MapId, MessageNo, MonsterNo, RoomId, RoomKey, ShopNo, TextBlockNo,
};
use crate::records::Coins;
use crate::records::layout::{self, Layout};
use crate::sizes::{NDIRS, NHIDITEMS, NPLACED, NROOMITEMS, NROOMMONS};

/// One exit of a room: the destination, exit type and four parameters at one direction index.
///
/// The parameters mean different things per [`ExitType`]; [`Exit::params`] names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    /// +0x338 destination room on the same map, 0 = no exit.
    pub target: RoomId,
    /// +0x360 exit type.
    pub kind: ExitType,
    /// +0x374 first parameter (32-bit unsigned).
    pub par1: u32,
    /// +0x39C second parameter (16-bit signed).
    pub par2: i16,
    /// +0x3B0 third parameter (32-bit signed).
    pub par3: i32,
    /// +0x3D8 fourth parameter (32-bit signed).
    pub par4: i32,
}

/// A lock state: a Door's or Gate's `par1`, a Key exit's `par2`, a Timed exit's `par3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// 0: open (a Timed exit: open).
    Open,
    /// 1: shut but not locked.
    Closed,
    /// 2: locked (a Timed exit: closed).
    Locked,
    /// Any other stored value, kept as it is.
    Other(i64),
}

impl Lock {
    /// Decodes a stored state.
    pub fn from_code(code: i64) -> Self {
        match code {
            0 => Self::Open,
            1 => Self::Closed,
            2 => Self::Locked,
            n => Self::Other(n),
        }
    }

    /// The stored state.
    pub fn code(self) -> i64 {
        match self {
            Self::Open => 0,
            Self::Closed => 1,
            Self::Locked => 2,
            Self::Other(n) => n,
        }
    }
}

/// A Trap's or Spell Trap's state (`par2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapState {
    /// 0: armed; a failed disarm hurts.
    Armed,
    /// 1: disarmed (was `Armed`).
    Disarmed,
    /// 2: not among the visible exits; springs only when walked into on purpose.
    Hidden,
    /// 3: armed; a failed disarm throws the player through.
    ArmedThrows,
    /// 4: disarmed (was `ArmedThrows`).
    DisarmedThrows,
    /// Any other stored value, kept as it is.
    Other(i16),
}

impl TrapState {
    /// Decodes a stored state.
    pub fn from_code(code: i16) -> Self {
        match code {
            0 => Self::Armed,
            1 => Self::Disarmed,
            2 => Self::Hidden,
            3 => Self::ArmedThrows,
            4 => Self::DisarmedThrows,
            n => Self::Other(n),
        }
    }

    /// The stored state.
    pub fn code(self) -> i16 {
        match self {
            Self::Armed => 0,
            Self::Disarmed => 1,
            Self::Hidden => 2,
            Self::ArmedThrows => 3,
            Self::DisarmedThrows => 4,
            Self::Other(n) => n,
        }
    }

    /// Armed, either kind.
    pub fn armed(self) -> bool {
        matches!(self, Self::Armed | Self::ArmedThrows)
    }
}

/// A Door's or Gate's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorExit {
    /// `par1`: the lock state.
    pub lock: Lock,
    /// `par2`: added to the bash and pick-lock chances; above 0 the door never locks.
    pub bonus: i16,
    /// `par3`: steps it stays open or unlocked.
    pub open_steps: i32,
    /// `par4`: the key item.
    pub key: ItemNo,
}

/// A Key exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyExit {
    /// `par1`: the key item.
    pub key: ItemNo,
    /// `par2`: the lock state.
    pub lock: Lock,
    /// `par3`: added to the pick-lock chance.
    pub pick_bonus: i32,
    /// `par4`: steps it stays open or unlocked.
    pub open_steps: i32,
}

/// An Item exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemExit {
    /// `par1`: the item that must be carried.
    pub item: ItemNo,
    /// `par2`: the message without it.
    pub refused: MessageNo,
    /// `par3`: the message with it.
    pub message: MessageNo,
}

/// A Hidden exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenExit {
    /// `par1`: the search and block flags.
    pub flags: ExitFlags,
    /// `par2`: how many blocks it gets back; negative, they may go in any order.
    pub blocks: i16,
    /// `par3`: the message when the last block goes.
    pub opened: MessageNo,
    /// `par4`: the message that names it among the exits.
    pub name: MessageNo,
}

impl HiddenExit {
    /// Passable, found or visible-but-blocked.
    pub fn passable(&self) -> bool {
        self.flags
            .intersects(ExitFlags::PASSABLE | ExitFlags::VISSEARCHED | ExitFlags::VISBLOCKED)
    }
}

/// A Trap's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrapExit {
    /// `par1`: the damage, from half of it to one more.
    pub damage: u32,
    /// `par2`: the trap state.
    pub state: TrapState,
    /// `par3`: the message when it springs.
    pub message: MessageNo,
    /// `par4`: the message when a disarm sets it off.
    pub disarm_failed: MessageNo,
}

/// A Spell Trap's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellTrapExit {
    /// `par1`: the spell cast on the player.
    pub spell: i32,
    /// `par2`: the trap state.
    pub state: TrapState,
    /// `par3`: the message when it is disarmed.
    pub disarmed: MessageNo,
    /// `par4`: the message when it springs or a disarm sets it off.
    pub message: MessageNo,
}

/// A Text exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextExit {
    /// `par1`: the message whose three texts are the words that work.
    pub words: MessageNo,
    /// `par3`: the message on the way through.
    pub message: MessageNo,
}

/// A Remote Action exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteExit {
    /// `par1`: the message whose three texts are the words that work.
    pub words: MessageNo,
    /// `par2`: the action times ten plus the exit of the far room it works on.
    pub code: i16,
    /// `par3`: the message.
    pub message: MessageNo,
    /// `par4`: the item that must be carried and loses a charge, 0 none.
    pub item: ItemNo,
}

impl RemoteExit {
    /// The action (the tens of `code`).
    pub fn action(&self) -> i32 {
        i32::from(self.code / 10)
    }

    /// The far room's exit (the units of `code`).
    pub fn exit(&self) -> i16 {
        self.code % 10
    }
}

/// A Class or Race exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassOrRaceExit {
    /// `par1`: the only class (race) allowed, 0 any.
    pub only: u32,
    /// `par2`: the class (race) barred, 0 none; used while `only` is 0.
    pub barred: i16,
}

/// A Level exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelExit {
    /// `par1`: the lowest level, 0 none.
    pub min: u32,
    /// `par2`: the highest level, 0 none.
    pub max: i16,
}

/// An Alignment exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignmentExit {
    /// `par1`: the fewest evil points, as stored; see [`AlignmentExit::min_evil`].
    pub min: u32,
    /// `par2`: the most evil points.
    pub max: i16,
}

impl AlignmentExit {
    /// `min` read as a signed int: its low word.
    pub fn min_evil(&self) -> i16 {
        self.min as u16 as i16
    }
}

/// A Timed exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedExit {
    /// `par1`: the opening, closing and refusal texts.
    pub message: MessageNo,
    /// `par2`: steps between swaps.
    pub steps: i16,
    /// `par3`: open, or closed as `Locked`.
    pub state: Lock,
}

impl TimedExit {
    /// Closed: any state but open.
    pub fn closed(&self) -> bool {
        self.state != Lock::Open
    }
}

/// A Cast exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastExit {
    /// `par1`: the spell cast on the player first, 0 none (then a plain exit).
    pub spell: i32,
    /// `par2`: the spell cast after any attempt, 0 none.
    pub after: i32,
    /// `par3`: the message on the way through.
    pub message: MessageNo,
    /// `par4`: the message LOOK shows instead of the room.
    pub look: MessageNo,
}

/// An Ability exit's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityExit {
    /// `par1`: the ability, 0 none.
    pub ability: u32,
    /// `par2`: its lowest value.
    pub min: i16,
    /// `par3`: its highest value (compared unsigned).
    pub max: i32,
    /// `par4`: the refusal message.
    pub refused: MessageNo,
}

/// An exit's type with its parameters named: [`Exit::params`] and [`Exit::set_params`].
///
/// The types with no parameters read carry none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitParams {
    /// Normal.
    Normal,
    /// Spell.
    Spell,
    /// Key.
    Key(KeyExit),
    /// Item.
    Item(ItemExit),
    /// Toll: `par1` gold crowns.
    Toll {
        /// `par1`: the price in gold crowns.
        gold: u32,
    },
    /// Action.
    Action {
        /// `par1`: the message, 0 for a plain exit.
        message: MessageNo,
    },
    /// Hidden.
    Hidden(HiddenExit),
    /// Door.
    Door(DoorExit),
    /// Map Change.
    MapChange {
        /// `par1`: the map it leads into.
        map: MapId,
    },
    /// Trap.
    Trap(TrapExit),
    /// Text.
    Text(TextExit),
    /// Gate.
    Gate(DoorExit),
    /// Remote Action.
    Remote(RemoteExit),
    /// Class.
    Class(ClassOrRaceExit),
    /// Race.
    Race(ClassOrRaceExit),
    /// Level.
    Level(LevelExit),
    /// Timed.
    Timed(TimedExit),
    /// Ticket.
    Ticket,
    /// User Count.
    UserCount,
    /// Block Guard.
    BlockGuard,
    /// Alignment.
    Alignment(AlignmentExit),
    /// Delay.
    Delay,
    /// Cast.
    Cast(CastExit),
    /// Ability.
    Ability(AbilityExit),
    /// Spell Trap.
    SpellTrap(SpellTrapExit),
    /// A code with no known type.
    Unknown(u16),
}

impl ExitParams {
    /// The exit type.
    pub fn kind(&self) -> ExitType {
        match self {
            Self::Normal => ExitType::Normal,
            Self::Spell => ExitType::Spell,
            Self::Key(_) => ExitType::Key,
            Self::Item(_) => ExitType::Item,
            Self::Toll { .. } => ExitType::Toll,
            Self::Action { .. } => ExitType::Action,
            Self::Hidden(_) => ExitType::Hidden,
            Self::Door(_) => ExitType::Door,
            Self::MapChange { .. } => ExitType::MapChange,
            Self::Trap(_) => ExitType::Trap,
            Self::Text(_) => ExitType::Text,
            Self::Gate(_) => ExitType::Gate,
            Self::Remote(_) => ExitType::RemoteAction,
            Self::Class(_) => ExitType::Class,
            Self::Race(_) => ExitType::Race,
            Self::Level(_) => ExitType::Level,
            Self::Timed(_) => ExitType::Timed,
            Self::Ticket => ExitType::Ticket,
            Self::UserCount => ExitType::UserCount,
            Self::BlockGuard => ExitType::BlockGuard,
            Self::Alignment(_) => ExitType::Alignment,
            Self::Delay => ExitType::Delay,
            Self::Cast(_) => ExitType::Cast,
            Self::Ability(_) => ExitType::Ability,
            Self::SpellTrap(_) => ExitType::SpellTrap,
            Self::Unknown(code) => ExitType::Unknown(*code),
        }
    }

    /// A Door's, Gate's or Key exit's lock state and the steps it stays open or unlocked.
    pub fn lock(&self) -> Option<(Lock, i32)> {
        match self {
            Self::Door(d) | Self::Gate(d) => Some((d.lock, d.open_steps)),
            Self::Key(k) => Some((k.lock, k.open_steps)),
            _ => None,
        }
    }

    /// The same with the lock state of a Door, Gate or Key exit changed.
    pub fn with_lock(mut self, lock: Lock) -> Self {
        match &mut self {
            Self::Door(d) | Self::Gate(d) => d.lock = lock,
            Self::Key(k) => k.lock = lock,
            _ => {}
        }
        self
    }

    /// The state of a Trap or Spell Trap.
    pub fn trap_state(&self) -> Option<TrapState> {
        match self {
            Self::Trap(t) => Some(t.state),
            Self::SpellTrap(t) => Some(t.state),
            _ => None,
        }
    }

    /// The same with the state of a Trap or Spell Trap changed.
    pub fn with_trap_state(mut self, state: TrapState) -> Self {
        match &mut self {
            Self::Trap(t) => t.state = state,
            Self::SpellTrap(t) => t.state = state,
            _ => {}
        }
        self
    }
}

impl Exit {
    /// The exit type with its parameters.
    pub fn params(&self) -> ExitParams {
        let (p1, p2, p3, p4) = (self.par1, self.par2, self.par3, self.par4);
        let door = || DoorExit {
            lock: Lock::from_code(i64::from(p1)),
            bonus: p2,
            open_steps: p3,
            key: ItemNo(p4),
        };
        let class_or_race = || ClassOrRaceExit {
            only: p1,
            barred: p2,
        };
        match self.kind {
            ExitType::Normal => ExitParams::Normal,
            ExitType::Spell => ExitParams::Spell,
            ExitType::Key => ExitParams::Key(KeyExit {
                key: ItemNo(p1 as i32),
                lock: Lock::from_code(i64::from(p2)),
                pick_bonus: p3,
                open_steps: p4,
            }),
            ExitType::Item => ExitParams::Item(ItemExit {
                item: ItemNo(p1 as i32),
                refused: MessageNo(i32::from(p2)),
                message: MessageNo(p3),
            }),
            ExitType::Toll => ExitParams::Toll { gold: p1 },
            ExitType::Action => ExitParams::Action {
                message: MessageNo(p1 as i32),
            },
            ExitType::Hidden => ExitParams::Hidden(HiddenExit {
                flags: ExitFlags::from_bits_retain(p1),
                blocks: p2,
                opened: MessageNo(p3),
                name: MessageNo(p4),
            }),
            ExitType::Door => ExitParams::Door(door()),
            ExitType::MapChange => ExitParams::MapChange { map: MapId(p1) },
            ExitType::Trap => ExitParams::Trap(TrapExit {
                damage: p1,
                state: TrapState::from_code(p2),
                message: MessageNo(p3),
                disarm_failed: MessageNo(p4),
            }),
            ExitType::Text => ExitParams::Text(TextExit {
                words: MessageNo(p1 as i32),
                message: MessageNo(p3),
            }),
            ExitType::Gate => ExitParams::Gate(door()),
            ExitType::RemoteAction => ExitParams::Remote(RemoteExit {
                words: MessageNo(p1 as i32),
                code: p2,
                message: MessageNo(p3),
                item: ItemNo(p4),
            }),
            ExitType::Class => ExitParams::Class(class_or_race()),
            ExitType::Race => ExitParams::Race(class_or_race()),
            ExitType::Level => ExitParams::Level(LevelExit { min: p1, max: p2 }),
            ExitType::Timed => ExitParams::Timed(TimedExit {
                message: MessageNo(p1 as i32),
                steps: p2,
                state: Lock::from_code(i64::from(p3)),
            }),
            ExitType::Ticket => ExitParams::Ticket,
            ExitType::UserCount => ExitParams::UserCount,
            ExitType::BlockGuard => ExitParams::BlockGuard,
            ExitType::Alignment => ExitParams::Alignment(AlignmentExit { min: p1, max: p2 }),
            ExitType::Delay => ExitParams::Delay,
            ExitType::Cast => ExitParams::Cast(CastExit {
                spell: p1 as i32,
                after: i32::from(p2),
                message: MessageNo(p3),
                look: MessageNo(p4),
            }),
            ExitType::Ability => ExitParams::Ability(AbilityExit {
                ability: p1,
                min: p2,
                max: p3,
                refused: MessageNo(p4),
            }),
            ExitType::SpellTrap => ExitParams::SpellTrap(SpellTrapExit {
                spell: p1 as i32,
                state: TrapState::from_code(p2),
                disarmed: MessageNo(p3),
                message: MessageNo(p4),
            }),
            ExitType::Unknown(code) => ExitParams::Unknown(code),
        }
    }

    /// Sets the exit type and the parameters `p` names; the others are left as they are.
    pub fn set_params(&mut self, p: ExitParams) {
        self.kind = p.kind();
        match p {
            ExitParams::Normal
            | ExitParams::Spell
            | ExitParams::Ticket
            | ExitParams::UserCount
            | ExitParams::BlockGuard
            | ExitParams::Delay
            | ExitParams::Unknown(_) => {}
            ExitParams::Key(k) => {
                self.par1 = k.key.0 as u32;
                self.par2 = k.lock.code() as i16;
                self.par3 = k.pick_bonus;
                self.par4 = k.open_steps;
            }
            ExitParams::Item(i) => {
                self.par1 = i.item.0 as u32;
                self.par2 = i.refused.0 as i16;
                self.par3 = i.message.0;
            }
            ExitParams::Toll { gold } => self.par1 = gold,
            ExitParams::Action { message } => self.par1 = message.0 as u32,
            ExitParams::Hidden(h) => {
                self.par1 = h.flags.bits();
                self.par2 = h.blocks;
                self.par3 = h.opened.0;
                self.par4 = h.name.0;
            }
            ExitParams::Door(d) | ExitParams::Gate(d) => {
                self.par1 = d.lock.code() as u32;
                self.par2 = d.bonus;
                self.par3 = d.open_steps;
                self.par4 = d.key.0;
            }
            ExitParams::MapChange { map } => self.par1 = map.0,
            ExitParams::Trap(t) => {
                self.par1 = t.damage;
                self.par2 = t.state.code();
                self.par3 = t.message.0;
                self.par4 = t.disarm_failed.0;
            }
            ExitParams::Text(t) => {
                self.par1 = t.words.0 as u32;
                self.par3 = t.message.0;
            }
            ExitParams::Remote(r) => {
                self.par1 = r.words.0 as u32;
                self.par2 = r.code;
                self.par3 = r.message.0;
                self.par4 = r.item.0;
            }
            ExitParams::Class(r) | ExitParams::Race(r) => {
                self.par1 = r.only;
                self.par2 = r.barred;
            }
            ExitParams::Level(l) => {
                self.par1 = l.min;
                self.par2 = l.max;
            }
            ExitParams::Timed(t) => {
                self.par1 = t.message.0 as u32;
                self.par2 = t.steps;
                self.par3 = t.state.code() as i32;
            }
            ExitParams::Alignment(a) => {
                self.par1 = a.min;
                self.par2 = a.max;
            }
            ExitParams::Cast(c) => {
                self.par1 = c.spell as u32;
                self.par2 = c.after as i16;
                self.par3 = c.message.0;
                self.par4 = c.look.0;
            }
            ExitParams::Ability(a) => {
                self.par1 = a.ability;
                self.par2 = a.min;
                self.par3 = a.max;
                self.par4 = a.refused.0;
            }
            ExitParams::SpellTrap(t) => {
                self.par1 = t.spell as u32;
                self.par2 = t.state.code();
                self.par3 = t.disarmed.0;
                self.par4 = t.message.0;
            }
        }
    }
}

/// An item slot of a room: `items[i]`, `itemuses[i]`, `itemcounts[i]` (or the hidden ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomItem {
    /// The item number, 0 = empty.
    pub item: ItemNo,
    /// Uses left, -1 none.
    pub uses: i16,
    /// Extra copies in this slot.
    pub count: i16,
}

/// A room: the `room` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    /// +0x000 map number and +0x004 room number (key 0).
    pub key: RoomKey,
    /// +0x105 room name.
    pub name: String,
    /// +0x13A long description, seven lines.
    pub desc: [String; 7],
    /// +0x32B ANSI map file.
    pub mapfile: String,
    /// The ten exits, by [`crate::enums::Direction`].
    pub exits: [Exit; NDIRS],
    /// +0x400 active monsters here.
    pub monsters: [ActiveMonsterId; NROOMMONS],
    /// +0x43C room type (`type`).
    pub room_type: RoomType,
    /// +0x43E shop/bank/trainer, 0 = none.
    pub shopnum: ShopNo,
    /// +0x442 owner's user-id (`RF_OWNED`).
    pub owner: String,
    /// +0x460 generation: min level.
    pub minlevel: i16,
    /// +0x462 generation: max level (arena: player max).
    pub maxlevel: i16,
    /// +0x464 generation monster number.
    pub genmonster: MonsterNo,
    /// +0x468 light level, <0 dark.
    pub light: i16,
    /// +0x46A gang house number 0..10.
    pub ganghouse: GangHouse,
    /// +0x46C visible items with +0x4B0 their uses and +0x56B their extra copies.
    pub items: [RoomItem; NROOMITEMS],
    /// +0x4D2 hidden items with +0x50E their uses and +0x58D their extra copies.
    pub hiditems: [RoomItem; NHIDITEMS],
    /// +0x52C..+0x53C coins lying here.
    pub coins: Coins,
    /// +0x540..+0x550 hidden coins.
    pub hidcoins: Coins,
    /// +0x554 most monsters generated here.
    pub maxmonsters: u8,
    /// +0x557 monster group.
    pub mongroup: i16,
    /// +0x559 minute of last kill (run time).
    pub lastkill: u16,
    /// +0x55B room flags.
    pub flags: RoomFlags,
    /// +0x55F hidden slots found (run time).
    pub foundmask: u32,
    /// +0x563 reincarnation room, 0 default.
    pub deathroom: RoomId,
    /// +0x567 room for a hang-up here.
    pub huproom: RoomId,
    /// +0x5AB WCCTEXT block: room command.
    pub cmdtext: TextBlockNo,
    /// +0x5B3 minutes between regenerations.
    pub regendelay: i16,
    /// +0x5B5 area monster maximum.
    pub areamax: i16,
    /// +0x5B7 area monsters now (run time).
    pub areacount: i16,
    /// +0x5B9 controlling room, 0 = none.
    pub ctlroom: RoomId,
    /// +0x5BD specific monster (known number).
    pub specmon: MonsterNo,
    /// +0x5C1 placed items.
    pub placed: [ItemNo; NPLACED],
    /// +0x5E9 WCCTEXT: protected reason.
    pub protmsg: TextBlockNo,
    /// +0x5EE spell on a failed pick-lock.
    pub picktrap: i16,
    /// +0x5F1 occupant spell, medium cycle.
    pub roomspell: i32,
    /// +0x5F6 players who found hidden items.
    pub searchers: u8,
    /// +0x5F7 monsters generated here now.
    pub nummonsters: u8,
}

impl Room {
    /// Byte layout of the `room` record.
    pub const LAYOUT: &'static Layout = &layout::ROOM;
    /// The `room` record length: 1528 bytes.
    pub const RECORD_LEN: usize = 1528;
}

impl RoomKey {
    /// Byte layout of the `roomkey` record.
    pub const LAYOUT: &'static Layout = &layout::ROOMKEY;
    /// The `roomkey` record length: 8 bytes.
    pub const RECORD_LEN: usize = 8;
}

#[cfg(test)]
mod tests {
    use super::*;

    // every type, ordinary and out-of-range values: decoding and encoding again changes nothing
    #[test]
    fn params_round_trip() {
        let pars: [(u32, i16, i32, i32); 5] = [
            (0, 0, 0, 0),
            (1, 2, 3, 4),
            (u32::MAX, i16::MIN, i32::MIN, i32::MAX),
            (0x8001_0002, -1, -7, -123_456),
            (5, 99, 2, 77),
        ];
        for code in 0..=30u16 {
            for (par1, par2, par3, par4) in pars {
                let exit = Exit {
                    target: RoomId(9),
                    kind: ExitType::from_code(code),
                    par1,
                    par2,
                    par3,
                    par4,
                };
                let mut again = exit;
                again.set_params(exit.params());
                assert_eq!(again, exit, "type {code}");
            }
        }
    }
}
