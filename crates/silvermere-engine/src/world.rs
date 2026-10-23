//! `World`: every piece of live game state, loaded once and owned by the engine thread.
//!
//! The engine holds the whole world in typed maps. Content (rooms, items, known monsters,
//! spells, classes, races, shops, messages, text blocks, actions) comes in as a
//! [`WorldSnapshot`] that `bin` fills from the `silvermere-db` repositories, so this crate
//! never sees a database. [`World::reload_content`] replaces the content maps for the admin
//! API (T-018/T-060) without touching live state.
//!
//! The engine-private tables (the auto-combat ring, the
//! occupied-room table, the monster quick reference, the exit revert queue, the restock
//! list, evil timers, gang invites) are fields here with the original's sizes.

use std::collections::{HashMap, VecDeque};

use silvermere_types::config::GameConfig;
use silvermere_types::flags::RoomFlags;
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ClassNo, ItemNo, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey,
    SessionId, ShopNo, SpellNo, TextBlockNo,
};
use silvermere_types::records::{
    Action, ActiveMonster, BankBook, Character, Class, Coins, EvilRecord, Gang, Item,
    ItemOwnership, KnownMonster, Message, PermanentRecord, Race, Room, RoomItem, Shop, Spell,
    TextBlock, TopEntry,
};
use silvermere_types::sizes::{NDIRS, NHIDITEMS, NROOMITEMS, NROOMMONS};

use crate::ids::SessionIds;
use crate::movement::exits::ExitOverride;
use crate::player_session::PlayerSession;
use crate::progression::TopTen;
use crate::scheduler::{Periods, Scheduler, SeededRng};

/// `NOCCROOMS`: entries of the occupied-room table.
pub const NOCCROOMS: usize = silvermere_rules::spawn::NOCCROOMS;

/// Why [`World::load`] refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorldError {
    /// A special room of `[game]` is not in the content (the rooms are checked on map 1).
    #[error("game.{name} = {room}: room does not exist on map 1")]
    MissingSpecialRoom {
        /// The option (`start_room`, `reinc_room`, `evil_reinc_room`, `jail_room`).
        name: &'static str,
        /// The room number it names.
        room: u32,
    },
    /// The content has no rooms at all.
    #[error("no rooms in the world data")]
    NoRooms,
}

/// The live part of a room (what `RoomState` in `silvermere-db` persists): floor
/// items, hidden items, monsters here, coins, owner, the alive flag and the counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomState {
    /// The room.
    pub key: RoomKey,
    /// Coins lying here.
    pub coins: Coins,
    /// Hidden coins.
    pub hidcoins: Coins,
    /// Owner's user-id (`RF_OWNED`).
    pub owner: String,
    /// `RF_SPECALIVE`: the room's specific monster is alive.
    pub specalive: bool,
    /// Hidden slots found.
    pub foundmask: u32,
    /// Minute of last kill.
    pub lastkill: u16,
    /// Area monsters now.
    pub areacount: i16,
    /// Players who found hidden items.
    pub searchers: u8,
    /// Monsters generated here now.
    pub nummonsters: u8,
    /// Visible items.
    pub items: [RoomItem; NROOMITEMS],
    /// Hidden items.
    pub hiditems: [RoomItem; NHIDITEMS],
    /// Active monsters here.
    pub monsters: [ActiveMonsterId; NROOMMONS],
    /// The live exit parameters (T-026): a door's lock state, a found hidden exit, a
    /// timed exit's phase, a disarmed trap. The content `exitpar*` are never changed;
    /// `movement::exits::live_exit` lays these over them. In memory only, by decision
    /// (#42): the original relocked every door and gate at start-up, so a restart
    /// reverting them to the content is the original's behaviour and the persisted room state
    /// does not carry them.
    pub exit_overrides: [ExitOverride; NDIRS],
}

impl RoomState {
    /// The live part of a loaded `room` record.
    pub fn of(room: &Room) -> Self {
        Self {
            key: room.key,
            coins: room.coins,
            hidcoins: room.hidcoins,
            owner: room.owner.clone(),
            specalive: room.flags.contains(RoomFlags::SPECALIVE),
            foundmask: room.foundmask,
            lastkill: room.lastkill,
            areacount: room.areacount,
            searchers: room.searchers,
            nummonsters: room.nummonsters,
            items: room.items,
            hiditems: room.hiditems,
            monsters: room.monsters,
            exit_overrides: [ExitOverride::default(); NDIRS],
        }
    }
}

/// One entry of the occupied-room table. A room
/// number of 0 marks a free entry; clearing the table zeroes only the room number, so a
/// reused entry keeps the counts of its last use (sic).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OccRoom {
    /// The room with players in it.
    pub room: RoomKey,
    /// Players counted there.
    pub nplayers: u8,
    /// Monsters already there.
    pub nmons: u8,
}

/// A pending gang invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GangInvite {
    /// The invited session.
    pub session: SessionId,
    /// The gang.
    pub gang: String,
}

