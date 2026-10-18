//! The medium cycle, every `secmed` (3) seconds: [`medium_cycle`] and
//! [`medium_update_character`].

use silvermere_text::Msg;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ItemNo, MessageNo, SessionId};

use super::monsters;
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// Above this encumbrance percentage the move count is not
/// reset by the medium pass.
pub const HEAVYLOAD: i16 = 66;

/// The medium cycle: the players, then the monsters. Autoplay (emulated input) is not
/// carried.
pub fn medium_cycle(world: &mut World) {
    world.cycles.medium_runs += 1;
    medium_update_characters(world);
    monsters::medium_update_monsters(world);
}

/// `altpass` flips, then every session in the realm in id order.
pub fn medium_update_characters(world: &mut World) {
    world.cycles.altpass = !world.cycles.altpass;
    for id in world.session_ids() {
        if world.is_in_game(id) {
            medium_update_character(world, id);
        }
    }
}

/// One player's medium update.
pub fn medium_update_character(world: &mut World, id: SessionId) {
    let hooks = world.cycles.hooks;
    let Some(key) = super::room_of(world, id) else {
        return;
    };
    // The room's own spell, every other pass (T-051)
    let roomspell = world.room(key).map_or(0, |r| r.roomspell);
    if world.cycles.altpass && roomspell != 0 {
        (hooks.room_spell)(world, id, roomspell);
    }
    let Some(plr) = world
        .sessions
        .get_mut(&id)
        .and_then(|s| s.character.as_mut())
    else {
        return;
    };
    // The per-period counters: the telepath, gossip and auction allowances
    plr.attackers = 0;
    plr.pagecnt = 0;
    plr.auctcnt = 0;
    plr.gosscnt = 0;
    if plr.encumpct <= HEAVYLOAD {
        plr.moves = 0;
    }
    plr.status.remove(PlayerStatus::JUSTARRIVED);
    burn_light(world, id);
    // TODO(T-050): the countdown of `unk_0x0d2` is not carried: nothing
    // sets it in 1.11p.
    // The spells on the player; a spell that kills ends the pass (T-050).
    if (hooks.spell_rounds)(world, id) {
        // nothing follows the spells in this pass anyway
    }
}

/// The light part of [`medium_update_character`]: the readied
/// light source burns one charge; at the last one the item goes (unless it is kept,
/// `retain`) with its own destroy messages or the standard `is no longer lit!` pair, and
/// nothing is readied any more; while it still burns the personal light timer is topped
/// up. Then the timer runs down, and the light goes out when it ends.
pub fn burn_light(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    let mut tells: Vec<(Option<Msg>, Option<Msg>)> = Vec::new();
    let mut released: Option<(ItemNo, String)> = None;
    if plr.readied >= 0 {
        let slot = plr.readied as usize;
        let uses = plr.itemuses.get(slot).copied().unwrap_or(0).wrapping_sub(1);
        if let Some(u) = plr.itemuses.get_mut(slot) {
            *u = uses;
        }
        if uses == 0 {
            // The last charge is gone
            let number = plr.items.get(slot).copied().unwrap_or(ItemNo(0));
            if let Some(itm) = world.items.get(&number) {
                if itm.retain == 0 {
                    // The item leaves the player's logical count, after the borrow
                    released = Some((number, plr.userid.clone()));
                    // (sic) the emptied slot gets -2 uses, not the usual -1.
                    plr.itemuses[slot] = -2;
                    plr.items[slot] = ItemNo(0);
                    plr.wgt = (i32::from(plr.wgt) - i32::from(itm.weight)) as u16;
                }
                if itm.destroymsg != MessageNo(0) {
                    // The item's own lines: [0] to the player, [1] to the room
                    if let Some(msg) = world.messages.get(&itm.destroymsg) {
                        let mine = (!msg.msgs[0].is_empty()).then(|| Msg::LightDestroyLine {
                            text: printf_s(&msg.msgs[0], &[&itm.name]),
                        });
                        let theirs = (!msg.msgs[1].is_empty()).then(|| Msg::LightDestroyLine {
                            text: printf_s(&msg.msgs[1], &[&plr.firstname, &itm.name]),
                        });
                        tells.push((mine, theirs));
                    }
                } else {
                    // The standard messages
                    tells.push((
                        Some(Msg::LightOut {
                            item: itm.name.clone(),
                        }),
                        Some(Msg::LightOutOther {
                            who: plr.firstname.clone(),
                            item: itm.name.clone(),
                        }),
                    ));
                }
            }
            plr.readied = -1;
        } else if plr.lighttime != 0 {
            // Still burning: top the timer up
            plr.lighttime = plr.lighttime.wrapping_add(1);
        } else {
            plr.lighttime = plr.lighttime.wrapping_add(2);
        }
    }
    // The personal light timer runs down
    if plr.lighttime > 1 {
        plr.lighttime -= 1;
    } else {
        plr.lighttime = 0;
        plr.light = 0;
    }
    let room = silvermere_types::ids::RoomKey {
        map: plr.map,
        room: plr.room,
    };
    if let Some((item, userid)) = released {
        crate::items::limited::release(world, item, crate::items::limited::Holder::User(&userid));
    }
    for (mine, theirs) in tells {
        if let Some(m) = mine {
            crate::tell::tell_user(world, id, m);
        }
        if let Some(t) = theirs {
            tell_room(
                world,
                room,
                [Some(id), None],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &t,
            );
        }
    }
}

/// A printf for the `%s` conversions a destroy message may hold: each `%s` takes
/// the next argument (an empty string once they run out), `%%` is a `%`.
pub fn printf_s(fmt: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(fmt.len());
    let mut args = args.iter();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('s') => {
                chars.next();
                out.push_str(args.next().copied().unwrap_or(""));
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::printf_s;

    #[test]
    fn printf_s_fills_the_string_conversions() {
        assert_eq!(
            printf_s("Your %s burns out.", &["torch"]),
            "Your torch burns out."
        );
        assert_eq!(
            printf_s("%s's %s crumbles. 100%%", &["Bob", "torch"]),
            "Bob's torch crumbles. 100%"
        );
        assert_eq!(printf_s("%s and %s", &["a"]), "a and ");
    }
}
