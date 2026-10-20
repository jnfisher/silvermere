//! `move_user`: the heart of all movement, split into its steps.
//!
//! The steps are, in order (so the player sees the same refusal first): the status
//! refusals (held, stunned, the party march's rules), the encumbrance check, the drag's own
//! checks, the sneak roll, the exit-type gate ([`super::exits::exit_gate`]),
//! `user_allowed_in_room`
//! ([`user_allowed_in_room`]), the monsters' free attack ([`give_monsters_a_free_attack`]),
//! the move itself ([`set_user_location`]) with the noise next door, the party line, the
//! departure and arrival lines, the room display, and the followers
//! ([`handle_following_monsters`]). The usual ending (the Cast exit's epilogue and the
//! prompt of a march or a typed exit) and the early way out are the two ends of
//! [`move_user_with`].
//!
//! The sneak and drag state of SNEAK and DRAG are the arguments of [`move_user_with`].

use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Alignment, Direction, ExitType, MonsterType, RoomType};
use silvermere_types::flags::{PlayerSettings, PlayerStatus, PlayerStatus2, RoomFlags};
use silvermere_types::ids::{RoomKey, SessionId};
use silvermere_types::names::{DIRECTION_NAMES_2, DIRECTION_NAMES_3};
use silvermere_types::records::Character;
use silvermere_types::sizes::{NROOMMONS, NTRAIL};

use super::exits::{Gate, GateCtx, exit_gate, live_exit, prompt, show_room, tell_room_all};
use super::{GenerateSpec, HOOKS, direction, dirname};
use crate::display::display_entry_movement;
use crate::scheduler::Rng;
use crate::tell::tell_user;
use crate::world::{RoomState, World};

/// The `how` argument of `move_user`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MoveType {
    /// An ordinary walk.
    Walk = 0,
    /// A sneaking player's move (SNEAK).
    Sneak = 1,
    /// A move that drags another player (DRAG).
    Drag = 2,
    /// Following the party leader (`fast_update_character`).
    Follow = 3,
    /// A Text exit's words were typed (`execute_input`).
    TextWords = 4,
    /// The player bashed the door (`cmd_bash`).
    Bash = 5,
    /// A spell sends the player off in a random direction.
    Fear = 6,
    /// A trap walked into on purpose (`cmd_disarm`, `run_room_exit_action`).
    TrapOnPurpose = 7,
}