/// Everything `World::load` takes: the content tables plus the live records that survive
/// a restart. `bin` fills it from the repositories; tests build it by hand.
#[derive(Debug, Clone, Default)]
pub struct WorldSnapshot {
    pub rooms: Vec<Room>,
    pub items: Vec<Item>,
    pub monsters: Vec<KnownMonster>,
    pub active_monsters: Vec<ActiveMonster>,
    pub spells: Vec<Spell>,
    pub classes: Vec<Class>,
    pub races: Vec<Race>,
    pub shops: Vec<Shop>,
    pub messages: Vec<Message>,
    pub text_blocks: Vec<TextBlock>,
    pub actions: Vec<Action>,
    pub gangs: Vec<Gang>,
    pub item_ownership: Vec<ItemOwnership>,
    /// Bank books, with the owning account.
    pub bank_books: Vec<(AccountId, BankBook)>,
    pub profiles: Vec<(AccountId, PermanentRecord)>,
    pub evil_points: Vec<(AccountId, EvilRecord)>,
    /// The characters with the most experience, best first, for the top list
    /// (`top_by_exp(NTOPLIST)` in `silvermere-db`).
    pub top_characters: Vec<Character>,
}

slotmap::new_key_type! {
    /// A slot in [`World::active_monsters`].
    pub struct ActiveSlot;
}

/// The `[compat]` settings the engine reads (set by `bin` after [`World::load`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compat {
    /// `compat.bbs_menu`: after login, the BBS main menu shim (`Make your selection (X to
    /// exit):`) before the `[MAJORMUD]:` menu.
    pub bbs_menu: bool,
    /// `compat.version_string`: shown in the menu title.
    pub mud_version: String,
    /// The logon greeting's version.
    pub short_version: String,
    /// `compat.charset = "utf8"`: new sessions start with UTF-8 output (T-028; `SET UTF8`
    /// switches one session).
    pub utf8: bool,
}

impl Default for Compat {
    fn default() -> Self {
        Self {
            bbs_menu: true,
            mud_version: silvermere_text::menu::MUDVER.into(),
            short_version: silvermere_text::menu::SHORTVER.into(),
            utf8: false,
        }
    }
}

