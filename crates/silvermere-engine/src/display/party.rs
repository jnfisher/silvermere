//! The PARTY screen.

use silvermere_text::Msg;
use silvermere_types::enums::Magery;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;
use silvermere_types::sizes::NGROUP;

use crate::party::{RANK_BACK, RANK_FRONT, same_party};
use crate::tell::tell_user;
use crate::world::World;

/// The class's name; "Class E0!" for a missing record.
fn class_name(world: &World, plr: &Character) -> String {
    world
        .classes
        .get(&plr.class)
        .map_or_else(|| "Class E0!".to_owned(), |c| c.name.clone())
}

/// The full name in `%-30.30s` and the class in parentheses in `%-12.12s`.
fn name_class(world: &World, plr: &Character) -> (String, String) {
    let name = format!("{} {}", plr.firstname, plr.lastname);
    let class = format!("({})", class_name(world, plr));
    (format!("{name:<30.30}"), format!("{class:<12.12}"))
}

/// One member's line (the player's own too): `  %-30.30s %-12.12s`, ` [M:%3.3s%]` (K for a
/// kai class) or nine spaces when the mana percentage is 0, ` [H:%3.3s%]`, P poisoned, R
/// resting or M meditating, the rank.
pub fn party_row(world: &World, plr: &Character) -> Msg {
    let (name, class) = name_class(world, plr);
    // Hit points in percent of the maximum, truncating towards zero
    let hppct = i32::from(plr.hp) * 100 / i32::from(plr.maxhp).max(1);
    let manapct = if plr.maxmana > 0 {
        i32::from(plr.mana) * 100 / i32::from(plr.maxmana)
    } else {
        0
    };
    let mana = if manapct != 0 {
        let kai = world
            .classes
            .get(&plr.class)
            .is_some_and(|c| c.magery == Magery::Kai);
        format!(
            " [{}:{:>3.3}%]",
            if kai { 'K' } else { 'M' },
            manapct.to_string()
        )
    } else {
        " ".repeat(9)
    };
    let rest = if plr.status.contains(PlayerStatus::RESTING) {
        "R"
    } else if plr.status2.contains(PlayerStatus2::MEDITATING) {
        "M"
    } else {
        " "
    };
    let rank = match plr.rank {
        RANK_FRONT => " - Frontrank",
        RANK_BACK => " - Backrank",
        _ => " - Midrank",
    };
    Msg::PartyRow {
        name,
        class,
        mana,
        hp: format!("{:>3.3}", hppct.to_string()),
        poison: if plr.poison != 0 { "P" } else { " " }.to_owned(),
        rest: rest.to_owned(),
        rank: rank.to_owned(),
    }
}

/// Who the player follows; the members of his list in slot order, each as an invitation
/// (`[Invited]`) or with his health (the heading before the first); "not in a party" when
/// nobody was listed (the rank goes back to the middle); and last, always, the player's own
/// line. A leader who has left the game is forgotten, and so is a member (his slot freed).
pub fn display_party(world: &mut World, id: SessionId) {
    let Some(mut plr) = world.session(id).and_then(|s| s.player()).cloned() else {
        return;
    };
    let mut out = vec![Msg::Clrline];
    let leader = world.session(id).and_then(|s| s.party.leader);
    if let Some(l) = leader {
        match world.session(l).and_then(|s| s.player()) {
            Some(lp) => out.push(Msg::PartyFollowing {
                who: lp.firstname.clone(),
            }),
            None => {
                if let Some(s) = world.session_mut(id) {
                    s.party.leader = None;
                }
            }
        }
    }
    let mut first = true;
    for i in 0..NGROUP {
        let Some(m) = world.session(id).and_then(|s| s.party.group[i]) else {
            continue;
        };
        let tplr = world.session(m).and_then(|s| s.player()).cloned();
        let Some(tplr) = tplr.filter(|_| world.is_in_game(m)) else {
            if let Some(s) = world.session_mut(id) {
                s.party.group[i] = None;
            }
            continue;
        };
        if first {
            first = false;
            out.push(Msg::PartyHeader);
        }
        if same_party(world, id, m) {
            out.push(party_row(world, &tplr));
        } else {
            let (name, class) = name_class(world, &tplr);
            out.push(Msg::PartyRowInvited { name, class });
        }
    }
    if first {
        out.push(Msg::PartyNone);
        plr.rank = 0;
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.rank = 0;
        }
    }
    out.push(party_row(world, &plr));
    for msg in out {
        tell_user(world, id, msg);
    }
}