impl MoveType {
    /// The `how` value.
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// A plain move, no sneak and no drag set around it. Returns whether the player ended up in
/// another room.
pub fn move_user(world: &mut World, id: SessionId, dir: i16, how: i32) -> bool {
    move_user_with(world, id, dir, how, false, None)
}

/// [`move_user`] with the sneak and drag state: `sneak` for a sneaking move, `drag` for the
/// player dragged along.
pub fn move_user_with(
    world: &mut World,
    id: SessionId,
    dir: i16,
    how: i32,
    sneak: bool,
    drag: Option<SessionId>,
) -> bool {
    let Some(dir) = direction(dir) else {
        return false;
    };
    // (after the record check, which only a missing record fails)
    HOOKS.combat.clear_backstab_attack_type(world, id);
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    let name = plr.firstname.clone();
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let leader = world.session(id).and_then(|s| s.party.leader);
    // held or stunned
    let held = plr.status2.contains(PlayerStatus2::HELD);
    let stunned = plr.status2.contains(PlayerStatus2::KNOCKEDDOWN);
    if held || stunned {
        tell_user(
            world,
            id,
            if held {
                Msg::CantMoveHeld
            } else {
                Msg::CantMoveStunned
            },
        );
        if how == 3 {
            stop_following(world, leader, id);
        }
        return false;
    }
    // a march keeps a sneaker sneaking; nobody drags in a party
    let mut movesneak = sneak;
    let mut movedrag = drag.is_some();
    let mut dragvictim = drag;
    if how == 3 {
        let sneaking = world
            .session(id)
            .and_then(|s| s.player())
            .is_some_and(|p| p.status.contains(PlayerStatus::SNEAKING));
        if sneaking {
            movesneak = true;
            tell_user(world, id, Msg::MoveSneaking);
        }
        let dragging = world.session(id).and_then(|s| s.party.dragging);
        if dragging.is_some() {
            tell_user(world, id, Msg::MoveNoDragInParty);
            movedrag = false;
            stop_dragging(world, id);
        }
    }
    if world.room(room).is_none() {
        return false;
    }
    let room_type = world.room(room).map_or(RoomType::Normal, |r| r.room_type);
    // moving ends hiding, resting and meditating
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.hidden = 0;
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
    if get_encumbrance_percent(world, id) > 100 {
        tell_user(world, id, Msg::MoveTooHeavy);
        if how == 3 {
            stop_following(world, leader, id);
        }
        return false;
    }
    // the drag's own checks; `buf` is the " (Dragging NAME)" suffix
    let mut buf = String::new();
    if movedrag {
        match dragvictim.filter(|v| world.is_in_game(*v)) {
            Some(v) => {
                let heavy = get_encumbrance_percent(world, v) > 100;
                let (hp, vname) = world
                    .session(v)
                    .and_then(|s| s.player())
                    .map(|p| (p.hp, p.firstname.clone()))
                    .unwrap_or((0, String::new()));
                if heavy {
                    tell_user(world, v, Msg::MoveTooHeavyDragged);
                    movedrag = false;
                    stop_dragging(world, id);
                } else if hp > 0 {
                    movedrag = false;
                    stop_dragging(world, id);
                } else {
                    buf = format!(" (Dragging {vname})");
                }
            }
            None => {
                stop_dragging(world, id);
                buf = " (Dragging something)".to_owned();
            }
        }
    }
    // the sneak roll
    let mut unseen = false;
    if movesneak {
        if HOOKS.abilities.has(world, id, Ability::PerfectStealth) {
            unseen = true;
        } else {
            let chance = calc_hide_sneak_chance(world, id, 100, Some(id));
            if world.rng.range(0, 101) < chance {
                unseen = true;
            } else if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                p.status.remove(PlayerStatus::SNEAKING);
            }
        }
    }
    let Some(exit) = live_exit(world, room, dir) else {
        return false;
    };
    let mut moved = false;
    let mut prompt_after = how == 3 || how == 4;
    if exit.target.0 != 0 {
        clear_search_flags(world, id, room);
        let ctx = GateCtx {
            id,
            name: name.clone(),
            room,
            room_type,
            dir,
            exit,
            how,
            drag: dragvictim,
            movedrag,
            drag_suffix: buf.clone(),
            unseen,
        };
        let gate = exit_gate(world, &ctx);
        let mut finish = true;
        match gate {
            Gate::Pass { newmap, seen } => {
                if seen {
                    unseen = false;
                }
                let target = RoomKey {
                    map: newmap,
                    room: exit.target,
                };
                if !user_allowed_in_room(world, id, target, room_type, false) {
                    tell_user(world, id, Msg::NotPermitted);
                } else {
                    let mut stay = false;
                    if !unseen {
                        if give_monsters_a_free_attack(world, id) {
                            stay = true;
                        } else {
                            let felled =
                                world.session(id).and_then(|s| s.player()).is_none_or(|p| {
                                    p.status2.intersects(
                                        PlayerStatus2::HELD | PlayerStatus2::KNOCKEDDOWN,
                                    )
                                });
                            if felled {
                                if how == 3 {
                                    stop_following(world, leader, id);
                                }
                                // the early way out
                                return false;
                            }
                        }
                    }
                    if !stay {
                        moved = true;
                        perform_move(
                            world, id, &name, room, target, dir, how, movesneak, unseen, movedrag,
                            dragvictim, &buf,
                        );
                    }
                }
            }
            Gate::Refused | Gate::Stay => {}
            Gate::Carried { out } => {
                moved = true;
                if out {
                    finish = false;
                    prompt_after = false;
                }
            }
        }
        // the usual ending: the Cast exit's epilogue
        if finish && exit.kind == ExitType::Cast {
            cast_epilogue(world, id, &name, &exit);
        }
    } else {
        tell_user(world, id, Msg::NoExitClr);
    }
    dragvictim = None;
    let _ = dragvictim;
    if prompt_after {
        prompt(world, id);
    }
    moved
}

