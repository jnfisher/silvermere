//! [`cmd_sysop`]: the operator's console inside the game.
//!
//! An operator types `SYSOP <sub-command> ...`. In [`cmd_sysop`] a word is tried against each
//! sub-command in turn with `starts_with_ignore_case` (an abbreviation matches), and a
//! sub-command whose gate fails lets the word fall through to the next one, ending in `Command
//! not recognized.`. The order is the one that resolves abbreviations as the original does.
//!
//! Gates. Sysop and game operator are the session's roles (`has_role`); the 21 `SYS_*` INI keys
//! and the developer gate are [`Key`]s the login computes from the account's permissions
//! ([`Keys`], `has_permission`). An INI key was an *extra* lock on top of the role test and empty
//! by default, so the default sets grant every key to both roles; what is the sysop's alone stays
//! a sysop role test at each sub-command. Two keys differ: `SYS_DISBAND_KEY` let a game operator
//! in only when set and held, so a game operator does not hold it by default, and the developer
//! gate (the twinks codec at 78, the registration-number `devmode`, the `devlevel` and "beta"
//! build checks) is [`Key::Debug`], held only when granted: the undocumented `CODEC_OF_TWINKS` was
//! in neither stock file, so no stock sysop passed it.
//!
//! Dropped (licensing and the host's caches; `docs/sysop.md`, `docs/differences.md`):
//! LANCE's `Reg#`/demo/IP/SBBS lines, INITIALIZE, EMULATE/NOEMULATE, the add-on gates of
//! GOTO, the credit lines of CONFIGURE SHOW and every demo-mode guard.
//!
//! The console notice becomes [`audit`]: a `tracing` line on target `audit` and a `sysop.<keyword>`
//! row in `events`. The log line "Sysop Command by %s: %s" under `LOG_SYSOP_COMMANDS` is a
//! `tracing` line on target `mmlog`.

use std::collections::HashMap;

use silvermere_text::Msg;
use silvermere_types::enums::{DebugLevel, RoomType};
use silvermere_types::flags::{
    GangFlags, PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2,
};
use silvermere_types::ids::{
    ItemNo, MapId, MonsterNo, RoomId, RoomKey, SessionId, SpellNo, TextBlockNo,
};

use super::dispatch::Handled;
use super::{Cmd, CommandLine, eq_ignore_case, starts_with_ignore_case};
use crate::comms::{UserMatch, find_user_number};
use crate::event_loop::{EngineOutput, GameEvent, PersistRequest};
use crate::items::inventory::show_options;
use crate::movement::HOOKS;
use crate::movement::move_user::set_user_location;
use crate::scheduler::Rng as _;
use crate::tell::{Noise, TellBlind, tell_game, tell_room, tell_user};
use crate::world::World;