/// The world.
#[derive(Debug)]
pub struct World {
    /// The game options.
    pub config: GameConfig,
    /// The `[compat]` settings (T-021).
    pub compat: Compat,
    /// Rooms: the static content record.
    pub rooms: HashMap<RoomKey, Room>,
    /// Rooms: the live part.
    pub room_state: HashMap<RoomKey, RoomState>,
    pub items: HashMap<ItemNo, Item>,
    pub monsters: HashMap<MonsterNo, KnownMonster>,
    pub spells: HashMap<SpellNo, Spell>,
    pub classes: HashMap<ClassNo, Class>,
    pub races: HashMap<RaceNo, Race>,
    pub shops: HashMap<ShopNo, Shop>,
    pub messages: HashMap<MessageNo, Message>,
    /// Text blocks by `(part, number)`.
    pub text_blocks: HashMap<(i16, TextBlockNo), TextBlock>,
    /// Social actions by name (ASCII upper case).
    pub actions: HashMap<String, Action>,
    /// The verbs in WCCACTS record order, white space removed, which the ACTION list
    /// prints.
    pub action_order: Vec<String>,
    /// Active monsters with their numbers' free list (T-035).
    pub active_monsters: crate::monsters::ActiveMonsters,
    /// Sessions online, by id.
    pub sessions: HashMap<SessionId, PlayerSession>,
    /// The id allocator.
    pub ids: SessionIds,
    /// Gangs by key (`gang.key`, upper case name).
    pub gangs: HashMap<String, Gang>,
    /// Bank books by account and bank.
    pub bank_books: HashMap<(AccountId, ShopNo), BankBook>,
    /// Limited-item ownership by row id.
    pub item_ownership: HashMap<i32, ItemOwnership>,
    pub profiles: HashMap<AccountId, PermanentRecord>,
    pub evil_points: HashMap<AccountId, EvilRecord>,
    /// The top list (`NTOPLIST` entries), kept by `progression::topten` (T-039).
    pub topten: TopTen,
    /// The timer queue.
    pub scheduler: Scheduler,
    /// The random source every handler and cycle draws from: the engine
    /// thread's one `Rng`. `World::load` seeds it from the OS; `Engine::new` reseeds it
    /// from the engine's `Rng`; tests replace it with a fixed seed.
    pub rng: SeededRng,
    /// The session while `handle_commands` runs a command: the player whose text sits in
    /// the one output buffer (the dispatcher's `ANSI_NORMAL`). A `tell_*` to anyone else
    /// sends that buffer, so the reset goes with it ([`crate::tell`]).
    pub prf_owner: Option<SessionId>,
    /// The occupied-room table, always [`NOCCROOMS`] entries.
    pub occupied: Vec<OccRoom>,
    /// `monqref`: the known-monster quick reference (T-035).
    pub monqref: Vec<silvermere_rules::spawn::QuickRef>,
    /// The spawn pass's and the monster passes' statics (T-035).
    pub spawn: crate::monsters::SpawnState,
    /// The wall clock of the input being handled, Unix seconds (UTC), set by the event
    /// loop with [`World::today`] and [`World::minute`]: what `now()`/`today()` gave
    /// the monster code (a unique monster's wait, a record's `created`).
    pub wall: i64,
    /// Timed exits waiting to revert.
    pub exit_reverts: VecDeque<(RoomKey, u8)>,
    /// Pending shop restocks (the head is the last entry).
    pub restock: Vec<crate::shops::RestockNode>,
    /// The evil timers, oldest first (T-038).
    pub evil_timers: Vec<crate::evil::EvilTimer>,
    /// Pending gang invitations.
    pub gang_invites: Vec<GangInvite>,
    /// The background cycles' own state and hooks (T-037).
    pub cycles: crate::cycles::CycleState,
    /// The auto-combat entries and ring (T-034).
    pub autocombat: crate::combat::autocombat::AutoCombat,
    /// The session slot the next save sweep starts at.
    pub save_next: u16,
    /// Players allowed in the realm at once, separate from the session slots; starts at
    /// `server.max_sessions`, the SYSOP command changes it (T-055).
    pub max_in_game: u16,
    /// The day of the input being handled (UTC midnight in Unix seconds,
    /// [`crate::event_loop::day_start`]), set by the event loop before each input: what
    /// `today()` gave handlers that have no `today` argument (`commit_suicide`, T-068).
    pub today: i64,
    /// The minute of the day of the input being handled (UTC), set with
    /// [`World::today`]: a room's `lastkill` stamp (T-033).
    pub minute: u16,
    /// `bootmin`: the minute of the day the engine started; a kill stamped earlier in the
    /// day counts as the next day.
    pub boot_minute: u16,
    /// The room display's globals: `arenasafe`, the live `dirnames[]`, the
    /// `FILE DESCRIPTION` files (T-025).
    pub display: crate::display::DisplayGlobals,
    /// The daily cleanup's state between its start and the store's answer (T-056).
    pub cleanup: crate::cleanup::CleanupState,
    /// Account deletions waiting for the store's read of the account (T-069), with the
    /// admin requests to answer.
    pub account_deletes: std::collections::HashMap<
        silvermere_types::ids::AccountId,
        Vec<std::sync::mpsc::Sender<crate::session::AccountDeletion>>,
    >,
    /// Accounts deleted in this run (T-072). The gate reads a login's character from the
    /// store before the engine sees it, so a login racing a deletion can bring a stale
    /// character; [`crate::state::login::logged_in`] closes any login for these.
    pub deleted_accounts: std::collections::HashSet<silvermere_types::ids::AccountId>,
    /// The character record the engine last queued for each account, `None` for a
    /// deletion (T-075; [`crate::player::queue_character`]). The store writes behind, and
    /// the gate reads a login's character before the engine sees the login, so a login
    /// that closely follows a save (a hang-up, then straight back in) can read the row
    /// as it was before; [`crate::state::login::logged_in`] takes this record instead and
    /// drops it. The offline readers (the cleanup scan, gang offline edits, the sysop's
    /// user checks) read it first as well ([`crate::player::queued_or_stored`], T-088).
    /// Every write of a character row comes from the engine, in order, so this record is
    /// never older than the stored row. An entry also goes when the store acknowledges
    /// its write ([`crate::player::on_characters_written`]); the store answers a read
    /// before it acknowledges any write queued after the read, so an answer always finds
    /// the entries of the writes it missed.
    pub queued_characters:
        std::collections::HashMap<silvermere_types::ids::AccountId, crate::player::QueuedCharacter>,
    /// How many character writes and deletions the engine has queued
    /// ([`crate::player::QueuedCharacter::seq`]); the store counts the same requests in the
    /// same order to acknowledge them (T-088).
    pub character_writes: u64,
    /// The store's acknowledgements not applied yet, each with the sessions that were in
    /// the login dialogue when it came ([`crate::player::on_characters_written`]).
    pub characters_written: Vec<(u64, Vec<silvermere_types::ids::SessionId>)>,
    /// The sysop command's and menu's globals and jobs (T-055).
    pub sysop: crate::commands::sysop::SysopState,
    /// Sessions whose output batches were touched during the current event, in order.
    pub(crate) touched: Vec<SessionId>,
    /// Non-write outputs (echo, char mode, close, events) queued during the event; sent
    /// after the writes.
    pub(crate) outbox: Vec<crate::event_loop::EngineOutput>,
    /// `Opened`/`Refused`, sent before the writes of the event so the transport side has
    /// the session's mapping when its first write arrives.
    pub(crate) outbox_first: Vec<crate::event_loop::EngineOutput>,
    /// Sessions that leave at the end of the event, after their last write went out.
    pub(crate) logoffs: Vec<SessionId>,
}