/// The Cast exit's epilogue after any attempt: spell `exitpar2s` too, and `msgs[2]` of
/// message `exitpar3s` to the room the player is in now.
fn cast_epilogue(
    world: &mut World,
    id: SessionId,
    name: &str,
    exit: &silvermere_types::records::Exit,
) {
    if exit.par2 != 0 {
        HOOKS
            .spells
            .room_cast_on_user(world, id, i32::from(exit.par2));
    }
    if exit.par3 == 0 {
        return;
    }
    let Some(m) = world
        .messages
        .get(&silvermere_types::ids::MessageNo(exit.par3))
        .map(|m| m.msgs.clone())
    else {
        return;
    };
    if m[2].is_empty() {
        return;
    }
    let Some(here) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let text = super::exits::fmt_msg(&m[2], &[super::exits::Arg::S(name)]);
    tell_room_all(world, here, [Some(id), None], Msg::ExitTextPlain { text });
}

/// The move itself and its messages: the player (and the one he drags) into `target`, the
/// noise next door, the party line, the sneak notices or the departure and arrival lines,
/// the room, the followers.
#[allow(clippy::too_many_arguments)]
fn perform_move(
    world: &mut World,
    id: SessionId,
    name: &str,
    from: RoomKey,
    target: RoomKey,
    dir: Direction,
    how: i32,
    movesneak: bool,
    unseen: bool,
    movedrag: bool,
    dragvictim: Option<SessionId>,
    buf: &str,
) {
    let rev = dir.reverse().unwrap_or(Direction::North);
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.status |= PlayerStatus::JUSTARRIVED;
    }
    set_user_location(world, id, target, false);
    if movedrag && let Some(v) = dragvictim {
        set_user_location(world, v, target, false);
    }
    if how != 3 && (!movesneak || !unseen) {
        display_entry_movement(world, Some(rev), target);
    }
    if how == 3 {
        tell_user(
            world,
            id,
            Msg::FollowLeader {
                dir: dirname(world, dir),
            },
        );
    }
    let blind_follow = world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.settings.contains(PlayerSettings::BLINDFOLLOW));
    if !unseen {
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.hidden = 0;
        }
        if movesneak {
            let perception = world
                .session(id)
                .and_then(|s| s.player())
                .map_or(0, |p| i32::from(p.perception));
            if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                p.status.remove(PlayerStatus::SNEAKING);
            }
            if world.rng.range(0, 100) < perception {
                tell_user(world, id, Msg::MadeSound);
            }
            let who = name.to_owned();
            let drag = buf.to_owned();
            let out = match dir {
                Direction::Up => Msg::SneakOutUp {
                    who: who.clone(),
                    drag: drag.clone(),
                },
                Direction::Down => Msg::SneakOutDown {
                    who: who.clone(),
                    drag: drag.clone(),
                },
                _ => Msg::SneakOutDir {
                    who: who.clone(),
                    dir: dirname(world, dir),
                    drag: drag.clone(),
                },
            };
            tell_room_all(world, from, [Some(id), dragvictim], out);
            let inm = match rev {
                Direction::Up => Msg::SneakInAbove { who, drag },
                Direction::Down => Msg::SneakInBelow { who, drag },
                _ => Msg::SneakInDir {
                    who,
                    dir: dirname(world, rev),
                    drag,
                },
            };
            tell_room_all(world, target, [Some(id), dragvictim], inm);
        } else {
            // the plain walk, one line each way
            let who = name.to_owned();
            let drag = buf.to_owned();
            let d = usize::from(dir.code());
            let left = match dir {
                Direction::Up => Msg::LeftUp {
                    who: who.clone(),
                    drag: drag.clone(),
                },
                Direction::Down => Msg::LeftDown {
                    who: who.clone(),
                    drag: drag.clone(),
                },
                _ => match world.rng.range(1, 4) {
                    2 => Msg::LeftDir2 {
                        who: who.clone(),
                        dir: DIRECTION_NAMES_2[d].to_owned(),
                        drag: drag.clone(),
                    },
                    3 => Msg::LeftDir3 {
                        who: who.clone(),
                        dir: DIRECTION_NAMES_3[d].to_owned(),
                        drag: drag.clone(),
                    },
                    _ => Msg::LeftDir1 {
                        who: who.clone(),
                        dir: dirname(world, dir),
                        drag: drag.clone(),
                    },
                },
            };
            tell_room_all(world, from, [Some(id), dragvictim], left);
            let arrive = match rev {
                Direction::Up => Msg::ArriveAbove { who, drag },
                Direction::Down => Msg::ArriveBelow { who, drag },
                _ => Msg::ArriveDir {
                    who,
                    dir: dirname(world, rev),
                    drag,
                },
            };
            tell_room_all(world, target, [Some(id), dragvictim], arrive);
        }
        tell_user(world, id, Msg::AnsiNormal);
        if movedrag && let Some(v) = dragvictim {
            let vname = world
                .session(v)
                .and_then(|s| s.player())
                .map_or(String::new(), |p| p.firstname.clone());
            tell_user(world, id, Msg::YouAreDragging { who: vname });
        }
        if how != 3 || !blind_follow {
            show_room(world, id);
        }
        if movedrag && let Some(v) = dragvictim {
            tell_user(
                world,
                v,
                Msg::DraggingYouWhite {
                    who: name.to_owned(),
                },
            );
            show_room(world, v);
            prompt(world, v);
        }
    } else if how == 3 && blind_follow {
        // nothing to show (sic, an empty branch)
    } else {
        show_room(world, id);
    }
    if !unseen {
        handle_following_monsters(world, id, from, dir);
    }
}