macro_rules! keys {
    ($( $(#[$doc:meta])* $variant:ident = $name:literal, )*) => {
        /// What the engine knows of an account's permissions: the 21 `SYS_*` INI keys
        /// and the developer gate, named as `silvermere-auth`'s `Perm` serialises them.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Key {
            $( $(#[$doc])* $variant, )*
        }

        impl Key {
            /// Every key.
            pub const ALL: &'static [Key] = &[$(Key::$variant,)*];

            /// The INI key name (`Perm::ini_name`).
            pub const fn ini_name(self) -> &'static str {
                match self {
                    $( Key::$variant => $name, )*
                }
            }
        }
    };
}

keys! {
    /// `SYS_MAP_KEY`: SYSOP MAP.
    SysMap = "SYS_MAP_KEY",
    /// `SYS_REPORT_KEY`: SYSOP REPORT.
    SysReport = "SYS_REPORT_KEY",
    /// `SYS_GOTO_KEY`: SYSOP GOTO.
    SysGoto = "SYS_GOTO_KEY",
    /// `SYS_LIGHTNING_KEY`: SYSOP LIGHTNING.
    SysLightning = "SYS_LIGHTNING_KEY",
    /// `SYS_STATUS_KEY`: SYSOP STATUS.
    SysStatus = "SYS_STATUS_KEY",
    /// `SYS_RESET_ROOMS`: SYSOP RESET.
    SysResetRooms = "SYS_RESET_ROOMS",
    /// `SYS_ADDLIFE_KEY`: GOD user ADD LIFE.
    SysAddLife = "SYS_ADDLIFE_KEY",
    /// `SYS_RETRAIN_KEY`: GOD user RETRAIN.
    SysRetrain = "SYS_RETRAIN_KEY",
    /// `SYS_ALIGNMENT_KEY`: GOD user GOOD/SAINT/NEUTRAL/LAWFUL/UNLAWFUL.
    SysAlignment = "SYS_ALIGNMENT_KEY",
    /// `SYS_EDITEVIL_KEY`: GOD user ADD EVIL.
    SysEditEvil = "SYS_EDITEVIL_KEY",
    /// `SYS_JAIL_KEY`: GOD user JAIL.
    SysJail = "SYS_JAIL_KEY",
    /// `SYS_SPEAK_KEY`: SYSOP SPEAK and GOD user SPEAK.
    SysSpeak = "SYS_SPEAK_KEY",
    /// `SYS_LISTUSER_KEY`: LIST USERS, LIST EVIL.
    SysListUser = "SYS_LISTUSER_KEY",
    /// `SYS_LISTLIMIT_KEY`: LIST LIMITED_ITEMS, NEWLIMITED, GANG.
    SysListLimit = "SYS_LISTLIMIT_KEY",
    /// `SYS_USERINFO_KEY`: the GOD user displays.
    SysUserInfo = "SYS_USERINFO_KEY",
    /// `SYS_CHANGE_KEY`: GOD user CHANGE NAME/GANG/SEX.
    SysChange = "SYS_CHANGE_KEY",
    /// `SYS_TOPTEN_KEY`: GOD user ENABLE/DISABLE TOPTEN.
    SysTopTen = "SYS_TOPTEN_KEY",
    /// `SYS_DISBAND_KEY`: SYSOP DISBAND for a game operator.
    SysDisband = "SYS_DISBAND_KEY",
    /// `SYS_BUFFERS_KEY`: SYSOP BUFFERS.
    SysBuffers = "SYS_BUFFERS_KEY",
    /// `SYS_CLEARSUICIDE_KEY`: GOD user CLEAR SUICIDE.
    SysClearSuicide = "SYS_CLEARSUICIDE_KEY",
    /// `SYS_DISABLE_GANGS`: SYSOP DISABLE/ENABLE gang.
    SysDisableGangs = "SYS_DISABLE_GANGS",
    /// The developer gate: DEBUG, MONSTER, DUMP, SUMMON, EXPRESS_TRAIN, VALUES, TWEAK, the
    /// developer LISTs and CONFIGURE GENRATE/MINWAIT.
    Debug = "DEBUG",
}

impl Key {
    /// The key an INI name (or `DEBUG`) names, ignoring case.
    pub fn from_ini_name(name: &str) -> Option<Key> {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.ini_name().eq_ignore_ascii_case(name))
    }
}

/// The keys one session holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Keys(u32);

impl Keys {
    /// No key.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether `key` is held.
    pub const fn contains(self, key: Key) -> bool {
        self.0 & (1 << key as u32) != 0
    }

    /// Adds `key`.
    pub fn insert(&mut self, key: Key) {
        self.0 |= 1 << key as u32;
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: Key) {
        self.0 &= !(1 << key as u32);
    }

    /// The default set of the roles, as `has_permission` grants it: a sysop every INI key;
    /// a game operator every INI key but `SYS_DISBAND_KEY`; anyone else none. Nobody holds
    /// [`Key::Debug`] by default: `CODEC_OF_TWINKS` was in neither stock file.
    pub fn for_roles(sysop: bool, gameop: bool) -> Self {
        let mut k: Keys = Key::ALL.iter().copied().collect();
        k.remove(Key::Debug);
        if sysop {
            return k;
        }
        if gameop {
            k.remove(Key::SysDisband);
            return k;
        }
        Self::empty()
    }
}

impl FromIterator<Key> for Keys {
    fn from_iter<I: IntoIterator<Item = Key>>(iter: I) -> Self {
        let mut set = Self::empty();
        for k in iter {
            set.insert(k);
        }
        set
    }
}

/// The sysop-wide state: the defunct-user limits
/// (`purgemins`/`purgedays`/`purgeexp`), the wait spinner's frame (`spinpos`), and the per-session
/// jobs and paged reports.
#[derive(Debug, Clone)]
pub struct SysopState {
    pub purge_mins: i32,
    pub purge_days: i32,
    pub purge_exp: i32,
    /// Set by RESET and the menu's R. Nothing reads it here.
    pub need_recov: bool,
    /// The wall clock, Unix seconds, when the cache statistics were last cleared.
    pub stats_start: i64,
    /// `spinpos`, 0-3.
    pub spinner: u8,
    /// A running menu job by session (C, CP, D, H).
    pub jobs: HashMap<SessionId, crate::state::sysop_menu::Job>,
    /// A paged REPORT USERS / NOTOPTEN by session.
    pub reports: HashMap<SessionId, crate::display::sysop::Report>,
    /// A paged GOD user INFO by session: (part, user shown) (`USF_SYSOPLIST`).
    pub info: HashMap<SessionId, (u8, SessionId)>,
    /// A hoarder check is running (one at a time across every channel, as in the original).
    pub item_check_running: bool,
}

impl Default for SysopState {
    fn default() -> Self {
        Self {
            purge_mins: 60,
            purge_days: 14,
            purge_exp: 1000,
            need_recov: false,
            stats_start: 0,
            spinner: 0,
            jobs: HashMap::new(),
            reports: HashMap::new(),
            info: HashMap::new(),
            item_check_running: false,
        }
    }
}

/// The disposition of one `cmd_sysop` keyword, for the table test and `docs/sysop.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Kept; the gate as the original has it.
    Kept(&'static str),
    /// Not carried, with the reason.
    Dropped(&'static str),
}

/// Every keyword of SYSOP and of the sysop menu, in the order they are tried, with its gate or
/// why it is gone.
pub const KEYWORDS: &[(&str, Disposition)] = &[
    (
        "lance",
        Disposition::Dropped(
            "reduced: Reg#, demo mode, IP and SBBS lines (licensing); Lcon and Death Setting kept, for anyone",
        ),
    ),
    ("debug", Disposition::Kept("sysop + DEBUG")),
    ("monster", Disposition::Kept("sysop + DEBUG")),
    (
        "dump",
        Disposition::Kept("sysop + DEBUG (the original's \"beta\" build check)"),
    ),
    ("reset", Disposition::Kept("sysop + SYS_RESET_ROOMS")),
    ("buffers", Disposition::Kept("operator + SYS_BUFFERS_KEY")),
    ("reload", Disposition::Kept("operator")),
    (
        "initialize",
        Disposition::Dropped("rebuilt the record caches; there are none (05 section 6)"),
    ),
    ("summon", Disposition::Kept("sysop + DEBUG")),
    ("express_train", Disposition::Kept("sysop + DEBUG")),
    (
        "goto",
        Disposition::Kept("operator + SYS_GOTO_KEY; SUPPORT sysop only"),
    ),
    (
        "lightning",
        Disposition::Kept("operator + SYS_LIGHTNING_KEY"),
    ),
    ("transfer", Disposition::Kept("sysop")),
    (
        "values",
        Disposition::Kept("sysop + DEBUG (the original's devlevel 2)"),
    ),
    ("status", Disposition::Kept("operator + SYS_STATUS_KEY")),
    (
        "disband",
        Disposition::Kept("sysop, or operator + SYS_DISBAND_KEY"),
    ),
    ("speak", Disposition::Kept("operator + SYS_SPEAK_KEY")),
    ("arena", Disposition::Kept("sysop")),
    ("disable", Disposition::Kept("sysop + SYS_DISABLE_GANGS")),
    ("enable", Disposition::Kept("sysop + SYS_DISABLE_GANGS")),
    ("gangsize", Disposition::Kept("sysop")),
    ("god", Disposition::Kept("operator + the option's key")),
    (
        "EMULATE",
        Disposition::Dropped("autoplay test mode (03, deleted outright)"),
    ),
    (
        "NOEMULATE",
        Disposition::Dropped("autoplay test mode (03, deleted outright)"),
    ),
    (
        "list",
        Disposition::Kept(
            "operator + SYS_LISTUSER_KEY / SYS_LISTLIMIT_KEY; developer lists sysop + DEBUG",
        ),
    ),
    ("report", Disposition::Kept("operator + SYS_REPORT_KEY")),
    ("map", Disposition::Kept("operator + SYS_MAP_KEY")),
    (
        "configure",
        Disposition::Kept("sysop; GENRATE/MINWAIT + DEBUG"),
    ),
    ("tweak", Disposition::Kept("sysop + DEBUG")),
    (
        "menu C",
        Disposition::Kept("sysop menu: clear saved evil points"),
    ),
    (
        "menu CP",
        Disposition::Kept("sysop menu: clear saved profiles"),
    ),
    (
        "menu U",
        Disposition::Dropped("apply an update: the importer CLI (T-009)"),
    ),
    (
        "menu R",
        Disposition::Kept("sysop menu: the obsolete-reindex notice"),
    ),
    (
        "menu D",
        Disposition::Kept("sysop menu: check for defunct users"),
    ),
    (
        "menu H",
        Disposition::Kept("sysop menu: check for item hoarders"),
    ),
    (
        "menu I",
        Disposition::Kept("sysop menu: WCCMMUD.RLN from the assets"),
    ),
    (
        "menu N",
        Disposition::Kept("sysop menu: WCCMMSYS.REL from the assets"),
    ),
    (
        "menu S",
        Disposition::Kept("sysop menu: WCCMMSYS.NOT from the assets"),
    ),
    (
        "menu B",
        Disposition::Dropped("edit the sysop bulletin: the kernel's editor"),
    ),
    (
        "menu E",
        Disposition::Dropped("edit wccmmud.ini: the kernel's editor; the config file instead"),
    ),
    (
        "menu L",
        Disposition::Kept("sysop menu: the defunct-user limits"),
    ),
    ("menu A", Disposition::Dropped("add-on codes (licensing)")),
    (
        "menu X",
        Disposition::Kept("sysop menu: back to the main menu"),
    ),
];

/// The console notice: a `tracing` line on target `audit` and a `sysop.<keyword>` row in `events`,
/// with the operator's user-id.
pub fn audit(world: &mut World, id: SessionId, kind: &'static str, detail: String) {
    let user = world
        .session(id)
        .and_then(|s| s.player())
        .map_or_else(String::new, |p| p.userid.clone());
    tracing::info!(target: "audit", kind, user = %user, "{detail}");
    world.outbox.push(EngineOutput::Event(GameEvent::Sysop {
        id,
        kind,
        user,
        detail,
    }));
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    tell_user(world, id, msg);
}

/// Leading blanks, a sign, digits; 0 for anything else.
pub fn leading_int(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| {
            n.saturating_mul(10).saturating_add(i64::from(d - b'0'))
        });
    if neg { -n } else { n }
}

/// What the gates read off the operator's session.
#[derive(Debug, Clone, Copy)]
struct Op {
    sysop: bool,
    gameop: bool,
    keys: Keys,
}

impl Op {
    fn has(self, key: Key) -> bool {
        self.keys.contains(key)
    }
}

/// SYSOP. `NotCommand` for a player who is neither sysop nor game operator: the word then
/// goes on to speech, as in the original.
pub fn cmd_sysop(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(s) = world.session(id) else {
        return Handled::NotCommand;
    };
    if s.player().is_none() {
        return Handled::NotCommand;
    }
    let op = Op {
        sysop: s.flags.sysop,
        gameop: s.flags.gameop,
        keys: s.sysop_keys,
    };
    let argc = line.argc();
    // LANCE answers everybody, before the key test (sic); an operator then also gets "Command not
    // recognized." from the chain below.
    if argc == 2 && eq_ignore_case(line.arg(1), "lance") {
        push(
            world,
            id,
            Msg::SysopLanceLcon {
                max: i64::from(world.max_in_game),
            },
        );
        push(
            world,
            id,
            Msg::SysopLanceDeath {
                hp: i64::from(world.config.death_hp),
            },
        );
    }
    // only an operator
    if !op.sysop && !op.gameop {
        return Handled::NotCommand;
    }
    // the help
    if argc == 1 {
        push(world, id, Msg::SysopHelp);
        if op.sysop {
            push(world, id, Msg::SysopHelpSysop);
        }
        return Handled::Done;
    }
    // LOG_SYSOP_COMMANDS
    if world.config.log_sysop_commands {
        let user = world
            .session(id)
            .and_then(|s| s.player())
            .map_or_else(String::new, |p| p.userid.clone());
        tracing::info!(target: "mmlog", "Sysop Command by {user}: {}", line.raw);
    }
    chain(world, id, op, line)
}

/// The sub-commands, tried in turn.
fn chain(world: &mut World, id: SessionId, op: Op, line: &CommandLine) -> Handled {
    let argc = line.argc();
    let w = line.arg(1);
    let dev = op.has(Key::Debug);
    if op.sysop && starts_with_ignore_case(w, "debug") && dev {
        debug(world, id, line);
    } else if op.sysop && starts_with_ignore_case(w, "monster") && dev {
        monster(world, id, line);
    } else if starts_with_ignore_case(w, "dump") && op.sysop && dev {
        dump(world);
    } else if starts_with_ignore_case(w, "reset") && op.sysop && op.has(Key::SysResetRooms) {
        reset(world, id, line);
    } else if starts_with_ignore_case(w, "buffers") && op.has(Key::SysBuffers) {
        buffers(world, id, line);
    } else if starts_with_ignore_case(w, "reload") {
        // the INI settings are read again; the log is `tracing`, nothing to reopen
        world.outbox.push(EngineOutput::ReloadConfig { id });
        push(world, id, Msg::SysopReloaded);
        audit(world, id, "sysop.reload", "INI settings reloaded".into());
    } else if op.sysop && starts_with_ignore_case(w, "summon") && argc > 2 && dev {
        summon(world, id, line);
    } else if op.sysop && starts_with_ignore_case(w, "express_train") && argc > 2 && dev {
        express_train(world, id, line);
    } else if starts_with_ignore_case(w, "goto") && argc > 2 && op.has(Key::SysGoto) {
        goto(world, id, op, line);
    } else if starts_with_ignore_case(w, "lightning") && op.has(Key::SysLightning) {
        return lightning(world, id, line);
    } else if op.sysop && starts_with_ignore_case(w, "transfer") {
        // the last line this operator typed
        let last = world
            .session(id)
            .map_or_else(String::new, |s| s.ust.lastinput.clone());
        push(world, id, Msg::SysopTransferInfo { line: last });
    } else if dev && op.sysop && starts_with_ignore_case(w, "values") {
        values(world, id, line);
    } else if starts_with_ignore_case(w, "status") && op.has(Key::SysStatus) {
        status(world, id, line);
    } else if starts_with_ignore_case(w, "disband") && (op.sysop || op.has(Key::SysDisband)) {
        gang_flag(world, id, line, GangCmd::Disband);
    } else if starts_with_ignore_case(w, "speak") && op.has(Key::SysSpeak) {
        speak(world, id, line);
    } else if starts_with_ignore_case(w, "arena") && op.sysop {
        arena(world, id, line);
    } else if starts_with_ignore_case(w, "disable") && op.sysop && op.has(Key::SysDisableGangs) {
        gang_flag(world, id, line, GangCmd::Disable);
    } else if starts_with_ignore_case(w, "enable") && op.sysop && op.has(Key::SysDisableGangs) {
        gang_flag(world, id, line, GangCmd::Enable);
    } else if starts_with_ignore_case(w, "gangsize") && op.sysop {
        gangsize(world, id, line);
    } else if starts_with_ignore_case(w, "god") {
        return god(world, id, op, line);
    } else if starts_with_ignore_case(w, "list") {
        return list(world, id, op, line);
    } else if starts_with_ignore_case(w, "report") && op.has(Key::SysReport) {
        return report(world, id, line);
    } else if starts_with_ignore_case(w, "map") && op.has(Key::SysMap) {
        let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
            return Handled::Done;
        };
        crate::display::sysop::print_map(world, id, here, false, 26, 7);
    } else if starts_with_ignore_case(w, "configure") && op.sysop {
        configure(world, id, op, line);
    } else if op.sysop && starts_with_ignore_case(w, "tweak") && dev {
        tweak(world, id, line);
    } else {
        push(world, id, Msg::SysopNotRecognized);
    }
    Handled::Done
}