impl World {
    /// Builds the world from a snapshot, checks the special rooms exist on map 1, arms the
    /// scheduler and computes the top list.
    pub fn load(
        config: GameConfig,
        max_sessions: u16,
        snapshot: WorldSnapshot,
    ) -> Result<Self, WorldError> {
        Self::load_seeded(config, max_sessions, snapshot, entropy_seed(), true)
    }

    /// [`Self::load`] with the start-up pass's rng seeded by `seed`, its saves coalesced
    /// (`coalesce`, as `load` does) or not: for the test that the coalescing changes
    /// nothing stored (T-113, `bin/silvermere/tests/startup_coalescing.rs`).
    #[doc(hidden)]
    pub fn load_seeded(
        config: GameConfig,
        max_sessions: u16,
        snapshot: WorldSnapshot,
        seed: u64,
        coalesce: bool,
    ) -> Result<Self, WorldError> {
        if snapshot.rooms.is_empty() {
            return Err(WorldError::NoRooms);
        }
        let mut world = Self {
            scheduler: Scheduler::new(Periods::from_config(&config)),
            rng: SeededRng::new(seed),
            prf_owner: None,
            config,
            compat: Compat::default(),
            rooms: HashMap::new(),
            room_state: HashMap::new(),
            items: HashMap::new(),
            monsters: HashMap::new(),
            spells: HashMap::new(),
            classes: HashMap::new(),
            races: HashMap::new(),
            shops: HashMap::new(),
            messages: HashMap::new(),
            text_blocks: HashMap::new(),
            actions: HashMap::new(),
            action_order: Vec::new(),
            active_monsters: crate::monsters::ActiveMonsters::default(),
            sessions: HashMap::new(),
            ids: SessionIds::new(max_sessions),
            gangs: HashMap::new(),
            bank_books: HashMap::new(),
            item_ownership: HashMap::new(),
            profiles: HashMap::new(),
            evil_points: HashMap::new(),
            topten: TopTen::new(),
            occupied: vec![OccRoom::default(); NOCCROOMS],
            monqref: Vec::new(),
            spawn: crate::monsters::SpawnState::default(),
            wall: 0,
            exit_reverts: VecDeque::new(),
            restock: Vec::new(),
            evil_timers: Vec::new(),
            gang_invites: Vec::new(),
            cycles: crate::cycles::CycleState::default(),
            autocombat: Default::default(),
            save_next: 0,
            max_in_game: max_sessions,
            today: 0,
            minute: 0,
            boot_minute: 0,
            display: crate::display::DisplayGlobals::default(),
            cleanup: crate::cleanup::CleanupState::default(),
            account_deletes: std::collections::HashMap::new(),
            deleted_accounts: std::collections::HashSet::new(),
            queued_characters: std::collections::HashMap::new(),
            character_writes: 0,
            characters_written: Vec::new(),
            sysop: crate::commands::sysop::SysopState::default(),
            touched: Vec::new(),
            outbox: Vec::new(),
            outbox_first: Vec::new(),
            logoffs: Vec::new(),
        };
        world.room_state = snapshot
            .rooms
            .iter()
            .map(|r| (r.key, RoomState::of(r)))
            .collect();
        world.active_monsters = crate::monsters::ActiveMonsters::load(&snapshot.active_monsters);
        // the cache's `monsbuf + 1` slots (MONSBUF, `game.buffers.monsbuf`)
        *world.active_monsters.cache_mut() =
            crate::monsters::cache::MonsterCache::new(world.config.buffers.monsbuf as usize);
        world.gangs = snapshot
            .gangs
            .iter()
            .map(|g| (g.key.clone(), g.clone()))
            .collect();
        world.item_ownership = snapshot
            .item_ownership
            .iter()
            .map(|o| (o.id, o.clone()))
            .collect();
        world.bank_books = snapshot
            .bank_books
            .iter()
            .map(|(a, b)| ((*a, b.bank), b.clone()))
            .collect();
        world.profiles = snapshot.profiles.iter().cloned().collect();
        world.evil_points = snapshot.evil_points.iter().cloned().collect();
        world.topten = TopTen::rebuild(&snapshot.top_characters);
        world.replace_content(&snapshot);
        world.check_special_rooms()?;
        // The first NRESTOCK shops queue their restocks once
        let mut rng = world.rng.clone();
        crate::shops::initiate_all(&mut world, &mut rng);
        world.rng = rng;
        world.scheduler.start();
        // At start-up: no recent kills, every exit back to
        // its start state, the timed exits armed (T-035)
        if coalesce {
            crate::monsters::preload(&mut world);
        } else {
            crate::monsters::preload_cycle(&mut world, crate::monsters::StartUp::Cold);
        }
        Ok(world)
    }