/// The player stops dragging.
pub fn stop_dragging(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
        if let Some(p) = s.character.as_mut() {
            p.status.remove(PlayerStatus::DRAGGING);
        }
    }
}

pub use crate::party::stop_following;

/// The load as a percentage of the maximum, stored in `encumpct`. 100 with no character.
pub fn get_encumbrance_percent(world: &mut World, id: SessionId) -> i16 {
    crate::items::inventory::update_encumbrance(world, id)
}

/// The chance to hide or sneak: stealth less 10 over 66% load or 5 over 33%, two thirds
/// when spotted, capped at 100, one less per other player in the room (but `except`) and
/// per monster, kept between 0 and `maxchance`.
pub fn calc_hide_sneak_chance(
    world: &World,
    id: SessionId,
    maxchance: i32,
    except: Option<SessionId>,
) -> i32 {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return 0;
    };
    let mut chance = i32::from(plr.stealth);
    if plr.encumpct > 66 {
        chance -= 10;
    } else if plr.encumpct > 33 {
        chance -= 5;
    }
    if plr.status.contains(PlayerStatus::SPOTTED) {
        chance = chance * 2 / 3;
    }
    chance = chance.min(100);
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    for other in world.session_ids() {
        if Some(other) == except || !world.is_in_game(other) {
            continue;
        }
        if world
            .session(other)
            .and_then(|s| s.room_key())
            .is_some_and(|k| k == room)
        {
            chance -= 1;
        }
    }
    if let Some(state) = world.room_state.get(&room) {
        chance -= state.monsters.iter().filter(|m| m.0 != 0).count() as i32;
    }
    chance.clamp(0, maxchance)
}

