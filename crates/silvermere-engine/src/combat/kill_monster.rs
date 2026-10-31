//! A monster's death: [`check_kill_monster`] and the hand-over of the experience
//! ([`distribute_experience`], shared out by `experience_split`).

use silvermere_text::Msg;
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId};

use super::monster;
use crate::items::limited::{self, Holder};
use crate::items::room_items::mark_changed;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// The room's regeneration clock, the minute of the day: [`World::minute_stamp`], the one
/// copy of the past-midnight rule. The game's clock is the host's local time;
/// [`World::minute`] is the UTC minute of the day, which is the host's local time in the
/// shipped container (it runs in UTC).
fn kill_minute(world: &World) -> u16 {
    world.minute_stamp()
}

/// The monster's death, if it has died. `killer` is the player who struck the blow, `None`
/// for a monster or a spell. Returns whether the monster was dead (and is now gone).
///
/// The generating room's counters go down (its specific monster may come back; other
/// monsters free a place and restart the regeneration clock, an area's count kept in its
/// controlling room); the dying room's clock restarts and it loses the monster; the coins
/// drop, one line a currency (the killer sees them); the death message's room text goes to
/// everyone there; the carried items are left in or near the room; the known monster counts
/// one fewer alive; the active monster is removed (`remove_active_monster`). The death
/// spell is T-052's.
pub fn check_kill_monster(
    world: &mut World,
    monnum: ActiveMonsterId,
    killer: Option<SessionId>,
) -> bool {
    let Some(mon) = monster(world, monnum).cloned() else {
        return false;
    };
    if mon.hp > 0 {
        return false;
    }
    let now = kill_minute(world);
    // the room that generated it
    let genkey = RoomKey {
        map: mon.map,
        room: mon.genroom,
    };
    if let Some(rm) = world.room(genkey).cloned() {
        if mon.knownnum == rm.specmon {
            if let Some(rs) = world.room_state.get_mut(&genkey) {
                rs.specalive = false;
            }
            mark_changed(world, genkey);
        } else {
            if let Some(rs) = world.room_state.get_mut(&genkey) {
                rs.nummonsters = rs.nummonsters.saturating_sub(1);
                rs.lastkill = now;
            }
            mark_changed(world, genkey);
            if rm.ctlroom.0 != 0 {
                let ctl = if rm.ctlroom == rm.key.room {
                    genkey
                } else {
                    RoomKey {
                        map: mon.map,
                        room: rm.ctlroom,
                    }
                };
                if let Some(rs) = world.room_state.get_mut(&ctl)
                    && rs.areacount > 0
                {
                    rs.areacount -= 1;
                    mark_changed(world, ctl);
                }
            }
        }
    }
    // the room where it died
    let here = RoomKey {
        map: mon.map,
        room: mon.room,
    };
    let room_known = world.room_state.contains_key(&here);
    if let Some(rs) = world.room_state.get_mut(&here) {
        rs.lastkill = now;
        for slot in rs.monsters.iter_mut() {
            if *slot == monnum {
                *slot = ActiveMonsterId(0);
            }
        }
    } else if killer.is_some() {
        return false;
    }
    // the coins, one line a currency; the killer sees them
    if room_known {
        let mut lines = vec![Msg::CoinsDropStart];
        let cash = mon.cash;
        let names = [
            (cash.runic, world.config.usr_cur_name.clone()),
            (cash.platinum, "platinum".to_owned()),
            (cash.gold, "gold".to_owned()),
            (cash.silver, "silver".to_owned()),
            (cash.copper, "copper".to_owned()),
        ];
        for (n, coin) in names {
            if n > 0 {
                lines.push(Msg::CoinsDrop { n, coin });
            }
        }
        if let Some(rs) = world.room_state.get_mut(&here) {
            let c = &mut rs.coins;
            c.runic = c.runic.wrapping_add(cash.runic);
            c.platinum = c.platinum.wrapping_add(cash.platinum);
            c.gold = c.gold.wrapping_add(cash.gold);
            c.silver = c.silver.wrapping_add(cash.silver);
            c.copper = c.copper.wrapping_add(cash.copper);
        }
        mark_changed(world, here);
        if let Some(k) = killer {
            for m in lines {
                tell_user(world, k, m);
            }
        }
    }
    // the known monster's record
    if let Some(kmon) = world.monsters.get(&mon.knownnum).cloned() {
        // the death spell, at level 1, at nobody
        if kmon.deathspell.0 != 0 {
            crate::monsters::cast::cast_by_number(world, monnum, kmon.deathspell, 1, None);
        }
        // the death message's room text, to everyone there
        if kmon.deathmsg.0 != 0
            && let Some(m) = world.messages.get(&kmon.deathmsg)
        {
            let text = m.msgs[2].clone();
            tell_room(
                world,
                here,
                [None, None],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &Msg::MonsterDeathText { text },
            );
        }
        // the carried items, released and left in or near the room
        for &(item, uses) in &mon.items {
            if item.0 == 0 || !world.items.contains_key(&item) {
                continue;
            }
            if limited::release(world, item, Holder::Monster(monnum)) {
                crate::items::room_items::dispose_of_item_in_room(world, here, item, uses);
            }
        }
        // one fewer alive
        if let Some(k) = world.monsters.get_mut(&mon.knownnum)
            && k.curalive > 0
        {
            k.curalive -= 1;
            k.killed_at = world.today + i64::from(world.minute) * 60;
        }
    }
    if world.monsters.contains_key(&mon.knownnum) {
        crate::monsters::active::save_known_monster(world, mon.knownnum);
    }
    // the record is deleted and its number goes to the head of the free list
    crate::monsters::active::delete_active_record(world, monnum);
    true
}

/// The share-out of [`super::experience_split::distribute_experience`]. Exactly one victim
/// must be named; with none or both nothing happens.
pub fn distribute_experience(
    world: &mut World,
    killer: Option<SessionId>,
    exp: u32,
    victim_user: Option<SessionId>,
    victim_monster: Option<ActiveMonsterId>,
    room: RoomKey,
) {
    let victim = match (victim_user, victim_monster) {
        (Some(u), None) => super::AutoTarget::Player(u),
        (None, Some(m)) => super::AutoTarget::Monster(m),
        _ => return,
    };
    super::experience_split::distribute_experience(world, killer, exp, victim, room);
}