    /// Replaces the content tables from a fresh snapshot and leaves every live table
    /// (`room_state`, active monsters, sessions, gangs, books, ownership) as it is. The
    /// special rooms are checked again; on failure nothing is replaced.
    pub fn reload_content(&mut self, snapshot: &WorldSnapshot) -> Result<(), WorldError> {
        if snapshot.rooms.is_empty() {
            return Err(WorldError::NoRooms);
        }
        let keep = std::mem::take(&mut self.rooms);
        self.rooms = snapshot.rooms.iter().map(|r| (r.key, r.clone())).collect();
        if let Err(e) = self.check_special_rooms() {
            self.rooms = keep;
            return Err(e);
        }
        // the live stock (`shop_stock`) outlives a content reload
        let stock: HashMap<ShopNo, [u16; silvermere_types::sizes::NSHOPITEMS]> = self
            .shops
            .iter()
            .map(|(&n, s)| (n, std::array::from_fn(|i| s.stock[i].qty)))
            .collect();
        self.replace_content(snapshot);
        for (n, qty) in stock {
            if let Some(s) = self.shops.get_mut(&n) {
                for (slot, q) in s.stock.iter_mut().zip(qty) {
                    slot.qty = q;
                }
            }
        }
        // A room new to the content gets a fresh live record; stale ones are kept so a
        // room taken out of the content and put back keeps its floor.
        for r in &snapshot.rooms {
            self.room_state
                .entry(r.key)
                .or_insert_with(|| RoomState::of(r));
        }
        Ok(())
    }

    fn replace_content(&mut self, s: &WorldSnapshot) {
        self.rooms = s.rooms.iter().map(|r| (r.key, r.clone())).collect();
        self.items = s.items.iter().map(|i| (i.number, i.clone())).collect();
        self.monsters = s.monsters.iter().map(|m| (m.number, m.clone())).collect();
        self.spells = s.spells.iter().map(|x| (x.number, x.clone())).collect();
        self.classes = s.classes.iter().map(|c| (c.number, c.clone())).collect();
        self.races = s.races.iter().map(|r| (r.number, r.clone())).collect();
        self.shops = s.shops.iter().map(|x| (x.number, x.clone())).collect();
        self.messages = s.messages.iter().map(|m| (m.number, m.clone())).collect();
        self.text_blocks = s
            .text_blocks
            .iter()
            .map(|t| ((t.part, t.number), t.clone()))
            .collect();
        crate::stock_faults::apply(&mut self.text_blocks);
        self.action_order = s
            .actions
            .iter()
            .map(|a| a.name.split_whitespace().collect())
            .collect();
        self.actions = s
            .actions
            .iter()
            .map(|a| (a.name.to_ascii_uppercase(), a.clone()))
            .collect();
        crate::monsters::quickref::rebuild(self);
    }

