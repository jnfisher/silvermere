//! `PlayerSession`: one connection's game state, the typed replacement for the VDA.
//!
//! A connection's player record, its session-only state, the stat editor's CP and the group
//! fields are one struct per [`SessionId`], and group, leader and dragging live here rather
//! than in `Character`.

use std::collections::VecDeque;
use std::time::Instant;

use silvermere_types::enums::{DebugLevel, InGame, MessageLevel, SessionState, StatlineMode, uls};
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{
    AccountId, ClassNo, ItemNo, MapId, RaceNo, RoomId, RoomKey, SessionId, SpellNo,
};
use silvermere_types::records::{Character, Coins, UserFlags, UserState};
use silvermere_types::sizes::{NABILITY, NINVITEMS, NKEYS, NSPELLBOOK, NSPELLCAST, NTRAIL, NWORN};

use crate::output::OutputBatch;
use crate::state::in_realm;

/// `NPENDPOOL`: queued command lines per session.
pub const PENDING_CAP: usize = 40;
/// `ACQSIZE`: slots of the auto-combat ring queue.
pub const ACQSIZE: usize = 550;
/// `ACQ_EMPTY`: a free (or killed) ring slot.
pub const ACQ_EMPTY: i16 = -1;
/// `ACQ_ROUNDEND`: marks the end of a round in the ring.
pub const ACQ_ROUNDEND: i16 = -2;

/// The account behind a session, as the engine needs it (the record is `silvermere-auth`'s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRef {
    pub id: AccountId,
    pub username: String,
    /// `haskey(SYSOPKEY)`: the sysop role.
    pub sysop: bool,
    /// `haskey(GAMOPKEY)`: the game operator role.
    pub gameop: bool,
    /// `usracc.sex`, `b'M'` or `b'F'`: `roll_stats` copies it into the character.
    pub sex: u8,
}

/// What the transport knows about the terminal (`usracc.scnwid/scnbrk`, `ansifl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub columns: u16,
    pub rows: u16,
    /// Sessions are always ANSI; kept for the menu's `*ANSI RECOMMENDED*`.
    pub ansi: bool,
    /// `set utf8 on` or a CHARSET negotiation: output in UTF-8 instead of CP437.
    pub utf8: bool,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            ansi: true,
            utf8: false,
        }
    }
}

pub use crate::party::Party;

/// The auto-combat ring queue (`ACQSIZE` slots): the order the
/// players and monsters act in during a combat round. T-034 fills the logic; the ring
/// and its markers are here so the size is fixed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoCombat {
    /// Slot values: a channel, `ACQ_EMPTY` or `ACQ_ROUNDEND`.
    pub ring: Vec<i16>,
    /// Next slot to run.
    pub head: usize,
    /// Next free slot.
    pub tail: usize,
}

impl Default for AutoCombat {
    fn default() -> Self {
        Self {
            ring: vec![ACQ_EMPTY; ACQSIZE],
            head: 0,
            tail: 0,
        }
    }
}

/// One session.
#[derive(Debug, Clone)]
pub struct PlayerSession {
    pub id: SessionId,
    /// `None` until the login dialogue (T-021) binds an account.
    pub account: Option<AccountRef>,
    /// The loaded character; `None` while no character is loaded.
    pub character: Option<Character>,
    /// The character is complete and may be saved: set by `load_player` and by character
    /// creation (T-022) once the stats are spent.
    pub created: bool,
    /// The session state.
    pub state: SessionState,
    /// The per-session flags.
    pub flags: UserFlags,
    /// The per-session play state, including the pending queue and the forget list.
    pub ust: UserState,
    /// The stat editor's CP, 100 = no editor session.
    pub editor_cp: u8,
    /// The party fields that were channel numbers in the player record.
    pub party: Party,
    /// The auto-combat ring for this player's combat rounds.
    pub autocmb: AutoCombat,
    /// The line-mode character editor while it is open (T-022).
    pub editor: Option<crate::state::newchar::stat_editor::StatEditor>,
    /// Output waiting for the end of the event.
    pub output: OutputBatch,
    pub caps: Caps,
    /// Where the session is: outside, at the menus, or in the realm.
    pub ingame: InGame,
    /// When the last input arrived.
    pub idle_since: Instant,
    /// At the BBS menu shim's `Are you sure (Y/N, or R to re-logon)?` (T-021).
    pub bbs_confirm: bool,
    /// The background cycles' per-session counters (T-037).
    pub ticks: crate::cycles::SessionTicks,
    /// The `SYS_*` keys and the developer gate the account holds (T-055).
    pub sysop_keys: crate::commands::sysop::Keys,
}

