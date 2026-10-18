//! [`perform_matched_action`] and its clause handlers; the routines only the scripts call:
//! [`add_quest_exp`], [`erase_item_from_room`], [`add_user_ability`] and
//! [`run_room_exit_action`], with the Remote Action exits of the input stage that call it.

use silvermere_rules::economy;
use silvermere_rules::progression::BILLION;
use silvermere_rules::spells::{LegalBand, can_use_spell};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, DebugLevel, ExitType};
use silvermere_types::flags::{ExitFlags, PlayerFlags};
use silvermere_types::ids::{
    ItemNo, MessageNo, MonsterNo, RoomId, RoomKey, SessionId, SpellNo, TextBlockNo,
};
use silvermere_types::records::{Character, RoomItem};
use silvermere_types::sizes::{NDIRS, NHIDITEMS, NROOMITEMS};

use super::special::{HOOKS, perform_random_text_block, run_text_block};
use super::{Clause, Coin, Skill, Teleport, clauses};
use crate::commands::input::RemoteExits;
use crate::commands::parse::eq_ignore_case;
use crate::display::long_text::{display_long_text, display_long_text_to_room};
use crate::items::equip::{add_spell_to_spellbook, purge_spell_from_spellbook};
use crate::items::inventory::{
    Search, add_item_to_inventory, deduct_item_charge, find_item_in_inventory,
    remove_item_from_inventory,
};
use crate::items::limited::{self, Holder};
use crate::items::room_items::{add_item_to_room, mark_changed};
use crate::movement::exits::{Arg, LOCK_LOCKED, LOCK_OPEN, fmt_msg, live_exit, set_par1, trap};
use crate::movement::move_user::{MoveType, move_user, set_user_location};
use crate::movement::timed_exits::add_exit_to_list;
use crate::movement::{self, GenerateSpec, direction, dirname};
use crate::scheduler::Rng as _;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// The result of a clause and of a whole line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineResult {
    /// 0: nothing ran (an empty line, a `random` block that ran no line, no player).
    #[default]
    Nothing,
    /// `ACT_OK`: the clause is done and the line goes on; a line that ends so ran through.
    Ok,
    /// `ACT_STOP`: a condition failed, the rest of the line is skipped.
    Stop,
}

/// Items one line remembers for a refund.
const MAXTAKEN: usize = 100;

/// One clause's outcome: the result it sets (`None` leaves the line's result as it was,
/// for an unknown keyword) and whether the line goes on.
struct Step {
    result: Option<LineResult>,
    go_on: bool,
}

const OK: Step = Step {
    result: Some(LineResult::Ok),
    go_on: true,
};
const STOP: Step = Step {
    result: Some(LineResult::Stop),
    go_on: false,
};
/// The line ends but its result stays `ACT_OK` (`giveitem` that fell to the floor,
/// `hideitem`).
const END_OK: Step = Step {
    result: Some(LineResult::Ok),
    go_on: false,
};

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id)?.player()
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id)?.character.as_mut()
}

fn room_of(world: &World, id: SessionId) -> Option<RoomKey> {
    player(world, id).map(|p| RoomKey {
        map: p.map,
        room: p.room,
    })
}

fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// The WCCMSG record's line 0 to the player, line 1 with his first name to everyone else in
/// his room (blind players too, with their prompts, no colour). Empty lines are not sent.
pub fn show_action_message(world: &mut World, id: SessionId, msgnum: i32) {
    let Some(m) = world.messages.get(&MessageNo(msgnum)).cloned() else {
        return;
    };
    if !m.msgs[0].is_empty() {
        tell_user(
            world,
            id,
            Msg::ScriptMessage {
                text: m.msgs[0].clone(),
            },
        );
    }
    if !m.msgs[1].is_empty() {
        let Some(plr) = player(world, id) else {
            return;
        };
        let key = RoomKey {
            map: plr.map,
            room: plr.room,
        };
        let text = fmt_msg(&m.msgs[1], &[Arg::S(&plr.firstname)]);
        tell_room(
            world,
            key,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::ScriptMessage { text },
        );
    }
}