    /// The four special rooms of `[game]`, all on map 1 (checked at start-up; the `serve`
    /// command makes the same check against the database before loading).
    pub fn special_rooms(config: &GameConfig) -> [(&'static str, u32); 4] {
        [
            ("start_room", config.start_room),
            ("reinc_room", config.reinc_room),
            ("evil_reinc_room", config.evil_reinc_room),
            ("jail_room", config.jail_room),
        ]
    }

    fn check_special_rooms(&self) -> Result<(), WorldError> {
        for (name, room) in Self::special_rooms(&self.config) {
            if !self.rooms.contains_key(&RoomKey::new(1, room)) {
                return Err(WorldError::MissingSpecialRoom { name, room });
            }
        }
        Ok(())
    }

    /// The session, if connected.
    pub fn session(&self, id: SessionId) -> Option<&PlayerSession> {
        self.sessions.get(&id)
    }

    /// The session, mutable, if connected. The batch it touches is flushed at the end of
    /// the event.
    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut PlayerSession> {
        if self.sessions.contains_key(&id) {
            self.touch(id);
        }
        self.sessions.get_mut(&id)
    }

    /// Session ids in channel order (from channel 0 up through every terminal), the order every
    /// `tell_*` loop and background pass walks.
    pub fn session_ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<SessionId> = self.sessions.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Marks `id` as touched this event so its batch is flushed, keeping first-touch order.
    pub(crate) fn touch(&mut self, id: SessionId) {
        if !self.touched.contains(&id) {
            self.touched.push(id);
        }
    }

    /// Announces the session the engine opened for `conn`; goes out before any write of
    /// the same event.
    pub fn opened(&mut self, conn: crate::event_loop::ConnId, id: SessionId) {
        self.outbox_first
            .push(crate::event_loop::EngineOutput::Opened { conn, id });
        self.outbox.push(crate::event_loop::EngineOutput::Event(
            crate::event_loop::GameEvent::Connected { id },
        ));
    }

    /// Announces that `conn` got no session; goes out before any write of the same event.
    pub fn refused(&mut self, conn: crate::event_loop::ConnId) {
        self.outbox_first
            .push(crate::event_loop::EngineOutput::Refused { conn });
    }

    /// Queues the session's departure (`sttrou` returning 0): what it has queued still
    /// goes out as its last write, then `Close`; the session is removed and its id freed
    /// at the end of the event's flush.
    pub fn logoff(&mut self, id: SessionId) {
        if !self.sessions.contains_key(&id) || self.logoffs.contains(&id) {
            return;
        }
        self.touch(id);
        self.outbox
            .push(crate::event_loop::EngineOutput::Close { id });
        self.outbox.push(crate::event_loop::EngineOutput::Event(
            crate::event_loop::GameEvent::Closed { id },
        ));
        self.logoffs.push(id);
    }

    /// Whether one session is in the game: in the realm, past
    /// the menus, with a character loaded.
    pub fn is_in_game(&self, id: SessionId) -> bool {
        self.sessions
            .get(&id)
            .is_some_and(PlayerSession::is_in_game)
    }

    /// The room content record.
    pub fn room(&self, key: RoomKey) -> Option<&Room> {
        self.rooms.get(&key)
    }

    /// The key of a room on map 1 (the special rooms).
    pub fn map1(room: u32) -> RoomKey {
        RoomKey {
            map: silvermere_types::ids::MapId(1),
            room: RoomId(room),
        }
    }

    // --- cycle bodies: stubs the owning cards fill ---------------------------------

    /// The energy cycle: new energy for everyone, then the combat
    /// round (`combat::autocombat::background_energy`; the monsters' part is T-036's).
    pub fn energy_cycle(&mut self) {
        crate::combat::autocombat::background_energy(self);
    }

    /// `medium_update_characters`, `medium_update_monsters`:
    /// `cycles::medium`.
    pub fn medium_cycle(&mut self) {
        crate::cycles::medium::medium_cycle(self);
    }

    /// `shuffle_mappings`, `slow_update_characters`, `slow_update_monsters`
    /// (`cycles::slow`). The scheduler's restock count follows, then
    /// [`World::slow_cycle_end`].
    pub fn slow_cycle(&mut self) {
        crate::cycles::slow::slow_cycle(self);
    }

    /// The direction name letters and the random event, after the
    /// restock count: `cycles::slow`.
    pub fn slow_cycle_end(&mut self) {
        crate::cycles::slow::slow_cycle_end(self);
    }

    /// Queues the persistence barrier (`PersistRequest::Flush`) after every write queued so
    /// far; the store answers it once they are committed.
    pub fn queue_flush(&mut self) {
        self.outbox.push(crate::event_loop::EngineOutput::Persist(
            crate::event_loop::PersistRequest::Flush,
        ));
    }

    /// `shops::restock_items` with the world's rng.
    pub fn restock_items(&mut self) {
        let mut rng = self.rng.clone();
        crate::shops::restock_items(self, &mut rng);
        self.rng = rng;
    }

    /// `generate_monsters_near_users` (`monsters::spawn_pass`).
    pub fn spawn_cycle(&mut self) {
        crate::monsters::spawn_pass::generate_monsters_near_users(self);
    }

    /// `save_characters`: the round-robin save (T-023), then the
    /// write-back of the active monsters changed since their last write (T-035).
    pub fn save_cycle(&mut self) {
        crate::player::save_characters(self);
        crate::monsters::active::save_active_monsters(self);
    }

    /// The two-strike idle hang-up, written by T-021 in
    /// `state::login::leave::inactivity_check`.
    pub fn inactivity_check(&mut self) {
        crate::state::login::leave::inactivity_check(self);
    }

    /// One queued exit change is undone
    /// (`movement::timed_exits::revert_exit`, T-026).
    pub fn revert_exit(&mut self, room: RoomKey, dir: u8) {
        crate::movement::timed_exits::revert_exit(self, room, dir);
    }

    /// Follower moves, rest heal every 21st
    /// tick, meditate every 15th, the stun, then the command delay and queue
    /// (`cycles::fast`; the delay is T-024's and T-021's, through
    /// `CycleHooks::delay_step`).
    pub fn fast_update_characters(&mut self) {
        crate::cycles::fast::fast_update_characters(self);
    }

    /// The fast monster pass starts
    /// (`cycles::monsters`).
    pub fn fast_update_monsters(&mut self) {
        crate::cycles::monsters::fast_update_monsters(self);
    }

    /// The polls between two ticks: the pending monster passes
    /// (`cycles::monsters::poll_monster_passes`), then the monsters the cache pushed out
    /// written back (`monsters::active::write_back_evicted`).
    pub fn poll_monster_passes(&mut self) {
        crate::cycles::monsters::poll_monster_passes(self);
        crate::monsters::active::write_back_evicted(self);
    }

    /// The minute stamp of now (the minute of the day, plus a day when it lies
    /// before `bootmin`): what `room.lastkill` holds,
    /// the same clock T-033's `check_kill_monster` stamps a kill with.
    pub fn minute_stamp(&self) -> u16 {
        if self.minute < self.boot_minute {
            self.minute + silvermere_rules::spawn::MINSPERDAY as u16
        } else {
            self.minute
        }
    }

    /// Sets the wall clock and the minute of the day (the event loop, before each
    /// input).
    pub fn set_wall(&mut self, secs: i64) {
        self.wall = secs;
        self.minute = crate::event_loop::minute_of_day(secs);
    }

    /// `shuffle_mappings`: a new random session order, read back with
    /// [`World::shuffled_sessions`]. The slow cycle calls it; no start-up call
    /// is needed (the order falls back to id order).
    pub fn shuffle_order(&mut self) {
        crate::cycles::slow::shuffle_mappings(self);
    }
}

/// A seed nobody can predict, from the process's hash randomness: the world's `Rng` is
/// seeded once here, so production draws differ run to run while tests set their own.
fn entropy_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

/// A top-list entry from a character record.
pub fn top_entry(c: &Character) -> TopEntry {
    TopEntry {
        userid: c.userid.clone(),
        name: c.firstname.clone(),
        lastname: c.lastname.clone(),
        classnum: c.class,
        gang: c.gang.clone(),
        exphi: c.expbil,
        explo: c.exprem,
        edited: String::new(),
        nrecs: 0,
    }
}

/// Zeroed records, for where a record starts empty. Every number
/// is zero, every string empty, every enum at its zero code, every flag off.
pub mod blank {
    use silvermere_types::enums::{
        Alignment, ArmourType, Currency, ExitType, GangHouse, Gender, GroupRole, ItemType, Magery,
        MonsterAttackType, MonsterType, RoomType, WeaponClass, WeaponType, WornLocation,
    };
    use silvermere_types::flags::RoomFlags;
    use silvermere_types::ids::{
        ActiveMonsterId, ClassNo, ItemNo, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, ShopNo,
        SpellNo, TextBlockNo,
    };
    use silvermere_types::records::{
        Class, Coins, Exit, Item, KnownMonster, MonsterAttack, MonsterItem, MonsterSpell, Race,
        Room, RoomItem,
    };

