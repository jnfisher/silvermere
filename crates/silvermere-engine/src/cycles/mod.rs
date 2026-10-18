//! The background cycles: the slow, medium and per-tick fast passes, with the per-player
//! and per-monster updates they run (`slow_update_character`, `medium_update_character`,
//! `fast_update_character`, `slow_update_monster`).
//!
//! The scheduler (`crate::scheduler`) owns the queue, the periods and the order the
//! routines fall due in; it calls into here through the `World` methods
//! (`World::slow_cycle`, `medium_cycle`, `fast_update_characters`, `save_cycle`). This
//! module owns what each pass does and in what order:
//!
//! | Pass | Order | File |
//! |---|---|---|
//! | slow, 30 s | shuffle the session order; each player (moves, evil timers, hunger, thirst, poison, bleed or aid, HP regen, mana regen, `sesstime`, prompt); each monster (poison, regen); then the scheduler's restock count; the direction letters; the random monster text | [`slow`], [`monsters`] |
//! | medium, 3 s | each player (room spell every other pass, per-period counters, `moves` unless over 66%, `ST_JUSTARRIVED`, light burn, light timer, spell rounds); then the monsters (T-035) | [`medium`] |
//! | fast, 1 s | each player (follow the leader, rest every 21st tick, meditate every 15th, the stun, the command delay and queue); then the monsters (T-035) | [`fast`] |
//! | polls, after each tick | the pending monster passes, a cache slot at a time, while anyone is in the game (T-086) | [`monsters`] |
//! | save, 60 s | T-023's round-robin `save_characters`, through `World::save_cycle` | (T-023) |
//!
//! Work other cards own is reached through [`CycleHooks`], function pointers with
//! defaults that keep the engine running until those cards land; tests install their own.
//!
//! Not carried (the card's list): `cycflags`, the sysop status update, the credit charging, the
//! `pollrate`/`btvsaves` adjustment, the buffer saves, the emulation input of the medium cycle, and
//! the current-user switch. The monster passes walk the monster cache a slot a poll (T-086,
//! [`monsters`]).

pub mod fast;
pub mod medium;
pub mod monsters;
pub mod slow;

use silvermere_types::ids::SessionId;
use silvermere_types::sizes::NDIRS;

use crate::world::World;

/// The background counters of one session (`resttick`, `medtick`); they live with the
/// session and are never persisted.
/// `sesstime` stays on the character and is stored: the midnight
/// cleanup folds it into `lifemins` (T-056). `delay`, `moves` and `pvptimer` stay on the
/// character too, where the command, movement and combat cards read them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionTicks {
    /// Fast ticks of resting since the last rest heal, 0..20.
    pub resttick: u8,
    /// Fast ticks of meditating since the last mana tick, 0..14.
    pub medtick: u8,
}

/// How a player moves (`move_user`'s third argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// 0: walking (the queued move).
    Walk,
    /// 3: following the party leader.
    Follow,
}

/// Work the cycles call that other cards own. Each default is safe to run before its
/// card lands.
#[derive(Debug, Clone, Copy)]
pub struct CycleHooks {
    /// Moves the player through the exit; `false` when the
    /// move failed. Default: T-026's [`crate::movement::move_user`].
    pub move_user: fn(&mut World, SessionId, u8, MoveKind) -> bool,
    /// T-033's death check: handles a death; `true` when
    /// the player died. Default: [`default_check_kill_user`].
    pub check_kill_user: fn(&mut World, SessionId) -> bool,
    /// Knocks the player out (T-033). Default: [`knock_out_user`].
    pub knock_out_user: fn(&mut World, SessionId),
    /// The command delay and the typed-ahead queue of `fast_update_character`
    /// (T-024's `fast_update_delay`, T-021's exit countdown).
    /// Default: [`fast::delay_step`].
    pub delay_step: fn(&mut World, SessionId),
    /// The room's spell cast on the player. Default: T-051's
    /// [`crate::spells::room::cast_spell_number`].
    pub room_spell: fn(&mut World, SessionId, i32),
    /// The spell rounds of `medium_update_character`: `true` when a
    /// spell killed the player. Default: [`crate::spells::active::tick_player_spells`].
    pub spell_rounds: fn(&mut World, SessionId) -> bool,
    /// The medium update for the monster in a cache slot.
    /// Default: T-035's `monsters::following::medium_update_monster`.
    pub medium_update_monster: fn(&mut World, silvermere_types::ids::ActiveMonsterId),
    /// The kill check for a monster below 0 hit points in the medium
    /// pass: `true` when it died. Default: T-033's `combat::check_kill_monster` with no
    /// killer.
    pub check_kill_monster: fn(&mut World, silvermere_types::ids::ActiveMonsterId) -> bool,
}