/// The clauses of one action line in turn until one stops it. Returns the result of the
/// last clause run, [`LineResult::Nothing`] for an empty line or no player.
pub fn perform_matched_action(world: &mut World, id: SessionId, line: &str) -> LineResult {
    if player(world, id).is_none() {
        return LineResult::Nothing;
    }
    let teleports = HOOKS.scripts.teleport_areas();
    let mut result = LineResult::Nothing;
    let mut taken: Vec<ItemNo> = Vec::new();
    for (text, more) in clauses(line) {
        let clause = Clause::parse(text, teleports);
        if let Clause::Unknown(c) = &clause {
            tracing::debug!(?id, clause = %c, "script clause with no keyword skipped");
        }
        let step = run_clause(world, id, &clause, more, &mut taken);
        if let Some(r) = step.result {
            result = r;
        }
        if !step.go_on {
            break;
        }
    }
    result
}

/// The refund of a failed `price` or `takeitem`: with
/// KID_GLOVES on, the items this line took are given back with 0 charges.
fn refund(world: &mut World, id: SessionId, taken: &[ItemNo]) {
    if world.config.kid_gloves {
        for &item in taken {
            add_item_to_inventory(world, id, item, 0);
        }
    }
}

/// A condition that failed: its message (when it has one) and the line stops.
fn fail(world: &mut World, id: SessionId, msg: Option<i32>) -> Step {
    if let Some(m) = msg {
        show_action_message(world, id, m);
    }
    STOP
}

fn ability(code: i32) -> Ability {
    Ability::from_code(code as u16)
}