    const NO_ITEM: RoomItem = RoomItem {
        item: ItemNo(0),
        uses: 0,
        count: 0,
    };

    /// An all-zero room with `key`.
    pub fn clear_room(key: RoomKey) -> Room {
        Room {
            key,
            name: String::new(),
            desc: std::array::from_fn(|_| String::new()),
            mapfile: String::new(),
            exits: std::array::from_fn(|_| Exit {
                target: RoomId(0),
                kind: ExitType::from_code(0),
                par1: 0,
                par2: 0,
                par3: 0,
                par4: 0,
            }),
            monsters: [ActiveMonsterId(0); silvermere_types::sizes::NROOMMONS],
            room_type: RoomType::from_code(0),
            shopnum: ShopNo(0),
            owner: String::new(),
            minlevel: 0,
            maxlevel: 0,
            genmonster: MonsterNo(0),
            light: 0,
            ganghouse: GangHouse::from_code(0),
            items: [NO_ITEM; silvermere_types::sizes::NROOMITEMS],
            hiditems: [NO_ITEM; silvermere_types::sizes::NHIDITEMS],
            coins: Coins::default(),
            hidcoins: Coins::default(),
            maxmonsters: 0,
            mongroup: 0,
            lastkill: 0,
            flags: RoomFlags::empty(),
            foundmask: 0,
            deathroom: RoomId(0),
            huproom: RoomId(0),
            cmdtext: TextBlockNo(0),
            regendelay: 0,
            areamax: 0,
            areacount: 0,
            ctlroom: RoomId(0),
            specmon: MonsterNo(0),
            placed: [ItemNo(0); silvermere_types::sizes::NPLACED],
            protmsg: TextBlockNo(0),
            picktrap: 0,
            roomspell: 0,
            searchers: 0,
            nummonsters: 0,
        }
    }

    /// An all-zero item with `number`.
    pub fn clear_item(number: ItemNo) -> Item {
        Item {
            number,
            level: 0,
            limit: 0,
            name: String::new(),
            desc: std::array::from_fn(|_| String::new()),
            hiddesc: std::array::from_fn(|_| String::new()),
            active: 0,
            weight: 0,
            item_type: ItemType::from_code(0),
            abilities: [(silvermere_types::enums::Ability::None, 0);
                silvermere_types::sizes::NITEMABILS],
            charges: 0,
            energy: 0,
            cost: 0,
            classes: std::array::from_fn(|_| ClassNo(0)),
            mindmg: 0,
            maxdmg: 0,
            ac: 0,
            races: std::array::from_fn(|_| RaceNo(0)),
            negates: std::array::from_fn(|_| SpellNo(0)),
            wpntype: WeaponType::from_code(0),
            armtype: ArmourType::from_code(0),
            worn: WornLocation::from_code(0),
            accuracy: 0,
            dr: 0,
            gettable: 0,
            strreq: 0,
            cash: Coins::default(),
            speed: 0,
            hitmsg: MessageNo(0),
            missmsg: MessageNo(0),
            textnum: TextBlockNo(0),
            destroymsg: MessageNo(0),
            nodrop: 0,
            currency: Currency::from_code(0),
            retain: 0,
            robable: 0,
            destroy: 0,
        }
    }