impl PlayerSession {
    /// `prf_prompt`: queues this session's prompt as it reads now.
    pub fn push_prompt(&mut self) {
        let state = crate::tell::PromptState::capture(self);
        self.output.push_prompt(state);
    }

    /// A session that has just connected: nothing loaded, at [`SessionState::Start`].
    pub fn new(id: SessionId, caps: Caps, now: Instant) -> Self {
        Self {
            id,
            account: None,
            character: None,
            created: false,
            state: SessionState::Start,
            flags: UserFlags {
                debug: DebugLevel::Off,
                sysop: false,
                gameop: false,
                hasgoss: false,
                hasauct: false,
                listslot: uls::NOTPLAYING,
                ingame: InGame::Outside,
            },
            ust: UserState::default(),
            editor_cp: 100,
            party: Party::default(),
            autocmb: AutoCombat::default(),
            editor: None,
            output: OutputBatch::default(),
            caps,
            ingame: InGame::Outside,
            idle_since: now,
            bbs_confirm: false,
            ticks: crate::cycles::SessionTicks::default(),
            sysop_keys: crate::commands::sysop::Keys::empty(),
        }
    }

    /// The logon reset of the login callback: the channel
    /// starts with a clean game state (`clear_player`, no pending lines, no inviter, no
    /// forget list, no editor session). The account and terminal are kept.
    pub fn reset_for_logon(&mut self) {
        self.character = None;
        self.created = false;
        self.party = Party::default();
        self.ust.pending.clear();
        self.ust.inviter.clear();
        self.ust.forgets.clear();
        self.editor_cp = 100;
        self.editor = None;
    }

    /// Whether a terminal user is in the game: in a play state with a record
    /// loaded. Worldgroup client/server channels (`ISGCSU`, `ingame` at `UIG_REALM`) are
    /// not carried; `ingame` is kept as a field only.
    pub fn is_in_game(&self) -> bool {
        self.character.is_some() && in_realm(self.state)
    }

    /// The character, or `None` when none is loaded.
    pub fn player(&self) -> Option<&Character> {
        self.character.as_ref()
    }

    /// Queues a command line (`add_delayed_command`); `false` when the 40 slots are full
    /// (the flood texts are T-024's).
    pub fn push_pending(&mut self, text: String) -> bool {
        if self.ust.pending.len() >= PENDING_CAP {
            return false;
        }
        self.ust
            .pending
            .push(silvermere_types::records::PendingCommand { text });
        true
    }

    /// Takes the oldest queued command line.
    pub fn pop_pending(&mut self) -> Option<String> {
        if self.ust.pending.is_empty() {
            return None;
        }
        Some(self.ust.pending.remove(0).text)
    }

    /// Queued lines, oldest first.
    pub fn pending(&self) -> VecDeque<&str> {
        self.ust.pending.iter().map(|p| p.text.as_str()).collect()
    }

    /// The room the character stands in.
    pub fn room_key(&self) -> Option<RoomKey> {
        self.character.as_ref().map(|c| RoomKey {
            map: c.map,
            room: c.room,
        })
    }
}