impl Default for CycleHooks {
    fn default() -> Self {
        Self {
            move_user: |world, id, dir, kind| {
                let how = match kind {
                    MoveKind::Walk => 0,
                    MoveKind::Follow => 3,
                };
                crate::movement::move_user(world, id, i16::from(dir), how)
            },
            check_kill_user: |world, id| crate::combat::check_kill_user(world, id).died(),
            knock_out_user,
            delay_step: fast::delay_step,
            room_spell: crate::spells::room::cast_spell_number,
            spell_rounds: crate::spells::active::tick_player_spells,
            medium_update_monster: crate::monsters::following::medium_update_monster,
            check_kill_monster: |w, n| crate::combat::check_kill_monster(w, n, None),
        }
    }
}

/// The cycles' own state, kept from one pass to the next.
#[derive(Debug, Clone)]
pub struct CycleState {
    /// `altpass`: flips each medium pass; room spells hit on the
    /// passes where it is set.
    pub altpass: bool,
    /// `slowmoncycle`: the slow monster pass number, stored in `actmon.lastupd`.
    pub slowmoncycle: i32,
    /// The monster passes' cache cursors (T-086).
    pub monster_passes: monsters::MonsterPasses,
    /// `mappings[]`: the session order of the slow cycle's random event, reshuffled at
    /// the start of every slow cycle ([`slow::shuffle_mappings`]).
    pub order: Vec<SessionId>,
    /// The anti-script letters of `dirnames[i][1]`, `dirnames2[i][2]`, `dirnames3[i][3]`,
    /// `b' '` until the first slow cycle.
    pub dir_letters: [[u8; 3]; NDIRS],
    /// Slow cycles run.
    pub slow_runs: u64,
    /// Medium cycles run.
    pub medium_runs: u64,
    /// The hooks into other cards' work.
    pub hooks: CycleHooks,
}

impl Default for CycleState {
    fn default() -> Self {
        Self {
            altpass: false,
            slowmoncycle: 0,
            monster_passes: monsters::MonsterPasses::default(),
            order: Vec::new(),
            dir_letters: [[b' '; 3]; NDIRS],
            slow_runs: 0,
            medium_runs: 0,
            hooks: CycleHooks::default(),
        }
    }
}

impl CycleState {
    /// A direction name with the current anti-script letter in its slot: `table` is 0
    /// for `dirnames`, 1 for `dirnames2`, 2 for `dirnames3`.
    pub fn direction_name(&self, table: usize, dir: usize) -> String {
        use silvermere_types::names::{DIRECTION_NAMES, DIRECTION_NAMES_2, DIRECTION_NAMES_3};
        let names = [DIRECTION_NAMES, DIRECTION_NAMES_2, DIRECTION_NAMES_3];
        let mut bytes = names[table][dir].as_bytes().to_vec();
        // The slot is the space before the backspace: index 1, 2 or 3.
        bytes[table + 1] = self.dir_letters[dir][table];
        String::from_utf8(bytes).expect("ASCII")
    }
}

impl World {
    /// The session order of the last slow cycle's shuffle (`mappings[]`), the order the
    /// random event and T-036's combat round visit the players in. Sessions that joined
    /// since come after, in id order.
    pub fn shuffled_sessions(&self) -> Vec<SessionId> {
        let mut out: Vec<SessionId> = self
            .cycles
            .order
            .iter()
            .copied()
            .filter(|id| self.sessions.contains_key(id))
            .collect();
        for id in self.session_ids() {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

/// The default death check until T-033's full one lands:
/// `false` while the player has hit points, or is only unconscious (above `deathhp`,
/// where the dragging is let go; a safe coliseum has no unconscious state);
/// otherwise `true`, with none of the death's consequences, which are T-033's.
pub fn default_check_kill_user(world: &mut World, id: SessionId) -> bool {
    use silvermere_types::enums::RoomType;
    let death_hp = world.config.death_hp;
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    if plr.hp > 0 {
        return false;
    }
    let key = silvermere_types::ids::RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let safe_arena = world.display.arena_safe
        && world
            .room(key)
            .is_some_and(|r| r.room_type == RoomType::Colliseum);
    if !safe_arena && i32::from(plr.hp) > death_hp {
        if let Some(s) = world.session_mut(id) {
            s.party.dragging = None;
        }
        return false;
    }
    true
}

/// Knocks the player out: the player stops moving, dragging,
/// resting and meditating, and the whole room (the player included, blinded players
/// too) is told `%s drops to the ground!` with the prompt.
pub fn knock_out_user(world: &mut World, id: SessionId) {
    use silvermere_text::Msg;
    use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.party.dragging = None;
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    plr.movedir = -1;
    plr.status
        .remove(PlayerStatus::DRAGGING | PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    let room = silvermere_types::ids::RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let msg = Msg::DropsUnconscious {
        who: plr.firstname.clone(),
    };
    crate::tell::tell_room(
        world,
        room,
        [None, None],
        crate::tell::Noise::Always,
        crate::tell::TellBlind::All,
        true,
        None,
        &msg,
    );
}

/// Pushes the player's prompt.
pub(crate) fn prompt(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
}

/// The player's room key, if a character is loaded.
pub(crate) fn room_of(world: &World, id: SessionId) -> Option<silvermere_types::ids::RoomKey> {
    world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| silvermere_types::ids::RoomKey {
            map: p.map,
            room: p.room,
        })
}