/// SYSOP DEBUG [OFF | ON | PARTIAL | BUFFERS]; alone it flips off and on.
fn debug(world: &mut World, id: SessionId, line: &CommandLine) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    if line.argc() == 2 {
        s.flags.debug = if s.flags.debug == DebugLevel::Off {
            DebugLevel::On
        } else {
            DebugLevel::Off
        };
    } else {
        let a = line.arg(2);
        if starts_with_ignore_case(a, "off") {
            s.flags.debug = DebugLevel::Off;
        } else if starts_with_ignore_case(a, "on") {
            s.flags.debug = DebugLevel::On;
        } else if starts_with_ignore_case(a, "partial") {
            s.flags.debug = DebugLevel::Partial;
        } else if starts_with_ignore_case(a, "buffers") {
            s.flags.debug = DebugLevel::Buffers;
        }
    }
    let level = i64::from(s.flags.debug.code());
    s.output.push(Msg::SysopDebugSet { level });
}

/// SYSOP MONSTER [number [room [map [min [max]]]]]: `generate_monster` with the operator's room and
/// map and levels 0 to 9999 for what is missing.
fn monster(world: &mut World, id: SessionId, line: &CommandLine) {
    let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let argc = line.argc();
    if argc > 7 {
        push(world, id, Msg::SysopMonsterSyntax);
        return;
    }
    let num = |i: usize| leading_int(line.arg(i));
    let monster = if argc >= 3 { num(2) } else { 0 };
    let room = RoomKey {
        room: if argc >= 4 {
            RoomId(num(3) as u32)
        } else {
            here.room
        },
        map: if argc >= 5 {
            MapId(num(4) as u32)
        } else {
            here.map
        },
    };
    let minlevel = if argc >= 6 { num(5) as i16 } else { 0 };
    let maxlevel = if argc >= 7 { num(6) as i16 } else { 9999 };
    // `generate_monster` in the operator's room, forced, so the room's spawn rules (its
    // monster limit, the regeneration delay after a kill) do not stop the operator
    crate::monsters::generate::generate_monster(
        world,
        room,
        crate::monsters::generate::Request {
            areatype: -1,
            monnum: MonsterNo(monster as i32),
            minlevel,
            maxlevel,
            owner: None,
            ally: false,
            force: true,
        },
    );
    audit(
        world,
        id,
        "sysop.monster",
        format!(
            "monster {monster} generated in {}/{}",
            room.room.0, room.map.0
        ),
    );
}

/// SYSOP DUMP: the active monsters' statistics and the active monsters written to the log.
fn dump(world: &World) {
    tracing::info!(target: "mmlog", count = world.active_monsters.len(), "active monster dump");
    for m in world.active_monsters.values() {
        tracing::info!(
            target: "mmlog",
            "{} {} [{}/{}] hp {}/{} {{{}}}",
            m.number.0,
            m.name,
            m.room,
            m.map,
            m.hp,
            m.maxhp,
            m.following
        );
    }
}

/// SYSOP RESET [ROOM <room> [map] | AREA | EXPERIENCE].
fn reset(world: &mut World, id: SessionId, line: &CommandLine) {
    let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let argc = line.argc();
    if argc > 3 && eq_ignore_case(line.arg(2), "room") {
        let room = RoomId(leading_int(line.arg(3)) as u32);
        let map = if argc == 4 {
            here.map
        } else {
            MapId(leading_int(line.arg(4)) as u32)
        };
        reset_room_monster_gen(world, id, RoomKey { map, room }, false);
        push(world, id, Msg::SysopDone);
    } else if argc == 3 && !eq_ignore_case(line.arg(2), "room") {
        if eq_ignore_case(line.arg(2), "area") {
            reset_room_monster_gen(world, id, here, true);
            push(world, id, Msg::SysopDone);
        } else if eq_ignore_case(line.arg(2), "experience") {
            let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
                return;
            };
            if plr.flags.contains(PlayerFlags::NEWEXP) {
                plr.expbil = 0;
                plr.exprem = 0;
                plr.flags.remove(PlayerFlags::NEWEXP);
                push(world, id, Msg::SysopResetExpUnmarked);
            } else {
                // the experience restructured
                plr.exprem = plr.exp;
                while plr.exprem >= 1_000_000_000 {
                    plr.expbil += 1;
                    plr.exprem -= 1_000_000_000;
                }
                plr.flags.insert(PlayerFlags::NEWEXP);
                plr.status2.insert(PlayerStatus2::EXPMARKED);
                push(world, id, Msg::SysopResetExpMarked);
            }
        } else {
            push(world, id, Msg::SysopResetSyntax);
        }
    } else {
        reset_room_monster_gen(world, id, here, false);
        push(world, id, Msg::SysopDone);
    }
    audit(
        world,
        id,
        "sysop.reset",
        format!("reset {}", line.rest_from(2)),
    );
}

/// A room's count of generated monsters, or its controlling room's area count, back to 0. The live
/// counts are saved at once (the room is marked changed).
pub fn reset_room_monster_gen(world: &mut World, id: SessionId, room: RoomKey, area: bool) {
    let Some(rm) = world.room(room) else {
        return;
    };
    if area {
        if rm.ctlroom.0 != 0 {
            let ctl = RoomKey {
                map: room.map,
                room: rm.ctlroom,
            };
            let Some(state) = world.room_state.get_mut(&ctl) else {
                push(world, id, Msg::SysopRecoveryNextShutdown);
                return;
            };
            state.areacount = 0;
            let save = state.clone();
            world
                .outbox
                .push(EngineOutput::Persist(PersistRequest::RoomState(Box::new(
                    save,
                ))));
            push(world, id, Msg::SysopAreaReset);
        }
    } else if let Some(state) = world.room_state.get_mut(&room) {
        state.nummonsters = 0;
        let save = state.clone();
        world
            .outbox
            .push(EngineOutput::Persist(PersistRequest::RoomState(Box::new(
                save,
            ))));
        push(world, id, Msg::SysopRoomReset);
    }
    push(world, id, Msg::SysopRecoveryNextShutdown);
    world.sysop.need_recov = true;
}

/// The slots of each cache as the shipped options set them, `xxxbuf + 1` (WCCMMUD.MSG:
/// ROOMBUF 2000, ITEMBUF 450, MONSBUF 300, KMONSBUF 200, SPELBUF 200, CLASSBUF 16, RACEBUF 12,
/// SHOPBUF 49, BANKBUF 25, MESSBUF 300, GANGBUF 35).
const ROOMBUF: usize = 2000;
const ITEMBUF: usize = 450;
const KMONSBUF: usize = 200;
const SPELBUF: usize = 200;
const CLASSBUF: usize = 16;
const RACEBUF: usize = 12;
const SHOPBUF: usize = 49;
const BANKBUF: usize = 25;
const MESSBUF: usize = 300;
const GANGBUF: usize = 35;

