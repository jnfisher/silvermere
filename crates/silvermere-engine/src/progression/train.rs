//! Training a level: [`train_level`], behind TRAIN (`cmd_train`) and the BUY of training in
//! a training hall (`buy_item`, T-041).

use silvermere_rules::economy;
use silvermere_rules::progression::{new_calc_exp_needed, train_gains, training_fee_copper};
use silvermere_text::Msg;
use silvermere_types::enums::{Currency, RoomType, ShopType};
use silvermere_types::ids::{RoomKey, SessionId};
use silvermere_types::records::Coins;

use crate::player::{CalcMode, class_title, recalculate};
use crate::world::World;

/// Returns whether the character went up a level. Every refusal and the receipt go to the
/// player; the caller's prompt follows.
///
/// The checks, in the original's order: the room is a shop of the Training type, the
/// trainer takes the character's class or every class (`trainclass` 0), the next level lies
/// in the trainer's `minlevel..=maxlevel`, the character has the experience
/// `new_calc_exp_needed(level, ..)` and the fee `level*5` silver plus the shop's markup
/// (Charm plays no part). Then: `You hand over` and the coins paid, ` and you receive
/// training to attain level N.`, the new title, `You receive the following:`, the character
/// points by band, random hit points from 0 to `hplevel` into `randomhp`, `trainlives` (5)
/// more lives up to 9 with `N additional lives` only when they changed, and the secondary
/// stats again in mode 2. The hit points come from `world.rng`.
///
/// Not carried: the DEMOLVL/NODMOKEY level cap (T-017 dropped the option), the sysop debug
/// lines. `train_level` is not refused in combat and does not save: the save sweep writes
/// the new level.
pub fn train_level(world: &mut World, id: SessionId) -> bool {
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.character.as_ref()) else {
        return false;
    };
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let Some(room) = world.rooms.get(&key) else {
        return false;
    };
    // The room must be a shop of the Training type
    if room.room_type != RoomType::Shop {
        return refuse(world, id, Msg::TrainWrongRoom);
    }
    let Some(shop) = world.shops.get(&room.shopnum) else {
        return false;
    };
    if shop.shop_type != ShopType::Training {
        return refuse(world, id, Msg::TrainWrongRoom);
    }
    if shop.trainclass.0 > 0 && shop.trainclass != plr.class {
        return refuse(world, id, Msg::TrainWrongClass);
    }
    let (minlevel, maxlevel, markup) = (shop.minlevel, shop.maxlevel, shop.markup);
    // A Kai class catches up on its powers
    if is_kai(world, id) {
        crate::spells::spellbook::update_kai_powers(world, id);
    }
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.character.as_ref()) else {
        return false;
    };
    let next = u32::from(plr.level) + 1;
    if u32::from(minlevel) > next {
        return refuse(world, id, Msg::TrainTooLow);
    }
    if u32::from(maxlevel) < next {
        return refuse(world, id, Msg::TrainTooHigh);
    }
    let (Some(cls), Some(rac)) = (world.classes.get(&plr.class), world.races.get(&plr.race)) else {
        return false;
    };
    let hplevel = cls.hplevel;
    // The experience the current level needs, with the class's and race's modifiers
    let need = new_calc_exp_needed(plr.level as i16, cls.exp.wrapping_add(rac.exp));
    // Enough experience
    let ok = plr.expbil > need.expbil || (plr.exprem >= need.exprem && plr.expbil == need.expbil);
    if !ok {
        return refuse(world, id, Msg::TrainNoExp);
    }
    // The fee
    let cost = training_fee_copper(plr.level, markup);
    let price = u32::try_from(cost).unwrap_or(u32::MAX);
    let Some(payment) = economy::pay(plr.coins, price) else {
        return refuse(world, id, Msg::TrainNoMoney);
    };
    let paid = coin_list(world, payment.paid);
    let lives_before = plr.lives;
    let new_level = plr.level.wrapping_add(1);
    let gains = train_gains(new_level, hplevel, lives_before, &mut world.rng);
    let title = class_title(world, plr.class, new_level, plr.sex);

    let Some(s) = world.session_mut(id) else {
        return false;
    };
    let Some(plr) = s.character.as_mut() else {
        return false;
    };
    // Paid, and the new level and title
    plr.coins = payment.left;
    plr.level = new_level;
    plr.title = title;
    // Character points (16-bit: above 32767 they show negative)
    plr.cp = plr.cp.wrapping_add(gains.cp as i16);
    plr.totalcp = plr.totalcp.wrapping_add(gains.cp);
    // Random hit points
    plr.randomhp = plr.randomhp.wrapping_add(gains.hp as u8);
    // Lives
    plr.lives = gains.lives;
    s.output.push(Msg::Trained {
        paid,
        level: i64::from(new_level),
    });
    s.output.push(Msg::TrainReceive);
    s.output.push(Msg::TrainCp {
        cp: i64::from(gains.cp),
    });
    if gains.lives != lives_before {
        s.output.push(Msg::TrainLives {
            lives: i64::from(gains.lives.wrapping_sub(lives_before)),
        });
    }
    // The secondary stats again in mode 2, then the title again.
    if let Some(mut c) = world.sessions.get(&id).and_then(|s| s.character.clone()) {
        recalculate(world, &mut c, CalcMode::KeepCurrent);
        c.title = class_title(world, c.class, c.level, c.sex);
        if let Some(s) = world.sessions.get_mut(&id) {
            s.character = Some(c);
        }
    }
    // A Kai class catches up on its powers
    if is_kai(world, id) {
        crate::spells::spellbook::update_kai_powers(world, id);
    }
    world.update_top_list(id);
    world.level_gained(id, new_level);
    true
}

/// A refusal: the line goes to the player.
fn refuse(world: &mut World, id: SessionId, msg: Msg) -> bool {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
    false
}

/// Singular for exactly one,
/// otherwise plural; the fifth currency is the sysop's `USRCURNM`.
fn currency_name(world: &World, amount: u32, cur: Currency) -> String {
    if cur == Currency::Runic {
        return world.config.usr_cur_name.clone();
    }
    let name = if amount == 1 {
        cur.singular_name()
    } else {
        cur.plural_name()
    };
    name.unwrap_or("").to_owned()
}

/// The coins a payment lists: ` N name` for each
/// type handed over, largest first, separated by commas, or ` nothing`.
fn coin_list(world: &World, paid: Coins) -> String {
    let parts = [
        (paid.runic, Currency::Runic),
        (paid.platinum, Currency::Platinum),
        (paid.gold, Currency::Gold),
        (paid.silver, Currency::Silver),
        (paid.copper, Currency::Copper),
    ];
    let listed: Vec<String> = parts
        .into_iter()
        .filter(|&(n, _)| n != 0)
        .map(|(n, cur)| format!(" {n} {}", currency_name(world, n, cur)))
        .collect();
    if listed.is_empty() {
        " nothing".to_string()
    } else {
        listed.join(",")
    }
}

/// Whether the character's class is a Kai class (magery Kai).
fn is_kai(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .and_then(|s| s.player())
        .and_then(|p| world.classes.get(&p.class))
        .is_some_and(|c| c.magery == silvermere_types::enums::Magery::Kai)
}