/// The player stops counting as a searcher of `room`; when the last one goes, the found
/// slots hide again.
pub fn clear_search_flags(world: &mut World, id: SessionId, room: RoomKey) {
    let World {
        sessions,
        room_state,
        ..
    } = world;
    if let Some(plr) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) {
        clear_search_flags_of(plr, room_state.get_mut(&room));
    }
}

/// [`clear_search_flags`] for a caller that already holds the character and the room's
/// state (`commit_suicide`, which has the `World` taken apart).
pub fn clear_search_flags_of(plr: &mut Character, state: Option<&mut RoomState>) {
    if !plr.status.contains(PlayerStatus::SEARCHED) {
        return;
    }
    plr.status.remove(PlayerStatus::SEARCHED);
    if let Some(state) = state
        && state.searchers > 0
    {
        state.searchers -= 1;
        if state.searchers == 0 {
            state.foundmask = 0;
        }
    }
}

/// The character into `room` without any of the walking checks or messages. Unless
/// `no_history`: the trail of rooms walked is pushed, a room with an entry spell casts it
/// (not when the old room has the same), and entering a colliseum from outside under the
/// arena-safe option keeps his hit points for later.
pub fn set_user_location(world: &mut World, id: SessionId, room: RoomKey, no_history: bool) {
    let Some(rm) = world.room(room) else {
        return;
    };
    let (entry_spell, roomspell, is_coll) = (
        rm.flags.contains(RoomFlags::ENTRYSPELL),
        rm.roomspell,
        rm.room_type == RoomType::Colliseum,
    );
    let Some(old) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    if !no_history {
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.trailmaps.copy_within(0..NTRAIL - 1, 1);
            p.trailmaps[0] = room.map;
            p.trailrooms.copy_within(0..NTRAIL - 1, 1);
            p.trailrooms[0] = room.room;
        }
        let oldrm = world.room(old).map(|r| (r.roomspell, r.room_type));
        if entry_spell
            && let Some((oldspell, _)) = oldrm
            && oldspell != roomspell
        {
            HOOKS.spells.room_cast_on_user(world, id, roomspell);
        }
        if is_coll
            && world.display.arena_safe
            && let Some((_, oldtype)) = oldrm
            && oldtype != RoomType::Colliseum
            && let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut())
        {
            p.arenahp = p.hp;
        }
    }
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.room = room.room;
        p.map = room.map;
    }
}

/// May the player enter `room`, coming from a room of type `from_type`? The room's specific
/// monster is created if it is not alive and an empty lair fills up (T-035's
/// `generate_monster`); then, unless `look_only`, the entry rules, each telling the player
/// why not.
pub fn user_allowed_in_room(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    from_type: RoomType,
    look_only: bool,
) -> bool {
    let Some(rm) = world.room(room).cloned() else {
        return false;
    };
    let plr = world.session(id).and_then(|s| s.player()).cloned();
    if rm.room_type == RoomType::Arena
        && let Some(p) = &plr
        && i32::from(p.level) > i32::from(rm.maxlevel)
    {
        tell_user(world, id, Msg::RoomTooFar);
        return false;
    }
    let specalive = world.room_state.get(&room).is_some_and(|s| s.specalive);
    if rm.specmon.0 != 0 && !specalive {
        HOOKS.monsters.generate_monster(
            world,
            room,
            GenerateSpec {
                group: 0,
                monster: rm.specmon,
                minlevel: 0,
                maxlevel: 0x7FFF,
            },
        );
    }
    if rm.room_type == RoomType::Lair {
        let found = world.session_ids().into_iter().any(|i| {
            world.is_in_game(i)
                && world
                    .session(i)
                    .and_then(|s| s.room_key())
                    .is_some_and(|k| k == room)
        });
        if !found {
            let spec = GenerateSpec {
                group: rm.mongroup,
                monster: rm.genmonster,
                minlevel: rm.minlevel,
                maxlevel: rm.maxlevel,
            };
            let mut j = 0;
            while HOOKS.monsters.generate_monster(world, room, spec) && j < 15 {
                j += 1;
            }
        }
    }
    if !look_only {
        if !world.config.tournament_play && rm.flags.contains(RoomFlags::PROTECTED) {
            if HOOKS.combat.in_autocombat(world, id) {
                tell_user(world, id, Msg::RoomInCombat);
                return false;
            }
            if HOOKS.combat.in_retaliation(world, id) {
                tell_user(world, id, Msg::RoomRetaliation);
                return false;
            }
        }
        let arena_safe = world.display.arena_safe;
        if arena_safe && from_type == RoomType::Colliseum && rm.room_type != RoomType::Colliseum {
            HOOKS.combat.kill_autocombat_against(world, id);
            HOOKS.combat.kill_autocombat(world, id);
        }
        if arena_safe && rm.room_type == RoomType::Colliseum && from_type != RoomType::Colliseum {
            if let Some(p) = &plr
                && p.hp < p.maxhp / 2
            {
                tell_user(world, id, Msg::RoomUnhealthy);
                return false;
            }
            if HOOKS.combat.in_retaliation(world, id) {
                tell_user(world, id, Msg::RoomRetaliation);
                return false;
            }
        }
    }
    true
}