    /// An all-zero known monster.
    pub fn clear_known_monster(number: MonsterNo) -> KnownMonster {
        KnownMonster {
            number,
            name: String::new(),
            montype: MonsterType::from_code(0),
            level: 0,
            genindex: 0,
            wornitem: ItemNo(0),
            weapon: ItemNo(0),
            dr: 0,
            ac: 0,
            group: 0,
            followpct: 0,
            magicres: 0,
            bsdefense: 0,
            exp: 0,
            maxhp: 0,
            energy: 0,
            hpregen: 0,
            abilities: [(silvermere_types::enums::Ability::None, 0);
                silvermere_types::sizes::NRECABILS],
            maxalive: 0,
            curalive: 0,
            grouprole: GroupRole::from_code(0),
            maxfollow: 0,
            undead: 0,
            alignment: Alignment::from_code(0),
            regenhrs: 0,
            killed_at: 0,
            arrivemsg: MessageNo(0),
            deathmsg: MessageNo(0),
            items: std::array::from_fn(|_| MonsterItem {
                item: ItemNo(0),
                uses: 0,
                pct: 0,
            }),
            cash: Coins::default(),
            talktext: TextBlockNo(0),
            charmlvl: 0,
            namelist: 0,
            attacks: std::array::from_fn(|_| MonsterAttack {
                kind: MonsterAttackType::from_code(0),
                accuracy: 0,
                pct: 0,
                min: 0,
                max: 0,
                hitmsg: MessageNo(0),
                defmsg: MessageNo(0),
                missmsg: MessageNo(0),
                energy: 0,
                hitspell: SpellNo(0),
            }),
            randtext: TextBlockNo(0),
            charmres: 0,
            deathspell: SpellNo(0),
            createspell: SpellNo(0),
            spells: std::array::from_fn(|_| MonsterSpell {
                spell: SpellNo(0),
                pct: 0,
                level: 0,
            }),
            desc: std::array::from_fn(|_| String::new()),
            gender: Gender::from_code(0),
        }
    }

    /// An all-zero active monster with `number`.
    pub fn clear_active_monster(
        number: ActiveMonsterId,
    ) -> silvermere_types::records::ActiveMonster {
        use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};
        silvermere_types::records::ActiveMonster {
            number,
            knownnum: MonsterNo(0),
            level: 0,
            map: silvermere_types::ids::MapId(0),
            room: RoomId(0),
            poison: 0,
            energy: 0,
            hp: 0,
            following: String::new(),
            trail: [RoomId(0); NAMTRAIL],
            hunters: [ActiveMonsterId(0); 10],
            hunting: ActiveMonsterId(0),
            name: String::new(),
            wornitem: ItemNo(0),
            weapon: ItemNo(0),
            items: [(ItemNo(0), 0); NMONITEMS],
            cash: Coins::default(),
            maxhp: 0,
            alignment: Alignment::from_code(0),
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 0,
            maxenergy: 0,
            friendly: 0,
            genroom: RoomId(0),
            inactive: 0,
            flags: silvermere_types::flags::ActiveMonsterFlags::empty(),
            montype: MonsterType::from_code(0),
            target: 0,
            hpregen: 0,
            lastdir: 0,
            lastupd: 0,
            grouprole: GroupRole::from_code(0),
            spells: [silvermere_types::records::ActiveSpell {
                spell: SpellNo(0),
                value: 0,
                rounds: 0,
            }; NMONSPELLS],
            downrounds: 0,
            created: 0,
        }
    }

    /// An all-zero class.
    pub fn clear_class(number: ClassNo) -> Class {
        Class {
            number,
            name: String::new(),
            basehp: 0,
            hplevel: 0,
            exp: 0,
            attmult: 0,
            maxenergy: 0,
            restored: 0,
            abilities: [(silvermere_types::enums::Ability::None, 0);
                silvermere_types::sizes::NRECABILS],
            magery: Magery::from_code(0),
            magelvl: 0,
            weapon: WeaponClass::from_code(0),
            armour: ArmourType::from_code(0),
            combat: 0,
            titles: TextBlockNo(0),
        }
    }

    /// An all-zero race.
    pub fn clear_race(number: RaceNo) -> Race {
        Race {
            number,
            name: String::new(),
            minint: 0,
            minwil: 0,
            minstr: 0,
            minhea: 0,
            minagi: 0,
            mincha: 0,
            hpbonus: 0,
            maxenergy: 0,
            restored: 0,
            abilities: [(silvermere_types::enums::Ability::None, 0);
                silvermere_types::sizes::NRECABILS],
            basecp: 0,
            exp: 0,
            maxint: 0,
            maxwil: 0,
            maxstr: 0,
            maxhea: 0,
            maxagi: 0,
            maxcha: 0,
        }
    }
}