/// One [`display_buffer_statistics`] line: the label padded to 17 columns, then `%-5u` for each
/// number.
fn buffer_line(
    label: &str,
    size: usize,
    used: usize,
    hits: u32,
    miss: u32,
    modified: usize,
) -> Msg {
    Msg::SysopBuffersLine {
        label: format!("{label:<17}"),
        size: format!("{:<5}", size + 1),
        used: format!("{used:<5}"),
        hits: format!("{hits:<5}"),
        miss: format!("{miss:<5}"),
        modified: format!("{modified:<5}"),
    }
}

/// SYSOP BUFFERS' statistics: the heading, then a line per cache.
///
/// The active monsters are the cache silvermere models (`MonsterCache`): its Used, Hits and
/// Miss are the original's, Mod counts the resident monsters not yet written. The other ten
/// caches do not exist here (every record is in memory), so their lines show what the original
/// would after start-up: Used the records held, up to the cache size, no look-ups counted,
/// nothing modified.
pub fn display_buffer_statistics(world: &mut World, id: SessionId) {
    // the host's timer ticks of 1/65536 s as an unsigned long, printed with %ld
    let secs = (world.wall - world.sysop.stats_start).max(0) as u64;
    let elapsed = (secs << 16) as u32;
    push(
        world,
        id,
        Msg::SysopBuffersHeading {
            ticks: (elapsed as i32).to_string(),
            secs: (elapsed >> 16).to_string(),
        },
    );
    let held = |n: usize, buf: usize| n.min(buf + 1);
    let mons = world.active_monsters.cache();
    let (used, hits, miss) = (mons.used(), mons.hits(), mons.misses());
    let monsbuf = mons.monsbuf();
    let dirty = mons
        .resident()
        .filter(|(_, n)| world.active_monsters.is_dirty(*n))
        .count();
    drop(mons);
    let rows = [
        ("Rooms:", ROOMBUF, held(world.rooms.len(), ROOMBUF), 0, 0, 0),
        ("Items:", ITEMBUF, held(world.items.len(), ITEMBUF), 0, 0, 0),
        (
            "Active Monsters:",
            monsbuf,
            used as usize,
            hits,
            miss,
            dirty,
        ),
        (
            "Known Monsters:",
            KMONSBUF,
            held(world.monsters.len(), KMONSBUF),
            0,
            0,
            0,
        ),
        (
            "Spells:",
            SPELBUF,
            held(world.spells.len(), SPELBUF),
            0,
            0,
            0,
        ),
        (
            "Classes:",
            CLASSBUF,
            held(world.classes.len(), CLASSBUF),
            0,
            0,
            0,
        ),
        ("Races:", RACEBUF, held(world.races.len(), RACEBUF), 0, 0, 0),
        ("Shops:", SHOPBUF, held(world.shops.len(), SHOPBUF), 0, 0, 0),
        ("Bankbooks:", BANKBUF, 0, 0, 0, 0),
        ("Messages:", MESSBUF, 0, 0, 0, 0),
        ("Gangs:", GANGBUF, held(world.gangs.len(), GANGBUF), 0, 0, 0),
    ];
    for (label, buf, used, hits, miss, modified) in rows {
        push(
            world,
            id,
            buffer_line(label, buf, used, hits, miss, modified),
        );
    }
}

/// The counters to zero, "Cleared", and the statistics clock restarts.
fn clear_statistics(world: &mut World, id: SessionId) {
    world.active_monsters.cache_mut().clear_statistics();
    push(world, id, Msg::SysopBuffersCleared);
    world.sysop.stats_start = world.wall;
}

/// SYSOP BUFFERS [CLEAR | SAVE [n]]: CLEAR zeroes the counters and shows the statistics, SAVE shows
/// them and then saves the buffers (the persistence worker writes behind, so there is nothing more
/// to do), anything else shows them.
fn buffers(world: &mut World, id: SessionId, line: &CommandLine) {
    let argc = line.argc();
    if argc == 3 && starts_with_ignore_case(line.arg(2), "clear") {
        clear_statistics(world, id);
    }
    display_buffer_statistics(world, id);
}

/// The item whose name the text abbreviates; an exact match wins; several and no exact match list
/// them (the "which one?" list). Returns the item and the count of matches.
fn get_item_from_name(world: &mut World, id: SessionId, name: &str) -> (Option<ItemNo>, usize) {
    let mut numbers: Vec<ItemNo> = world.items.keys().copied().collect();
    numbers.sort_by_key(|n| n.0);
    let mut matches: Vec<(ItemNo, String)> = Vec::new();
    for n in numbers {
        let Some(itm) = world.items.get(&n) else {
            continue;
        };
        if !crate::items::inventory::matches_any_word(name, &itm.name) {
            continue;
        }
        if eq_ignore_case(&itm.name, name) {
            return (Some(n), matches.len() + 1);
        }
        matches.push((n, itm.name.clone()));
    }
    match matches.len() {
        1 => (Some(matches[0].0), 1),
        0 => (None, 0),
        n => {
            // the second candidate first, then the first, then the rest
            let mut names: Vec<String> = matches.iter().map(|m| m.1.clone()).collect();
            names.swap(0, 1);
            show_options(world, id, &names);
            (None, n)
        }
    }
}

/// The first spell, by number, whose name the text abbreviates.
fn get_spell_from_name(world: &World, name: &str) -> Option<SpellNo> {
    let mut numbers: Vec<SpellNo> = world.spells.keys().copied().collect();
    numbers.sort_by_key(|n| n.0);
    numbers.into_iter().find(|n| {
        world
            .spells
            .get(n)
            .is_some_and(|s| crate::items::inventory::matches_any_word(name, &s.name))
    })
}

/// SYSOP SUMMON <name>: an item into the pack, else a spell into the book; the record is marked
/// edited (`immortal`).
fn summon(world: &mut World, id: SessionId, line: &CommandLine) {
    let name = line.rest_from(2).to_string();
    let (item, cnt) = get_item_from_name(world, id, &name);
    if let Some(n) = item {
        let iname = world
            .items
            .get(&n)
            .map_or_else(String::new, |i| i.name.clone());
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.immortal = 1;
        }
        push(
            world,
            id,
            Msg::SysopConjured {
                name: iname.clone(),
            },
        );
        crate::items::inventory::add_item_to_inventory(world, id, n, -2);
        audit(world, id, "sysop.summon", format!("summoned item {iname}"));
    } else if let Some(sp) = get_spell_from_name(world, &name) {
        let sname = world
            .spells
            .get(&sp)
            .map_or_else(String::new, |s| s.name.clone());
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.immortal = 1;
            crate::items::equip::add_spell_to_spellbook(p, sp);
        }
        push(
            world,
            id,
            Msg::SysopConjured {
                name: sname.clone(),
            },
        );
        audit(world, id, "sysop.summon", format!("summoned spell {sname}"));
    } else if cnt == 0 {
        push(world, id, Msg::SysopNoSuchSummon);
    }
}

/// Nobody dragged, every follower of the operator told and let go, the party gone.
fn leave_party(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
    }
    crate::party::stop_following_all(world, id);
}

/// SYSOP EXPRESS_TRAIN <room> [map]: straight to a room, nothing printed.
fn express_train(world: &mut World, id: SessionId, line: &CommandLine) {
    let room = leading_int(line.arg(2));
    if room <= 0 {
        return;
    }
    let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let mut map = if line.argc() > 3 {
        leading_int(line.arg(3))
    } else {
        i64::from(here.map.0)
    };
    if map < 1 {
        map = i64::from(here.map.0);
    }
    leave_party(world, id);
    set_user_location(
        world,
        id,
        RoomKey {
            map: MapId(map as u32),
            room: RoomId(room as u32),
        },
        false,
    );
}

/// The six SYSOP GOTO places: (word, room, map, sysop only).
pub const GOTO_PLACES: [(&str, u32, u32, bool); 6] = [
    ("newhaven", 2150, 1, false),
    ("silvermere", 0, 1, false),
    ("support", 164, 1, true),
    ("rhudaur", 2519, 2, false),
    ("khazarad", 1249, 6, false),
    ("lostcity", 426, 16, false),
];

/// SYSOP GOTO <place>: no message on success. SILVERMERE is the reincarnation room (`reincroom`);
/// SUPPORT the sysop's alone. The add-on towns go without the add-on test (licensing).
fn goto(world: &mut World, id: SessionId, op: Op, line: &CommandLine) {
    goto_with(world, id, op.sysop, line, HOOKS.combat);
}