fn run_clause(
    world: &mut World,
    id: SessionId,
    clause: &Clause,
    more: bool,
    taken: &mut Vec<ItemNo>,
) -> Step {
    let Some(plr) = player(world, id) else {
        return STOP;
    };
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    match *clause {
        Clause::Price { copper, msg } => {
            let price = u32::try_from(copper).unwrap_or(0);
            match economy::pay(plr.coins, price) {
                Some(p) => {
                    if let Some(plr) = player_mut(world, id) {
                        plr.coins = p.left;
                    }
                    OK
                }
                None => {
                    show_action_message(world, id, msg);
                    refund(world, id, taken);
                    STOP
                }
            }
        }
        Clause::Cast { spell } => {
            let spell = SpellNo(spell as i16);
            if world.spells.contains_key(&spell) && !HOOKS.scripts.cast_no_target(world, id, spell)
            {
                return STOP;
            }
            OK
        }
        Clause::GiveAbility { ability: a, value } => {
            let abl = ability(a);
            let value = value as i16;
            let Some(plr) = player_mut(world, id) else {
                return STOP;
            };
            let mut found = false;
            for slot in plr.abils.iter_mut() {
                if slot.0.code() == a as u16 {
                    found = true;
                    if abl != Ability::GiveSpell && slot.1 < value {
                        slot.1 = value;
                    }
                }
            }
            if !found && let Some(slot) = plr.abils.iter_mut().find(|s| s.0.code() == 0) {
                *slot = (abl, value);
                found = true;
                if abl == Ability::GiveSpell {
                    add_spell_to_spellbook(plr, SpellNo(value));
                }
            }
            if found { OK } else { STOP }
        }
        Clause::AddAbility { ability: a, amount } => {
            let Some(plr) = player_mut(world, id) else {
                return STOP;
            };
            if add_user_ability(plr, ability(a), amount as i16) {
                OK
            } else {
                STOP
            }
        }
        Clause::Teleport(t) => teleport(world, id, t),
        Clause::RemoveAbility { ability: a } => {
            let Some(plr) = player_mut(world, id) else {
                return STOP;
            };
            let mut found = false;
            for i in 0..plr.abils.len() {
                if plr.abils[i].0.code() == a as u16 {
                    found = true;
                    if plr.abils[i].0 == Ability::GiveSpell {
                        let spell = SpellNo(plr.abils[i].1);
                        purge_spell_from_spellbook(plr, spell);
                    }
                    plr.abils[i] = (Ability::from_code(0), 0);
                }
            }
            if found { OK } else { STOP }
        }
        // (sic) The loop also tests whether a `:` follows, so as the last clause of a line
        // (no `:` after it) the room is never looked at and the clause passes.
        Clause::NoMonsters { msg } => {
            let any = world
                .room_state
                .get(&key)
                .is_some_and(|s| s.monsters.iter().any(|m| m.0 != 0));
            if more && any {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::Monsters { msg } => match world.room_state.get(&key) {
            Some(s) if !s.monsters.iter().any(|m| m.0 != 0) => fail(world, id, Some(msg)),
            _ => OK,
        },
        Clause::NeedMonster { monster, msg } => {
            let Some(state) = world.room_state.get(&key) else {
                return OK;
            };
            let here = state.monsters.iter().filter(|m| m.0 != 0).any(|&m| {
                world
                    .active_monsters
                    .values()
                    .any(|a| a.number == m && a.knownnum == MonsterNo(monster))
            });
            if here { OK } else { fail(world, id, Some(msg)) }
        }
        // One monster generated in the player's room
        Clause::Summon { monster } => {
            let spec = GenerateSpec {
                group: 0,
                monster: MonsterNo(monster),
                minlevel: 0,
                maxlevel: 0x7FFF,
            };
            if movement::HOOKS.monsters.generate_monster(world, key, spec) {
                OK
            } else {
                STOP
            }
        }
        Clause::Message { msg } => {
            show_action_message(world, id, msg);
            OK
        }
        Clause::TakeItem { item, msg } => {
            if remove_item_from_inventory(world, id, ItemNo(item)).is_none() {
                refund(world, id, taken);
                return fail(world, id, Some(msg));
            }
            // (sic, not carried) the original misbehaves when all 100 are used;
            // the 101st is not remembered here
            if taken.len() < MAXTAKEN {
                taken.push(ItemNo(item));
            }
            OK
        }
        Clause::AddExp { exp } => {
            add_quest_exp(world, id, exp as u32);
            OK
        }
        Clause::GiveItem { item } => {
            let mark = world.session(id).map_or(0, |s| s.output.items().len());
            if add_item_to_inventory(world, id, ItemNo(item), -2) {
                return OK;
            }
            // The refusal `add_item_to_inventory` prepared is dropped
            truncate_output(world, id, mark);
            if let Some(charges) = world.items.get(&ItemNo(item)).map(|i| i.charges) {
                add_item_to_room(world, key, ItemNo(item), false, charges);
            }
            END_OK
        }
        Clause::HideItem { item } => {
            if let Some(charges) = world.items.get(&ItemNo(item)).map(|i| i.charges) {
                add_item_to_room(world, key, ItemNo(item), true, charges);
            }
            END_OK
        }
        Clause::CheckAbility { ability: a, min } => {
            let abl = ability(i32::from(a));
            match movement::HOOKS.abilities.value(world, id, abl) {
                None => STOP,
                Some(v) if v < i32::from(min) => STOP,
                Some(_) => OK,
            }
        }
        Clause::Class { class } => {
            if plr.class.0 != class as u16 {
                STOP
            } else {
                OK
            }
        }
        Clause::Race { race } => {
            if plr.race.0 != race as u16 {
                STOP
            } else {
                OK
            }
        }
        Clause::GoodAligned { limit, msg } => {
            if i32::from(plr.evil) > limit {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::EvilAligned { limit, msg } => {
            if i32::from(plr.evil) < limit {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::MinLevel { level, msg } => {
            if i32::from(plr.level) < level {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::MaxLevel { level, msg } => {
            if i32::from(plr.level) > level {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        // The inventory, then the key ring
        Clause::CheckItem { item, msg } => {
            if carries(plr, ItemNo(item)) {
                OK
            } else {
                fail(world, id, Some(msg))
            }
        }
        Clause::FailItem { item, msg } => {
            if carries(plr, ItemNo(item)) {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::Text { block } => {
            display_long_text(world, id, TextBlockNo(block), None);
            OK
        }
        Clause::RoomText { block } => {
            display_long_text_to_room(world, key, TextBlockNo(block), None);
            OK
        }
        // Visible or hidden; a room that cannot be read has none
        Clause::RoomItem { item, msg } => {
            if lies_in(world, key, ItemNo(item)) {
                OK
            } else {
                fail(world, id, Some(msg))
            }
        }
        Clause::FailRoomItem { item, msg } => {
            if lies_in(world, key, ItemNo(item)) {
                fail(world, id, Some(msg))
            } else {
                OK
            }
        }
        Clause::ClearItem { item, msg } => {
            if erase_item_from_room(world, key, ItemNo(item)) {
                OK
            } else {
                fail(world, id, Some(msg))
            }
        }
        Clause::AddEvil { points } => {
            if let Some(plr) = player_mut(world, id) {
                plr.evil = plr.evil.wrapping_add(points);
            }
            OK
        }
        // With the ability, at most `max`; without it, only a negative
        // `max` passes
        Clause::TestAbility { ability: a, max } => {
            let abl = ability(i32::from(a));
            match movement::HOOKS.abilities.value(world, id, abl) {
                None if max >= 0 => STOP,
                None => OK,
                Some(v) if v > i32::from(max) => STOP,
                Some(_) => OK,
            }
        }
        Clause::FailAbility { ability: a, msg } => {
            let abl = ability(i32::from(a));
            if movement::HOOKS.abilities.has(world, id, abl) {
                fail(world, id, Some(i32::from(msg)))
            } else {
                OK
            }
        }
        // An exit 0 to 10 of a room on the player's map
        Clause::RemoteAction {
            room,
            msg,
            action,
            exit,
        } => {
            if (0..=10).contains(&exit) {
                let target = RoomKey {
                    map: key.map,
                    room: RoomId(room as u32),
                };
                if world.room(target).is_some() {
                    run_room_exit_action(world, id, target, msg, action, exit);
                }
            }
            OK
        }
        Clause::Random { block } => {
            let r = perform_random_text_block(world, id, TextBlockNo(block));
            Step {
                result: Some(r),
                go_on: r != LineResult::Stop,
            }
        }
        // The delay is a byte, the sum wraps
        Clause::AddDelay { ticks } => {
            if let Some(plr) = player_mut(world, id) {
                plr.delay = plr.delay.wrapping_add(ticks as u8);
            }
            OK
        }
        // Mode 0: roll 0..range plus the modifier
        // against the stat; above it, the block runs and the line stops
        Clause::TestSkill {
            skill,
            modifier,
            block,
        } => {
            let value = skill_value(plr, skill);
            let roll = world.rng.range(0, skill.range()) + i32::from(modifier);
            if roll > value {
                run_text_block(world, id, TextBlockNo(i32::from(block)));
                STOP
            } else {
                OK
            }
        }
        // Mode 1
        Clause::CheckSkill { skill, min, msg } => {
            if skill_value(plr, skill) < i32::from(min) {
                fail(world, id, Some(i32::from(msg)))
            } else {
                OK
            }
        }
        Clause::GiveCoins { amount, coin } => {
            if let Some(plr) = player_mut(world, id) {
                let c = &mut plr.coins;
                let purse = match coin {
                    Coin::Runic => &mut c.runic,
                    Coin::Platinum => &mut c.platinum,
                    Coin::Gold => &mut c.gold,
                    Coin::Silver => &mut c.silver,
                    Coin::Copper => &mut c.copper,
                };
                *purse = purse.wrapping_add(amount as u32);
            }
            OK
        }
        // Tournament play must be on (TOURN_ON)
        Clause::TestTournament => {
            if world.config.tournament_play {
                OK
            } else {
                STOP
            }
        }
        Clause::CheckSpell { spell, block } => {
            if plr.spells.iter().any(|s| i32::from(s.0) == spell) {
                OK
            } else {
                run_text_block(world, id, TextBlockNo(block));
                STOP
            }
        }
        // (sic) The verb and message are read from
        // the flag number's word, so no verb ever matches: a flag outside 1-64 stops the
        // line, any other passes and changes nothing.
        Clause::Flag { flag, .. } => {
            if (1..=64).contains(&flag) {
                OK
            } else {
                STOP
            }
        }
        Clause::LearnSpell { spell } => learn_spell(world, id, spell),
        Clause::Unknown(_) => Step {
            result: None,
            go_on: true,
        },
    }
}

/// The inventory or the key ring holds the item.
fn carries(plr: &Character, item: ItemNo) -> bool {
    plr.items.contains(&item) || plr.keys.contains(&item)
}

/// The item lies in the room's visible or hidden slots.
fn lies_in(world: &World, key: RoomKey, item: ItemNo) -> bool {
    world.room_state.get(&key).is_some_and(|s| {
        s.items[..NROOMITEMS].iter().any(|r| r.item == item)
            || s.hiditems[..NHIDITEMS].iter().any(|r| r.item == item)
    })
}

/// The stat a skill clause names.
fn skill_value(plr: &Character, skill: Skill) -> i32 {
    match skill {
        Skill::Agility => i32::from(plr.agi),
        Skill::Strength => i32::from(plr.str),
        Skill::Intellect => i32::from(plr.intel),
        Skill::Wisdom => i32::from(plr.wil),
        Skill::Health => i32::from(plr.hea),
        Skill::Charm => i32::from(plr.cha),
        Skill::Spellcasting => i32::from(plr.spellcast),
        Skill::Perception => i32::from(plr.perception),
        Skill::Stealth => i32::from(plr.stealth),
        Skill::Thievery => i32::from(plr.thievery),
        Skill::Traps => i32::from(plr.traps),
        Skill::Picklocks => i32::from(plr.picklocks),
        Skill::Tracking => i32::from(plr.tracking),
        Skill::MagicResistance => i32::from(plr.mr),
        Skill::CurrentHp => i32::from(plr.hp),
        Skill::Unknown => 0,
    }
}

/// The output cleared after a refused `add_item_to_inventory`: what was queued since `mark`
/// is dropped (what was told before the clause stays).
fn truncate_output(world: &mut World, id: SessionId, mark: usize) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.output.truncate(mark);
}

/// The `teleport` clause. The player stops dragging, his followers stop following, and he
/// is moved with `set_user_location` without the room display (the room is not shown: the
/// scripts follow a teleport with their own message or text). A room or map of 0 does
/// nothing; a move that did not land stops the line.
fn teleport(world: &mut World, id: SessionId, t: Teleport) -> Step {
    let (room, map) = match t {
        Teleport::Named(i) => {
            let Some(area) = HOOKS.scripts.teleport_areas().get(i) else {
                return OK;
            };
            // A roll from 1 bounded by `roll`: range n-1, the first for a roll past the last
            // range
            let pick = if area.ranges.len() > 1 {
                let n = world.rng.range(1, area.roll);
                usize::try_from(n - 1)
                    .ok()
                    .filter(|&k| k < area.ranges.len())
                    .unwrap_or(0)
            } else {
                0
            };
            let (lo, hi) = area.ranges[pick];
            (world.rng.long(lo, hi), i64::from(area.map))
        }
        Teleport::Room { room, map } => (i64::from(room), i64::from(map)),
    };
    if room == 0 || map == 0 {
        return OK;
    }
    let target = RoomKey::new(map as u32, room as u32);
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
    }
    // All of my followers
    crate::party::stop_following_all(world, id);
    set_user_location(world, id, target, false);
    if room_of(world, id) == Some(target) {
        OK
    } else {
        STOP
    }
}

/// The `learnspell` clause.
fn learn_spell(world: &mut World, id: SessionId, spell: i16) -> Step {
    let n = SpellNo(spell);
    let Some(spl) = world.spells.get(&n).cloned() else {
        return STOP;
    };
    if spell == 0 {
        return STOP;
    }
    let Some(plr) = player(world, id) else {
        return STOP;
    };
    if plr.spellbook.contains(&n) {
        return OK;
    }
    // The spell's class, level and alignment test
    let can = world.classes.get(&plr.class).is_some_and(|cls| {
        can_use_spell(&spl, cls, plr.level, LegalBand::of_evil(plr.evil)).is_ok()
    });
    let learnt = can && player_mut(world, id).is_some_and(|p| add_spell_to_spellbook(p, n));
    if learnt {
        tell_user(world, id, Msg::LearnSpell { name: spl.name });
        OK
    } else {
        tell_user(world, id, Msg::LearnSpellRefused);
        STOP
    }
}

/// Raises every slot holding the ability by `amount`, or puts it in the first free slot.
/// Give Spell is refused. `true` when added or raised.
pub fn add_user_ability(plr: &mut Character, abl: Ability, amount: i16) -> bool {
    if abl == Ability::GiveSpell {
        return false;
    }
    let mut found = false;
    for slot in plr.abils.iter_mut() {
        if slot.0 == abl {
            found = true;
            slot.1 = slot.1.wrapping_add(amount);
        }
    }
    if !found && let Some(slot) = plr.abils.iter_mut().find(|s| s.0.code() == 0) {
        *slot = (abl, amount);
        found = true;
    }
    found
}

/// Quest experience, the `addexp` clause's only use: whole billions into `expbil`, the rest
/// into `exprem` with a carry, for a character on the 1.11 experience system (`PF_NEWEXP`);
/// nothing for another. Nothing is shown (the clause passes `show` 0), nothing saved, no
/// level or training-cap test. (sic) The "not been restructured" warning can never print:
/// kept unprinted.
pub fn add_quest_exp(world: &mut World, id: SessionId, exp: u32) {
    let Some(plr) = player_mut(world, id) else {
        return;
    };
    if !plr.flags.contains(PlayerFlags::NEWEXP) {
        return;
    }
    let mut exp = exp;
    while exp >= BILLION {
        exp -= BILLION;
        plr.expbil = plr.expbil.wrapping_add(1);
    }
    plr.exprem = plr.exprem.wrapping_add(exp);
    if plr.exprem >= BILLION {
        plr.exprem -= BILLION;
        plr.expbil = plr.expbil.wrapping_add(1);
    }
}

/// An empty room item slot (`items[i]=0`, `itemuses[i]=-1`, `itemcounts[i]=0`).
const EMPTY_SLOT: RoomItem = RoomItem {
    item: ItemNo(0),
    uses: -1,
    count: 0,
};

/// Every visible and hidden slot holding the item is emptied, a limited item's copies
/// released; item 0 erases everything (and counts as found). `false` when the item was not
/// there, the room does not exist or the item has no record.
pub fn erase_item_from_room(world: &mut World, room: RoomKey, item: ItemNo) -> bool {
    let Some(state) = world.room_state.get(&room) else {
        return false;
    };
    if item.0 == 0 {
        let all: Vec<ItemNo> = state.items[..NROOMITEMS]
            .iter()
            .chain(state.hiditems[..NHIDITEMS].iter())
            .map(|s| s.item)
            .filter(|i| i.0 != 0)
            .collect();
        for i in all {
            erase_item_from_room(world, room, i);
        }
        return true;
    }
    let Some(limit) = world.items.get(&item).map(|i| i.limit) else {
        return false;
    };
    let mut releases = 0usize;
    let mut found = false;
    if let Some(state) = world.room_state.get_mut(&room) {
        for s in state.items[..NROOMITEMS]
            .iter_mut()
            .chain(state.hiditems[..NHIDITEMS].iter_mut())
        {
            if s.item == item {
                found = true;
                if limit != 0 {
                    releases += usize::try_from(i32::from(s.count) + 1).unwrap_or(0);
                }
                *s = EMPTY_SLOT;
            }
        }
    }
    for _ in 0..releases {
        limited::release(world, item, Holder::Room(room));
    }
    if found {
        mark_changed(world, room);
    }
    found
}

/// The player's message (line 1 with his name to the rest of his own room first, then line
/// 0 to him), then by the type of exit `exitnum` of `room`: a Door or Gate swaps between
/// open and locked (and the same door from the other side), to swap back after its time; a
/// hidden trap throws everyone in the room, then every monster, through; a Hidden exit
/// loses block `actnum` (0: all; in order from the highest when `exitpar2s` is 0 or more)
/// and, with none left, opens as a visible passage, the room told. Then the player's prompt
/// (not after a sprung trap). Exit 10, past the last direction, has no type here (it has
/// no defined one).
pub fn run_room_exit_action(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    msgnum: i32,
    actnum: i32,
    exitnum: i16,
) {
    if debug_on(world, id) {
        tell_user(
            world,
            id,
            Msg::ExitActionDebug {
                action: i64::from(actnum),
                exit: i64::from(exitnum),
                room: i64::from(room.room.0 as u16 as i16),
            },
        );
    }
    let msg = (msgnum != 0)
        .then(|| world.messages.get(&MessageNo(msgnum)).cloned())
        .flatten();
    if let Some(m) = msg {
        if !m.msgs[1].is_empty()
            && let Some(plr) = player(world, id)
        {
            let here = RoomKey {
                map: plr.map,
                room: plr.room,
            };
            let text = fmt_msg(&m.msgs[1], &[Arg::S(&plr.firstname)]);
            tell_room(
                world,
                here,
                [Some(id), None],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &Msg::ScriptMessage { text },
            );
        }
        if !m.msgs[0].is_empty() {
            tell_user(
                world,
                id,
                Msg::ScriptMessage {
                    text: m.msgs[0].clone(),
                },
            );
        }
    }
    let dir = usize::try_from(exitnum)
        .ok()
        .filter(|&d| d < NDIRS)
        .and_then(|d| direction(d as i16));
    if let Some(dir) = dir
        && let Some(exit) = live_exit(world, room, dir)
    {
        match exit.kind {
            ExitType::Door | ExitType::Gate => {
                swap_door(world, room, dir, exit.par1, exit.par3);
                // the same door from the other side: it must lead back here
                let far = RoomKey {
                    map: room.map,
                    room: exit.target,
                };
                if let Some(rev) = dir.reverse()
                    && let Some(back) = live_exit(world, far, rev)
                    && back.target == room.room
                    && matches!(back.kind, ExitType::Door | ExitType::Gate)
                {
                    swap_door(world, far, rev, back.par1, back.par3);
                }
            }
            ExitType::Trap | ExitType::SpellTrap => {
                if exit.par2 != trap::HIDDEN {
                    prompt(world, id);
                    return;
                }
                let players: Vec<SessionId> = world
                    .session_ids()
                    .into_iter()
                    .filter(|&s| world.is_in_game(s) && room_of(world, s) == Some(room))
                    .collect();
                for s in players {
                    move_user(world, s, dir.code() as i16, MoveType::TrapOnPurpose.code());
                }
                let monsters: Vec<_> = world
                    .room_state
                    .get(&room)
                    .map(|st| st.monsters.iter().copied().filter(|m| m.0 != 0).collect())
                    .unwrap_or_default();
                for m in monsters {
                    movement::HOOKS.monsters.move_monster(world, m, dir);
                }
                return;
            }
            ExitType::Hidden => hidden_action(world, room, dir, exit, actnum),
            _ => {}
        }
    }
    prompt(world, id);
}

fn prompt(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
}

/// A Door or Gate between open and locked, queued to swap back after `exitpar3s` steps,
/// at least one. (sic) Under the original a negative time never comes round; here it is
/// taken as the longest delay.
fn swap_door(
    world: &mut World,
    room: RoomKey,
    dir: silvermere_types::enums::Direction,
    par1: u32,
    par3: i32,
) {
    let v = if par1 == LOCK_OPEN {
        LOCK_LOCKED
    } else {
        LOCK_OPEN
    };
    set_par1(world, room, dir, v);
    let steps = if par3 < 0 { i32::MAX } else { par3.max(1) };
    add_exit_to_list(world, room, dir, steps);
}

/// The blocks of a Hidden exit, `EXF_BLOCKED1` .. `EXF_BLOCKED10`.
const BLOCKS: [ExitFlags; 10] = [
    ExitFlags::BLOCKED1,
    ExitFlags::BLOCKED2,
    ExitFlags::BLOCKED3,
    ExitFlags::BLOCKED4,
    ExitFlags::BLOCKED5,
    ExitFlags::BLOCKED6,
    ExitFlags::BLOCKED7,
    ExitFlags::BLOCKED8,
    ExitFlags::BLOCKED9,
    ExitFlags::BLOCKED10,
];

/// The Hidden exit case of [`run_room_exit_action`].
fn hidden_action(
    world: &mut World,
    room: RoomKey,
    dir: silvermere_types::enums::Direction,
    exit: silvermere_types::records::Exit,
    actnum: i32,
) {
    let inorder = exit.par2 >= 0;
    let mut flags = ExitFlags::from_bits_retain(exit.par1);
    match actnum {
        // every block, then (falling through) the tenth
        0 => {
            for b in BLOCKS {
                flags.remove(b);
            }
        }
        10 => flags.remove(ExitFlags::BLOCKED10),
        // block n goes, in order only once block n+1 has gone
        1..=9 => {
            let n = actnum as usize;
            if !inorder || !flags.contains(BLOCKS[n]) {
                flags.remove(BLOCKS[n - 1]);
            }
        }
        _ => {}
    }
    if BLOCKS.iter().all(|b| !flags.contains(*b)) {
        flags = ExitFlags::VISBLOCKED;
        add_exit_to_list(world, room, dir, 1);
        let own = (exit.par3 != 0)
            .then(|| world.messages.get(&MessageNo(exit.par3)).cloned())
            .flatten();
        let name = dirname(world, dir);
        let msg = match own {
            Some(m) => Msg::PassageOpens {
                text: if m.msgs[0].is_empty() {
                    String::new()
                } else {
                    format!("{}\r", fmt_msg(&m.msgs[0], &[Arg::S(&name)]))
                },
            },
            None => Msg::ConcealedPassage { dir: name },
        };
        tell_room(
            world,
            room,
            [None, None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &msg,
        );
    }
    set_par1(world, room, dir, flags.bits());
}

/// Stage 7's Remote Action exits: an exit whose WCCMSG record
/// (`exitpar1s`) has one of its three texts equal to the typed line runs
/// `run_room_exit_action` on the room it leads to, with `exitpar3s` as the message and
/// `exitpar2s` as action (tens) and exit (units). An `exitpar4s` item must be carried
/// and loses a charge. (When a Text and a Remote Action exit answer the same words, the
/// original takes the lower-numbered exit; `execute_input` tests every Text exit first.)
pub struct RemoteExitWords;

impl RemoteExits for RemoteExitWords {
    fn perform(&self, world: &mut World, id: SessionId, line: &str) -> bool {
        let Some(key) = room_of(world, id) else {
            return false;
        };
        let Some(rm) = world.room(key).cloned() else {
            return false;
        };
        for exit in rm.exits.iter().take(NDIRS) {
            if exit.target.0 == 0 || exit.kind != ExitType::RemoteAction {
                continue;
            }
            let Some(m) = world.messages.get(&MessageNo(exit.par1 as i32)) else {
                continue;
            };
            if !m.msgs.iter().any(|t| eq_ignore_case(line, t)) {
                continue;
            }
            if exit.par4 != 0 {
                let Some(name) = world.items.get(&ItemNo(exit.par4)).map(|i| i.name.clone()) else {
                    continue;
                };
                let Some(plr) = player(world, id) else {
                    return false;
                };
                let f = find_item_in_inventory(world, plr, &name, Search::Any);
                let Some(item) = f.item else {
                    continue;
                };
                deduct_item_charge(world, id, item, f.charges);
            }
            let dest = RoomKey {
                map: key.map,
                room: exit.target,
            };
            if world.room(dest).is_some() {
                run_room_exit_action(
                    world,
                    id,
                    dest,
                    exit.par3,
                    i32::from(exit.par2 / 10),
                    exit.par2 % 10,
                );
                return true;
            }
        }
        false
    }
}