/// The player record set to all zeroes. Strings are empty, numbers zero, every
/// flag off, enums at their zero value. Character creation (T-022) starts from this.
pub fn clear_player() -> Character {
    Character {
        userid: String::new(),
        firstname: String::new(),
        lastname: String::new(),
        exp: 0,
        spells: [SpellNo(0); NSPELLCAST],
        spellvals: [0; NSPELLCAST],
        spellrnds: [0; NSPELLCAST],
        title: String::new(),
        race: RaceNo(0),
        class: ClassNo(0),
        level: 0,
        basint: 0,
        baswil: 0,
        basstr: 0,
        bashea: 0,
        basagi: 0,
        bascha: 0,
        intel: 0,
        wil: 0,
        str: 0,
        hea: 0,
        agi: 0,
        cha: 0,
        maxhp: 0,
        hp: 0,
        maxwgt: 0,
        wgt: 0,
        maxenergy: 0,
        energy: 0,
        poison: 0,
        maxmr: 0,
        mr: 0,
        map: MapId(0),
        room: RoomId(0),
        light: 0,
        hunger: 0,
        thirst: 0,
        ninvslots: 0,
        items: vec![ItemNo(0); NINVITEMS],
        itemuses: vec![0; NINVITEMS],
        nkeyslots: 0,
        keys: vec![ItemNo(0); NKEYS],
        keyuses: vec![0; NKEYS],
        qflags2: 0,
        hpbonus: 0,
        manabonus: 0,
        expbil: 0,
        exprem: 0,
        nspellslots: 0,
        spellbook: vec![SpellNo(0); NSPELLBOOK],
        evil: 0,
        lawful: 0,
        trailmaps: [MapId(0); NTRAIL],
        trailrooms: [RoomId(0); NTRAIL],
        verbose: 0,
        bchannel: 0,
        fasttalk: 0,
        statline: StatlineMode::Off,
        hidden: 0,
        perception: 0,
        stealth: 0,
        martial: 0,
        thievery: 0,
        maxmana: 0,
        mana: 0,
        spellcast: 0,
        traps: 0,
        disarm: 0,
        picklocks: 0,
        tracking: 0,
        coins: Coins::default(),
        weapon: ItemNo(0),
        worn: [ItemNo(0); NWORN],
        wornuses: [0; NWORN],
        autoengage: 0,
        lives: 0,
        aided: 0,
        rank: 0,
        lighttime: 0,
        readied: 0,
        delay: 0,
        roomlight: 0,
        gang: String::new(),
        hairlen: 0,
        haircol: 0,
        eyecol: 0,
        paging: 0,
        cp: 0,
        suicpw: String::new(),
        attackers: 0,
        status: PlayerStatus::empty(),
        movedir: 0,
        dragging: 0,
        totalcp: 0,
        immortal: 0,
        pagecnt: 0,
        pagemax: 0,
        settings: PlayerSettings::empty(),
        gossmax: 0,
        auctmax: 0,
        gosscnt: 0,
        auctcnt: 0,
        encumpct: 0,
        avbonus: 0,
        acbonus: 0,
        dmgbonus: 0,
        mabase: 0,
        arenahp: 0,
        qflags: 0,
        version: 0,
        msglevel: MessageLevel::High,
        pvpcnt: 0,
        randomhp: 0,
        abils: [(silvermere_types::enums::Ability::None, 0); NABILITY],
        resttick: 0,
        critbonus: 0,
        drbonus: 0,
        drpct: 0,
        energyuse: 0,
        shield: 0,
        defmod: 0,
        sesstime: 0,
        lifemins: 0,
        moves: 0,
        lastdate: 0,
        status2: PlayerStatus2::empty(),
        palette: 0,
        deathdate: 0,
        flags: PlayerFlags::empty(),
        sex: 0,
        healrate: 0,
        manarate: 0,
        racechg: 0,
        lastmon: 0,
        lastmoncnt: 0,
        pvptimer: 0,
        medtick: 0,
    }
}