/// SYSOP GOTO with the combat state asked of `combat` (the game's `movement::HOOKS.combat`;
/// tests pass a stub until T-034 fills the hooks). `sysop`: the operator holds the sysop
/// role (SUPPORT).
pub fn goto_with(
    world: &mut World,
    id: SessionId,
    sysop: bool,
    line: &CommandLine,
    combat: &dyn crate::movement::CombatHooks,
) {
    leave_party(world, id);
    // not in a fight
    if combat.in_autocombat(world, id) || combat.being_attacked(world, id) {
        push(world, id, Msg::SysopNoAutocombat);
        return;
    }
    if combat.in_retaliation(world, id) {
        push(world, id, Msg::SysopNoTeleportRetaliation);
        return;
    }
    let word = line.arg(2);
    let place = GOTO_PLACES.iter().find(|(name, _, _, sysop_only)| {
        starts_with_ignore_case(word, name) && (!sysop_only || sysop)
    });
    match place {
        Some(&(name, room, map, _)) => {
            let room = if name == "silvermere" {
                world.config.reinc_room
            } else {
                room
            };
            let key = RoomKey {
                map: MapId(map),
                room: RoomId(room),
            };
            set_user_location(world, id, key, false);
        }
        None => push(world, id, Msg::SysopGotoLocations),
    }
}

/// A user typed as a channel number or a name (`leading_int32`, then [`get_user_number`]), for
/// LIGHTNING, VALUES and STATUS. `Err(true)`: the lookup already answered (several names);
/// `Err(false)`: nobody.
fn find_user(
    world: &mut World,
    id: SessionId,
    word: &str,
    by_number: bool,
) -> Result<SessionId, bool> {
    let lnum = leading_int(word);
    if by_number && lnum > 0 && lnum < i64::from(world.ids.max()) {
        // as in the original, an empty channel shows nothing
        let sid = SessionId(lnum as u16);
        return if world.session(sid).is_some() {
            Ok(sid)
        } else {
            Err(true)
        };
    }
    match find_user_number(world, id, word, false) {
        UserMatch::One(t) => Ok(t),
        UserMatch::Several(names) => {
            show_options(world, id, &names);
            Err(true)
        }
        UserMatch::None => Err(false),
    }
}