/// The monsters' free attack on a player walking in: one roll of 0-99 for the room; the
/// first monster whose follow chance is at least the roll and whose alignment lets it (good
/// ones only strike a player they follow and are not friends with; a Lawful Evil one spares
/// a player with 80 or more evil points) strikes one round through T-033's
/// `attack_monster_user`. Returns whether the player lost a life.
pub fn give_monsters_a_free_attack(world: &mut World, id: SessionId) -> bool {
    let roll = i16::from(world.rng.pct());
    let Some(plr) = world.session(id).and_then(|s| s.player()).cloned() else {
        return false;
    };
    let lives = plr.lives;
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let Some(slots) = world.room_state.get(&room).map(|s| s.monsters) else {
        return false;
    };
    for monnum in slots {
        let attackers = world
            .session(id)
            .and_then(|s| s.player())
            .map_or(1, |p| p.attackers);
        if attackers >= 1 {
            break;
        }
        if monnum.0 == 0 {
            continue;
        }
        let Some(mon) = world
            .active_monsters
            .values()
            .find(|m| m.number == monnum)
            .cloned()
        else {
            continue;
        };
        if mon.followpct < roll {
            continue;
        }
        let follows_me = mon.following.eq_ignore_ascii_case(&plr.firstname);
        let good = matches!(
            mon.alignment,
            Alignment::LawfulGood | Alignment::Good | Alignment::Neutral
        ) || mon.montype == MonsterType::Angel;
        let strikes = if !good {
            (mon.alignment != Alignment::LawfulEvil || plr.evil < 80)
                && (!follows_me || mon.friendly == 0)
        } else {
            follows_me && mon.friendly == 0
        };
        if strikes {
            if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                p.attackers += 1;
            }
            HOOKS.combat.attack_monster_user(world, monnum, id);
            break;
        }
    }
    world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.lives != lives)
}

/// Every monster in the room left that follows the player comes along with a chance of
/// `followpct` percent (T-035's `monsters::move_monster`).
pub fn handle_following_monsters(world: &mut World, id: SessionId, from: RoomKey, dir: Direction) {
    let Some(name) = world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| p.firstname.clone())
    else {
        return;
    };
    if !world.room_state.contains_key(&from) {
        return;
    }
    // slot i of the room is read afresh on every pass, since a move changes the room's list
    for i in 0..NROOMMONS {
        let Some(monnum) = world.room_state.get(&from).map(|s| s.monsters[i]) else {
            return;
        };
        if monnum.0 == 0 {
            continue;
        }
        let Some((following, followpct)) = world
            .active_monsters
            .by_number(monnum)
            .map(|m| (m.following.clone(), m.followpct))
        else {
            continue;
        };
        if following.eq_ignore_ascii_case(&name) && i16::from(world.rng.pct()) < followpct {
            HOOKS.monsters.move_monster(world, monnum, dir);
        }
    }
}