/// SYSOP LIGHTNING <user>: 25% to 50% of the victim's maximum hit points, for a victim at legal
/// level 5 unless `LIGHTNING_EVERYONE` (`lightevy`). A conscious victim who falls below 0 is
/// knocked out (`knock_out_user`); what follows is the cycle's.
fn lightning(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if line.argc() <= 2 {
        push(world, id, Msg::SysopLightningSyntax);
        return Handled::Done;
    }
    let target = match find_user_number(world, id, line.arg(2), false) {
        UserMatch::One(t) => Some(t),
        UserMatch::Several(names) => {
            show_options(world, id, &names);
            return Handled::Done;
        }
        UserMatch::None => None,
    };
    let Some(t) = target else {
        // nobody: nothing at all
        return Handled::Done;
    };
    let light_evy = world.config.light_evy;
    let Some(tplr) = world.session(t).and_then(|s| s.player()) else {
        push(world, id, Msg::SysopLightningRefused);
        return Handled::Done;
    };
    if !(light_evy || crate::display::who::legal_level(tplr.evil) == 5) {
        push(world, id, Msg::SysopLightningRefused);
        return Handled::Done;
    }
    let maxhp = i64::from(tplr.maxhp);
    let mut lo = maxhp * 25 / 100;
    let hi = maxhp * 50 / 100;
    if lo > hi {
        lo = hi;
    }
    // a roll from 0 to hi-lo+1 inclusive
    let span = (hi - lo + 1).max(0) as u32;
    let dmg = i64::from(world.rng.range(0, span as i32)) + lo;
    let (wasup, name, room) = {
        let tplr = world.session(t).and_then(|s| s.player()).expect("checked");
        (
            tplr.hp > 0,
            tplr.firstname.clone(),
            RoomKey {
                map: tplr.map,
                room: tplr.room,
            },
        )
    };
    let below = {
        let p = world
            .session_mut(t)
            .and_then(|s| s.character.as_mut())
            .expect("checked");
        p.hp = (i64::from(p.hp) - dmg).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
        p.hp < 0
    };
    push(world, t, Msg::SysopLightningYou { dmg });
    tell_room(
        world,
        room,
        [Some(t), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::SysopLightningRoom { name: name.clone() },
    );
    if wasup && below {
        if let Some(p) = world.session_mut(t).and_then(|s| s.character.as_mut()) {
            p.status.remove(PlayerStatus::RESTING);
            p.status2.remove(PlayerStatus2::MEDITATING);
        }
        (world.cycles.hooks.knock_out_user)(world, t);
    }
    let by = world
        .session(id)
        .and_then(|s| s.player())
        .map_or_else(String::new, |p| p.userid.clone());
    audit(
        world,
        id,
        "sysop.lightning",
        format!("Lightning struck: {name} by {by}"),
    );
    push(world, id, Msg::SysopDone);
    Handled::Done
}

/// SYSOP VALUES [user]: `display_attack_values`, the player's AV/DV line.
fn values(world: &mut World, id: SessionId, line: &CommandLine) {
    let target = if line.argc() > 2 {
        match find_user(world, id, line.arg(2), true) {
            Ok(t) => t,
            Err(true) => return,
            Err(false) => {
                push(
                    world,
                    id,
                    Msg::SysopCannotFindUser {
                        name: line.arg(2).to_string(),
                    },
                );
                return;
            }
        }
    } else {
        id
    };
    crate::display::sysop::display_attack_values(world, target, id);
}

/// SYSOP STATUS [ROOM <room> [map] | <user>].
fn status(world: &mut World, id: SessionId, line: &CommandLine) {
    let argc = line.argc();
    if argc > 2 {
        if eq_ignore_case(line.arg(2), "room") && argc > 3 {
            let room = leading_int(line.arg(3)) as u32;
            let map = if argc == 5 {
                leading_int(line.arg(4)) as u32
            } else {
                1
            };
            crate::display::sysop::display_debug_room_stats(world, id, room, map);
        } else {
            match find_user(world, id, line.arg(2), true) {
                Ok(t) => crate::display::status::show_status(world, t, id),
                Err(true) => {}
                Err(false) => push(
                    world,
                    id,
                    Msg::SysopCannotFindUser {
                        name: line.arg(2).to_string(),
                    },
                ),
            }
        }
    } else {
        crate::display::sysop::display_debug_room_stats(world, id, 0, 0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GangCmd {
    Disband,
    Disable,
    Enable,
}

/// DISBAND, DISABLE and ENABLE: a gang flag, saved at once (the gang marked modified).
fn gang_flag(world: &mut World, id: SessionId, line: &CommandLine, cmd: GangCmd) {
    push(world, id, Msg::SysopClrline);
    if line.argc() == 2 {
        push(
            world,
            id,
            match cmd {
                GangCmd::Disband => Msg::SysopDisbandSyntax,
                GangCmd::Disable => Msg::SysopDisableSyntax,
                GangCmd::Enable => Msg::SysopEnableSyntax,
            },
        );
        return;
    }
    let name = line.rest_from(2).to_string();
    if crate::gangs::get_gang(world, &name).is_none() {
        push(world, id, Msg::SysopGangNotFound { gang: name });
        return;
    }
    if let Some(g) = crate::gangs::get_gang_mut(world, &name) {
        match cmd {
            GangCmd::Disband => g.flags.insert(GangFlags::DISBANDED | GangFlags::DELETE),
            GangCmd::Disable => g.flags.insert(GangFlags::HIDDEN),
            GangCmd::Enable => g.flags.remove(GangFlags::HIDDEN),
        }
    }
    crate::gangs::save_gang(world, &name);
    let (msg, kind) = match cmd {
        GangCmd::Disband => (
            Msg::SysopGangDisbanded { gang: name.clone() },
            "sysop.disband",
        ),
        GangCmd::Disable => (
            Msg::SysopGangDisabled { gang: name.clone() },
            "sysop.disable",
        ),
        GangCmd::Enable => (Msg::SysopGangEnabled { gang: name.clone() }, "sysop.enable"),
    };
    push(world, id, msg);
    audit(world, id, kind, format!("gang {name}"));
}

/// SYSOP GANGSIZE <n> <gang>.
fn gangsize(world: &mut World, id: SessionId, line: &CommandLine) {
    if line.argc() < 4 {
        push(world, id, Msg::SysopGangsizeSyntax);
        return;
    }
    let name = line.rest_from(3).to_string();
    let n = leading_int(line.arg(2));
    if crate::gangs::get_gang(world, &name).is_none() {
        push(world, id, Msg::SysopGangNotFound { gang: name });
        return;
    }
    if n == 0 {
        push(world, id, Msg::SysopGangsizeZero);
        return;
    }
    let mut members = 0;
    if let Some(g) = crate::gangs::get_gang_mut(world, &name) {
        g.members = n as i16;
        members = i64::from(g.members);
    }
    crate::gangs::save_gang(world, &name);
    push(
        world,
        id,
        Msg::SysopGangResized {
            gang: name.clone(),
            n: members,
        },
    );
    audit(
        world,
        id,
        "sysop.gangsize",
        format!("gang {name} resized to {members}"),
    );
}

/// SYSOP SPEAK <text>: a line to everybody in the game, no colour.
fn speak(world: &mut World, id: SessionId, line: &CommandLine) {
    let text = line.rest_from(2).to_string();
    tell_game(
        world,
        Some(id),
        PlayerFlags::empty(),
        true,
        None,
        &Msg::SysopSpeakLine { text: text.clone() },
    );
    push(world, id, Msg::SysopSpoken);
    audit(world, id, "sysop.speak", text);
}

/// Anyone in the game in a Colliseum room.
fn arena_has_users(world: &World) -> bool {
    world.session_ids().into_iter().any(|i| {
        world.is_in_game(i)
            && world
                .session(i)
                .and_then(|s| s.room_key())
                .and_then(|k| world.room(k))
                .is_some_and(|r| r.room_type == RoomType::Colliseum)
    })
}

/// SYSOP ARENA [STATUS | NORMAL | COMBAT] (`arenasafe`).
fn arena(world: &mut World, id: SessionId, line: &CommandLine) {
    push(world, id, Msg::SysopClrline);
    let argc = line.argc();
    let safe = world.display.arena_safe;
    let msg = if argc == 2 || (argc == 3 && starts_with_ignore_case(line.arg(2), "status")) {
        if safe {
            Msg::SysopArenaNormalMode
        } else {
            Msg::SysopArenaCombatMode
        }
    } else if argc == 3 {
        if starts_with_ignore_case(line.arg(2), "normal") {
            if safe {
                Msg::SysopArenaAlreadyNormal
            } else if arena_has_users(world) {
                Msg::SysopArenaOccupied
            } else {
                world.display.arena_safe = true;
                audit(world, id, "sysop.arena", "arena rooms now normal".into());
                Msg::SysopArenaNowNormal
            }
        } else if starts_with_ignore_case(line.arg(2), "combat") {
            if !safe {
                Msg::SysopArenaAlreadyCombat
            } else if arena_has_users(world) {
                Msg::SysopArenaOccupied
            } else {
                world.display.arena_safe = false;
                audit(world, id, "sysop.arena", "arena rooms now combat".into());
                Msg::SysopArenaNowCombat
            }
        } else {
            Msg::SysopArenaSyntax
        }
    } else {
        Msg::SysopArenaSyntax
    };
    push(world, id, msg);
}

/// The target of a GOD command, mutable.
fn target_mut(
    world: &mut World,
    t: SessionId,
) -> Option<&mut silvermere_types::records::Character> {
    world.session_mut(t).and_then(|s| s.character.as_mut())
}

fn target_name(world: &World, t: SessionId) -> (String, String) {
    world
        .session(t)
        .and_then(|s| s.player())
        .map_or_else(Default::default, |p| {
            (p.firstname.clone(), p.userid.clone())
        })
}

/// SYSOP GOD <user> <command> ...
fn god(world: &mut World, id: SessionId, op: Op, line: &CommandLine) -> Handled {
    let argc = line.argc();
    if argc == 2 {
        push(world, id, Msg::SysopGodSyntax);
        return Handled::Done;
    }
    // a channel number, or a name
    let lnum = leading_int(line.arg(2));
    let found = if lnum > 0 {
        if lnum < i64::from(world.ids.max()) {
            Some(SessionId(lnum as u16))
        } else {
            None
        }
    } else {
        match find_user_number(world, id, line.arg(2), false) {
            UserMatch::One(t) => Some(t),
            UserMatch::Several(names) => {
                show_options(world, id, &names);
                return Handled::Done;
            }
            UserMatch::None => None,
        }
    };
    let Some(t) = found else {
        push(
            world,
            id,
            Msg::SysopCannotFindUser {
                name: line.arg(2).to_string(),
            },
        );
        return Handled::Done;
    };
    // only a player in the game; nothing said otherwise
    if !world.is_in_game(t) {
        return Handled::Done;
    }
    let w3 = line.arg(3);
    let w4 = line.arg(4);
    if argc < 5 {
        god_short(world, id, op, t, line);
    } else if starts_with_ignore_case(w3, "add") {
        if starts_with_ignore_case(w4, "life") && op.has(Key::SysAddLife) {
            let (tname, tuser) = target_name(world, t);
            let _ = tname;
            let lives = world.session(t).and_then(|s| s.player()).map(|p| p.lives);
            if lives.is_some_and(|l| l < 9) {
                let by = world
                    .session(id)
                    .and_then(|s| s.player())
                    .map_or_else(String::new, |p| p.userid.clone());
                audit(
                    world,
                    id,
                    "sysop.god",
                    format!("{by} added life to {tuser}"),
                );
                if let Some(p) = target_mut(world, t) {
                    p.lives += 1;
                }
                push(world, id, Msg::SysopDone);
            }
        } else if starts_with_ignore_case(w4, "evil") && op.has(Key::SysEditEvil) {
            let add = (argc == 6).then(|| leading_int(line.arg(5)));
            if let Some(p) = target_mut(world, t) {
                match add {
                    Some(n) => p.evil = (i64::from(p.evil) + n) as i16,
                    None => p.evil = 0,
                }
            }
            let (_, tuser) = target_name(world, t);
            audit(
                world,
                id,
                "sysop.god",
                format!("add evil {add:?} to {tuser}"),
            );
            push(world, id, Msg::SysopDone);
        } else {
            push(world, id, Msg::SysopNoAdd);
        }
    } else if starts_with_ignore_case(w3, "clear") {
        if starts_with_ignore_case(w4, "suicide") && op.has(Key::SysClearSuicide) {
            if let Some(p) = target_mut(world, t) {
                p.suicpw.clear();
            }
            let (_, tuser) = target_name(world, t);
            audit(
                world,
                id,
                "sysop.god",
                format!("cleared suicide password of {tuser}"),
            );
            push(world, id, Msg::SysopDone);
        } else {
            push(world, id, Msg::SysopNoClear);
        }
    } else if starts_with_ignore_case(w3, "speak") && op.has(Key::SysSpeak) {
        // the gods speak to the player
        let text = line.rest_from(4).to_string();
        push(world, t, Msg::GodsTell { text: text.clone() });
        crate::movement::exits::prompt(world, t);
        push(world, id, Msg::SysopSpoken);
        let (_, tuser) = target_name(world, t);
        audit(world, id, "sysop.god", format!("spoke to {tuser}: {text}"));
    } else if starts_with_ignore_case(w3, "change") && op.has(Key::SysChange) {
        god_change(world, id, t, line);
    } else if starts_with_ignore_case(w3, "disable") {
        if starts_with_ignore_case(w4, "topten") && op.has(Key::SysTopTen) {
            // (sic) nothing printed
            if let Some(p) = target_mut(world, t) {
                p.settings.insert(PlayerSettings::NOTOPTEN);
            }
            let (_, tuser) = target_name(world, t);
            audit(
                world,
                id,
                "sysop.god",
                format!("{tuser} left out of topten"),
            );
        } else {
            push(world, id, Msg::SysopNoDisable);
        }
    } else if starts_with_ignore_case(w3, "enable") {
        if starts_with_ignore_case(w4, "topten") && op.has(Key::SysTopTen) {
            if let Some(p) = target_mut(world, t) {
                p.settings.remove(PlayerSettings::NOTOPTEN);
            }
            let (_, tuser) = target_name(world, t);
            audit(world, id, "sysop.god", format!("{tuser} back in topten"));
            push(world, id, Msg::SysopDone);
        } else {
            push(world, id, Msg::SysopNoEnable);
        }
    } else if starts_with_ignore_case(w3, "verify") {
        // the old-ability conversion: the records are already in the 1.11 form (the importer
        // converts them), so there is nothing left to convert.
        if starts_with_ignore_case(w4, "abilities") {
            push(world, id, Msg::SysopDone);
        } else {
            push(world, id, Msg::SysopVerifyOnly);
        }
    } else {
        push(world, id, Msg::SysopNoGod);
    }
    Handled::Done
}

/// The four-word GOD commands: the displays and the alignment.
fn god_short(world: &mut World, id: SessionId, op: Op, t: SessionId, line: &CommandLine) {
    let four = line.argc() == 4;
    let w3 = line.arg(3);
    let info = op.has(Key::SysUserInfo);
    let align = op.has(Key::SysAlignment);
    let is = |word: &str| four && starts_with_ignore_case(w3, word);
    if is("inventory") && info {
        crate::display::inventory::display_inventory_items(world, t, id);
    } else if is("profile") && info {
        crate::display::profile::display_profile_of(world, t, id);
    } else if is("spells") && info {
        crate::display::spells::display_users_spells(world, t, id, Cmd::Spells);
    } else if is("powers") && info {
        crate::display::spells::display_users_spells(world, t, id, Cmd::Powers);
    } else if is("abilities") && info {
        crate::display::sysop::display_user_abilities(world, t, id);
    } else if is("status") && info {
        crate::display::status::show_status(world, t, id);
    } else if is("info") && info {
        // the status page, the BBS user-id, the inventory; the rest one part a poll
        crate::display::status::show_status(world, t, id);
        let (_, tuser) = target_name(world, t);
        push(world, id, Msg::SysopBbsUserid { userid: tuser });
        crate::display::inventory::display_inventory_items(world, t, id);
        world.sysop.info.insert(id, (0, t));
    } else if is("retrain") && op.has(Key::SysRetrain) {
        // (CP -1 marks a retrain for the stat editor at the next login)
        if let Some(p) = target_mut(world, t) {
            p.cp = -1;
        }
        let (tname, tuser) = target_name(world, t);
        push(world, id, Msg::SysopRetrain { name: tname });
        audit(world, id, "sysop.god", format!("{tuser} must retrain"));
    } else if is("good") && align {
        set_alignment(world, id, t, None, -51, "good");
    } else if is("saint") && align {
        set_alignment(world, id, t, None, -201, "saint");
    } else if is("neutral") && align {
        set_alignment(world, id, t, None, 0, "neutral");
    } else if is("lawful") && align {
        set_alignment(world, id, t, Some(-10), -51, "lawful");
    } else if is("unlawful") && align {
        // lawful 0, evil at least 40
        let evil = world
            .session(t)
            .and_then(|s| s.player())
            .map_or(0, |p| p.evil.max(40));
        set_alignment(world, id, t, Some(0), evil, "unlawful");
    } else if is("jail") && op.has(Key::SysJail) {
        // text block 1510 run as a special command for the player.
        crate::display::sysop::perform_sysop_text_block(world, t);
        let (_, tuser) = target_name(world, t);
        audit(world, id, "sysop.god", format!("{tuser} jailed"));
        push(world, id, Msg::SysopDone);
    } else {
        push(world, id, Msg::SysopGodValidOptions);
    }
}

fn set_alignment(
    world: &mut World,
    id: SessionId,
    t: SessionId,
    lawful: Option<i8>,
    evil: i16,
    what: &str,
) {
    if let Some(p) = target_mut(world, t) {
        if let Some(l) = lawful {
            p.lawful = l;
        }
        p.evil = evil;
    }
    let (_, tuser) = target_name(world, t);
    audit(world, id, "sysop.god", format!("{tuser} made {what}"));
    push(world, id, Msg::SysopDone);
}

/// A player in the game with this first name.
fn name_in_use(world: &World, name: &str) -> bool {
    world.session_ids().into_iter().any(|i| {
        world.is_in_game(i)
            && world
                .session(i)
                .and_then(|s| s.player())
                .is_some_and(|p| eq_ignore_case(&p.firstname, name))
    })
}

/// GOD <user> CHANGE NAME <name> | GANG <name> | SEX.
fn god_change(world: &mut World, id: SessionId, t: SessionId, line: &CommandLine) {
    let argc = line.argc();
    let w4 = line.arg(4);
    let (tname, tuser) = target_name(world, t);
    if argc == 6 && starts_with_ignore_case(w4, "name") {
        let new = line.arg(5).to_string();
        if new.len() > 10 {
            push(world, id, Msg::SysopNameTooLong);
        } else if name_in_use(world, &new) || leading_int(&new) != 0 {
            push(world, id, Msg::SysopNameRefused);
        } else {
            if let Some(p) = target_mut(world, t) {
                p.firstname = new.clone();
            }
            // no user list to rebuild: the WHO order is computed from the names on demand
            push(world, id, Msg::SysopNameChanged);
            audit(
                world,
                id,
                "sysop.god",
                format!("{tuser} renamed from {tname} to {new}"),
            );
        }
    } else if argc > 5 && starts_with_ignore_case(w4, "gang") {
        let new: String = line.rest_from(5).chars().take(20).collect();
        let old = world
            .session(t)
            .and_then(|s| s.player())
            .map_or_else(String::new, |p| p.gang.clone());
        if let Some(p) = target_mut(world, t) {
            p.gang = new.clone();
        }
        push(
            world,
            id,
            Msg::SysopGangChanged {
                name: tname,
                old: old.clone(),
                new: new.clone(),
            },
        );
        audit(
            world,
            id,
            "sysop.god",
            format!("{tuser} gang {old} -> {new}"),
        );
    } else if argc >= 5 && starts_with_ignore_case(w4, "sex") {
        let female = world
            .session(t)
            .and_then(|s| s.player())
            .is_some_and(|p| p.sex == b'F');
        if let Some(p) = target_mut(world, t) {
            p.sex = if female { b'M' } else { b'F' };
        }
        push(
            world,
            id,
            if female {
                Msg::SysopSexMale { name: tname }
            } else {
                Msg::SysopSexFemale { name: tname }
            },
        );
        audit(world, id, "sysop.god", format!("{tuser} sex changed"));
    } else {
        push(world, id, Msg::SysopNoChange);
    }
}

/// SYSOP LIST <what> [...].
fn list(world: &mut World, id: SessionId, op: Op, line: &CommandLine) -> Handled {
    use crate::display::sysop as d;
    let argc = line.argc();
    if argc <= 2 {
        push(world, id, Msg::SysopValidLists);
        if op.has(Key::Debug) && op.sysop {
            push(world, id, Msg::SysopValidListsDev);
        }
        return Handled::Done;
    }
    let w = line.arg(2);
    let users = op.has(Key::SysListUser);
    let limit = op.has(Key::SysListLimit);
    if op.has(Key::Debug) && op.sysop {
        // the developer lists
        if starts_with_ignore_case(w, "items") {
            d::display_list_of_all_items(world, id);
        } else if starts_with_ignore_case(w, "known_monsters") {
            d::display_list_of_all_known_monsters(world, id);
        } else if starts_with_ignore_case(w, "newlimited") && limit {
            d::display_new_list_of_limited_items(world, id, line);
        } else if starts_with_ignore_case(w, "limited_items") && limit {
            d::display_list_of_limited_items(world, id);
        } else if starts_with_ignore_case(w, "active_monsters") {
            let first = if argc == 3 {
                1
            } else {
                leading_int(line.arg(3))
            };
            let max = if argc <= 4 {
                10
            } else {
                leading_int(line.arg(4))
            };
            d::display_list_of_all_active_monsters(world, id, first, max);
        } else if starts_with_ignore_case(w, "spells") {
            d::display_list_of_all_spells(world, id);
        } else if starts_with_ignore_case(w, "users") && users {
            d::display_scan_of_users(world, id, 1);
        } else if starts_with_ignore_case(w, "evil") && users {
            d::display_scan_of_users(world, id, 2);
        } else if starts_with_ignore_case(w, "text") {
            if argc == 4 {
                // the text block numbered by the fourth word, no substitution
                crate::display::long_text::display_long_text(
                    world,
                    id,
                    TextBlockNo(leading_int(line.arg(3)) as i32),
                    None,
                );
            }
        } else if starts_with_ignore_case(w, "restock") {
            let shop = if argc > 3 {
                leading_int(line.arg(3))
            } else {
                0
            };
            d::display_all_restocking_required(world, id, shop);
        } else if starts_with_ignore_case(w, "gang") && limit {
            if argc >= 4 {
                // no prompt: the paged list (T-044) ends with its own prompt
                crate::display::gang::display_gang_members(world, id, line.rest_from(3));
                return Handled::Wait;
            }
        } else {
            push(world, id, Msg::SysopNoListDev);
        }
    } else if starts_with_ignore_case(w, "users") && users {
        d::display_scan_of_users(world, id, 1);
    } else if starts_with_ignore_case(w, "evil") && users {
        d::display_scan_of_users(world, id, 2);
    } else if starts_with_ignore_case(w, "newlimited") && limit {
        d::display_new_list_of_limited_items(world, id, line);
    } else if starts_with_ignore_case(w, "limited_items") && limit {
        d::display_list_of_limited_items(world, id);
    } else if argc >= 4 && starts_with_ignore_case(w, "gang") && limit {
        // no prompt, as above
        crate::display::gang::display_gang_members(world, id, line.rest_from(3));
        return Handled::Wait;
    } else {
        push(world, id, Msg::SysopNoList);
    }
    Handled::Done
}

/// SYSOP REPORT [USERS [filter] | NOTOPTEN | MEMORY].
fn report(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    let argc = line.argc();
    if argc == 2 {
        push(world, id, Msg::SysopReportOptions);
        return Handled::Done;
    }
    let w = line.arg(2);
    if starts_with_ignore_case(w, "users") {
        let filter = (argc == 4).then(|| line.arg(3).to_string());
        crate::display::sysop::display_sysop_user_report(world, id, 0, filter);
        return Handled::Wait;
    } else if starts_with_ignore_case(w, "notopten") {
        crate::display::sysop::display_sysop_user_report(world, id, 1, None);
        return Handled::Wait;
    } else if starts_with_ignore_case(w, "memory") {
        // (sic) the original shows nothing; the process's resident set instead
        push(world, id, Msg::SysopReportMemory { kb: resident_kb() });
    }
    Handled::Done
}

/// The resident set of this process in KB, from `/proc/self/status`; 0 elsewhere.
fn resident_kb() -> i64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(|l| leading_int(l.trim_start_matches("VmRSS:")))
        })
        .unwrap_or(0)
}

/// SYSOP CONFIGURE SHOW | GENRATE <n> | MINWAIT <n> | MAXUSERS <n>.
fn configure(world: &mut World, id: SessionId, op: Op, line: &CommandLine) {
    let argc = line.argc();
    let dev = op.has(Key::Debug) && op.sysop;
    if argc == 3 {
        if starts_with_ignore_case(line.arg(2), "show") {
            push(world, id, Msg::SysopConfigHeader);
            if dev {
                push(
                    world,
                    id,
                    Msg::SysopConfigGenrate {
                        secs: i64::from(world.config.sec_create),
                    },
                );
                push(
                    world,
                    id,
                    Msg::SysopConfigRegen {
                        mins: i64::from(world.config.regen_mins),
                    },
                );
            }
            // pollrate/initrate set the record caches' pace: there are none (0)
            push(
                world,
                id,
                Msg::SysopConfigBody {
                    max: i64::from(world.max_in_game),
                    poll: 0,
                    init: 0,
                },
            );
            if world.config.track_limited {
                push(world, id, Msg::SysopConfigTracking);
            }
            push(
                world,
                id,
                Msg::SysopConfigLogfile {
                    file: "tracing".into(),
                },
            );
        } else {
            push(world, id, Msg::SysopNoConfigure);
        }
    } else if argc > 3 {
        let w = line.arg(2);
        let n = leading_int(line.arg(3));
        if dev && starts_with_ignore_case(w, "genrate") {
            if n > 0 {
                world.config.sec_create = n as u32;
                push(world, id, Msg::SysopGenrateSet { secs: n });
                audit(world, id, "sysop.configure", format!("genrate {n}"));
            }
        } else if dev && starts_with_ignore_case(w, "minwait") {
            if n >= 0 {
                world.config.regen_mins = n as u32;
                push(world, id, Msg::SysopMinwaitSet { mins: n });
                audit(world, id, "sysop.configure", format!("minwait {n}"));
            }
        } else if starts_with_ignore_case(w, "maxusers") {
            // between 1 and the session slots (`licusers`)
            let max = n.min(i64::from(world.ids.max())).max(1);
            world.max_in_game = max as u16;
            push(world, id, Msg::SysopMaxusersSet { max });
            audit(world, id, "sysop.configure", format!("maxusers {max}"));
        } else {
            push(world, id, Msg::SysopInvalidOption);
        }
    } else if dev {
        push(world, id, Msg::SysopConfigureOptionsDev);
    } else {
        push(world, id, Msg::SysopConfigureOptions);
    }
}

/// SYSOP TWEAK CLEAR | <field> <value>: the operator's own record.
fn tweak(world: &mut World, id: SessionId, line: &CommandLine) {
    let argc = line.argc();
    if argc == 3 {
        if starts_with_ignore_case(line.arg(2), "clear") {
            tweak_clear(world, id);
            push(world, id, Msg::SysopTweakDone);
        } else {
            push(world, id, Msg::SysopTweakNoCommand);
        }
        return;
    }
    if argc <= 3 {
        push(world, id, Msg::SysopValidEdits);
        return;
    }
    let field = line.arg(2);
    let n = leading_int(line.arg(3));
    let mut out = None;
    let mut add_exp = None;
    {
        let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return;
        };
        let stat = |n: i64| n as u16;
        let mut set = true;
        if starts_with_ignore_case(field, "hp") {
            if n > 0 {
                p.hp = n as i16;
                p.maxhp = n as i16;
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "randomhp") {
            if n > 0 {
                out = Some(Msg::SysopTweakBefore {
                    n: i64::from(p.randomhp),
                });
                p.randomhp = n as u8;
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "intellect") {
            if n > 0 {
                (p.intel, p.basint) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "wisdom") {
            if n > 0 {
                (p.wil, p.baswil) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "strength") {
            if n > 0 {
                (p.str, p.basstr) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "health") {
            if n > 0 {
                (p.hea, p.bashea) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "agility") {
            if n > 0 {
                (p.agi, p.basagi) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "charm") {
            if n > 0 {
                (p.cha, p.bascha) = (stat(n), stat(n));
            } else {
                set = false;
            }
        } else if starts_with_ignore_case(field, "experience") {
            add_exp = Some(n);
        } else if starts_with_ignore_case(field, "light") {
            p.light = (i64::from(p.light) + n) as i16;
            out = Some(Msg::SysopTweakLight {
                n: i64::from(p.light),
            });
        } else if starts_with_ignore_case(field, "lives") {
            p.lives = n as i16;
        } else if starts_with_ignore_case(field, "level") {
            p.level = n as u16;
        } else if starts_with_ignore_case(field, "class") {
            p.class = silvermere_types::ids::ClassNo(n as u16);
        } else if starts_with_ignore_case(field, "race") {
            if n == 0 {
                p.flags.remove(PlayerFlags::RACECHANGED);
                p.racechg = 0;
            } else {
                p.race = silvermere_types::ids::RaceNo(n as u16);
            }
        } else if starts_with_ignore_case(field, "revision") {
            p.version = n as i16;
        } else if starts_with_ignore_case(field, "mana") {
            p.maxmana = n as u16;
            p.mana = n as i16;
        } else if starts_with_ignore_case(field, "copper") {
            p.coins.copper = (i64::from(p.coins.copper) + n) as u32;
        } else if starts_with_ignore_case(field, "gold") {
            p.coins.gold = (i64::from(p.coins.gold) + n) as u32;
        } else if starts_with_ignore_case(field, "userdefined") {
            p.coins.runic = (i64::from(p.coins.runic) + n) as u32;
        } else if starts_with_ignore_case(field, "evil") {
            p.evil = (i64::from(p.evil) + n) as i16;
        } else {
            set = false;
            out = Some(Msg::SysopTweakNoChange);
        }
        if set {
            p.immortal = 1;
        }
    }
    if let Some(m) = out {
        push(world, id, m);
    }
    if let Some(n) = add_exp {
        crate::progression::add_experience(world, id, n as i32, true);
    }
    // the secondary stats again, the maximum hit points kept
    if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
        crate::player::recalculate(world, &mut c, crate::player::CalcMode::KeepMaxHp);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
    }
    audit(world, id, "sysop.tweak", line.rest_from(2).to_string());
}

/// TWEAK CLEAR: the pack, the worn items and the keys emptied (limited copies released, item
/// abilities taken off), the weight and every coin zeroed.
fn tweak_clear(world: &mut World, id: SessionId) {
    let Some((userid, items, worn, keys)) = world.session(id).and_then(|s| s.player()).map(|p| {
        (
            p.userid.clone(),
            p.items.clone(),
            p.worn.to_vec(),
            p.keys.clone(),
        )
    }) else {
        return;
    };
    for n in items.iter().filter(|n| n.0 != 0) {
        crate::items::limited::release(world, *n, crate::items::limited::Holder::User(&userid));
    }
    for n in worn.iter().filter(|n| n.0 != 0) {
        if let Some(itm) = world.items.get(n).cloned() {
            crate::items::equip::unequip_apply_item_abilities(world, id, &itm);
        }
    }
    for n in keys.iter().filter(|n| n.0 != 0) {
        if let Some(itm) = world.items.get(n).cloned() {
            crate::items::equip::unequip_apply_item_abilities(world, id, &itm);
        }
        crate::items::limited::release(world, *n, crate::items::limited::Holder::User(&userid));
    }
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.items.iter_mut().for_each(|n| *n = ItemNo(0));
        p.worn.iter_mut().for_each(|n| *n = ItemNo(0));
        p.keys.iter_mut().for_each(|n| *n = ItemNo(0));
        p.keyuses.iter_mut().for_each(|u| *u = -2);
        p.wgt = 0;
        p.coins = Default::default();
    }
    audit(world, id, "sysop.tweak", "clear".into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_int_reads_sign_and_digits() {
        assert_eq!(leading_int("12abc"), 12);
        assert_eq!(leading_int("  -4"), -4);
        assert_eq!(leading_int("x"), 0);
        assert_eq!(leading_int(""), 0);
    }

    #[test]
    fn keys_name_every_perm() {
        assert_eq!(Key::ALL.len(), 22);
        assert_eq!(Key::from_ini_name("sys_goto_key"), Some(Key::SysGoto));
        assert_eq!(Key::from_ini_name("GOSSIP"), None);
    }

    #[test]
    fn default_key_sets() {
        let sysop = Keys::for_roles(true, false);
        assert!(
            Key::ALL
                .iter()
                .all(|k| sysop.contains(*k) == (*k != Key::Debug))
        );
        let gameop = Keys::for_roles(false, true);
        assert!(!gameop.contains(Key::Debug));
        assert!(!gameop.contains(Key::SysDisband));
        assert!(gameop.contains(Key::SysGoto));
        assert_eq!(Keys::for_roles(false, false), Keys::empty());
    }

    // the dropped list Done-when names
    #[test]
    fn dropped_keywords() {
        let dropped: Vec<&str> = KEYWORDS
            .iter()
            .filter(|(_, d)| matches!(d, Disposition::Dropped(_)))
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(
            dropped,
            [
                "lance",
                "initialize",
                "EMULATE",
                "NOEMULATE",
                "menu U",
                "menu B",
                "menu E",
                "menu A"
            ]
        );
    }
}
