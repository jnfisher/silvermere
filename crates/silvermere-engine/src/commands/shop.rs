//! Shops: LIST, BUY, SELL and APPRAISE and the work behind them, [`buy_item`] and [`sell_item`].
//! The LIST screen is `display::shop`; stock, restocking and the the shelf copies of limited items
//! are `shops`.
//!
//! STOCK, UNSTOCK and MARKUP (the gang shop owner's commands) are registered here; their
//! bodies are `commands::gang` (T-044).

use silvermere_rules::abilities::{item_has_ability, spell_has_ability};
use silvermere_rules::economy;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Currency, DebugLevel, ShopType};
use silvermere_types::flags::{GangFlags, PlayerFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ItemNo, RoomKey, SessionId, ShopNo, SpellNo};
use silvermere_types::sizes::{NINVITEMS, NSPELLCAST};

use super::dispatch::Handled;
use super::input::HOOKS;
use super::settings::coins_handed_over;
use super::{Cmd, CommandLine, eq_ignore_case, starts_with_ignore_case};
use crate::display::bank::proper_currency_name;
use crate::display::shop::display_shop_items;
use crate::items::inventory::{
    Search, add_item_to_inventory, find_item_in_inventory, remove_item_from_inventory,
    show_options, user_is_wearing,
};
use crate::items::limited::{self, Holder};
use crate::output::clear_output;
use crate::shops::{
    GANG_SLOTS, ShelfCopy, get_shop_item_from_name, persist_stock, return_shelf_copy, shop_room,
    take_shelf_copy,
};
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// A gang shop slot emptied by its last sale gets `regennum` 5, the "Undefined" currency.
const GANG_SLOT_CLEARED_CURRENCY: i16 = 5;

fn room_of(world: &World, id: SessionId) -> Option<RoomKey> {
    let plr = world.session(id)?.player()?;
    Some(RoomKey {
        map: plr.map,
        room: plr.room,
    })
}

/// Doing anything ends resting and meditating (LIST, BUY and SELL alike).
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.status.remove(PlayerStatus::RESTING);
        plr.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// Buying and selling give the player away.
fn stop_hiding(world: &mut World, id: SessionId) {
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.status.remove(PlayerStatus::SNEAKING);
        plr.hidden = 0;
    }
}

fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// A message to everyone else in the room, always heard, by the blind too, with a prompt
/// and no colour.
fn tell_others(world: &mut World, id: SessionId, room: RoomKey, msg: &Msg) {
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        msg,
    );
}

/// LIST: resting and meditating end first; LIST with an argument is not a command, so the
/// line goes on to the rest of the input pipeline; outside a shop room the player is told off; in
/// one, `display_shop_items` shows the goods (or a bank's rate card).
pub fn cmd_list(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() != 1 {
        return Handled::NotCommand;
    }
    match room_of(world, id).and_then(|r| shop_room(world, r)) {
        Some(shop) => display_shop_items(world, id, shop),
        None => tell_user(world, id, Msg::ShopNotList),
    }
    Handled::Done
}

/// STOCK: [`super::gang::cmd_stock`].
pub fn cmd_stock(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    super::gang::cmd_stock(world, id, cmd, line)
}

/// UNSTOCK: [`super::gang::cmd_unstock`].
pub fn cmd_unstock(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    super::gang::cmd_unstock(world, id, cmd, line)
}

/// MARKUP: [`super::gang::cmd_markup`].
pub fn cmd_markup(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    super::gang::cmd_markup(world, id, cmd, line)
}

/// The room's sysop command text (`perform_special_command` over its `cmdtext`), when the room has
/// one: `Some(Done)` when a line of it matched, `Some(NotCommand)` when none did, `None` when the
/// room has no command text.
fn room_command_text(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    line: &str,
) -> Option<Handled> {
    let cmdtext = world.room(room)?.cmdtext;
    if cmdtext.0 == 0 {
        return None;
    }
    Some(Handled::from_rc(HOOKS.room_text.perform(world, id, line)))
}

/// BUY. In a shop room the rest of the line is bought ([`buy_item`]) and then offered to
/// the room's command text, whose verdict is returned. Elsewhere BUY ROOM explains the gang house
/// lease, a room with a command text gets the line, and any other room refuses. A missing room is
/// a room with no text.
pub fn cmd_buy(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::ShopSyntaxBuy);
        return Handled::Done;
    }
    let Some(room) = room_of(world, id) else {
        return Handled::NotCommand;
    };
    if let Some(shop) = shop_room(world, room) {
        stop_hiding(world, id);
        buy_item(world, id, shop, line.rest_from(1));
        return room_command_text(world, id, room, &line.raw).unwrap_or(Handled::Done);
    }
    if eq_ignore_case(line.arg(1), "room") {
        tell_user(world, id, Msg::ShopBuyRoom);
        return Handled::Done;
    }
    if let Some(rc) = room_command_text(world, id, room, &line.raw) {
        return rc;
    }
    tell_user(world, id, Msg::ShopNotBuy);
    Handled::Done
}

/// SELL {item}: resting and meditating end; alone it shows the syntax; in a shop room
/// [`sell_item`], elsewhere a refusal.
pub fn cmd_sell(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::ShopSyntaxSell);
        return Handled::Done;
    }
    match room_of(world, id).and_then(|r| shop_room(world, r)) {
        Some(shop) => {
            stop_hiding(world, id);
            sell_item(world, id, shop, line.rest_from(1), false);
        }
        None => tell_user(world, id, Msg::ShopNotSell),
    }
    Handled::Done
}

/// APPRAISE: not a command without a word after it or in a room with no shop number (the room type
/// is not checked); otherwise [`sell_item`] with `appraise` set. Resting goes on.
pub fn cmd_appraise(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() || line.argc() < 2 {
        return Handled::NotCommand;
    }
    let Some(shop) = room_of(world, id)
        .and_then(|r| world.room(r))
        .map(|rm| rm.shopnum)
        .filter(|n| n.0 != 0)
    else {
        return Handled::NotCommand;
    };
    sell_item(world, id, shop, line.rest_from(1), true);
    Handled::Done
}

/// Why a purchase failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 0: not a known item.
    Unknown,
    /// 1: cannot buy it here (out of stock, no free limited copy).
    NotHere,
    /// 2: cannot afford it.
    Poor,
    /// 4: already owns a gang house.
    HouseOwner,
    /// 5: the gang has too little experience.
    GangExp,
    /// 6: not a gang leader.
    NotLeader,
    /// 7: sold a deed before.
    SoldDeed,
}

/// The slot a purchase goes ahead from (`slot`, `price` and `ok`).
struct Sale {
    slot: usize,
    price: u16,
    currency: Currency,
    copy: ShelfCopy,
}

/// Marks the shop dirty: queues the stock when a slot's quantity differs from `before`.
fn persist_if_changed(world: &mut World, before: &silvermere_types::records::Shop) {
    let changed = world.shops.get(&before.number).is_some_and(|now| {
        now.stock
            .iter()
            .zip(&before.stock)
            .any(|(a, b)| a.qty != b.qty)
    });
    if changed {
        persist_stock(world, before.number);
    }
}

/// The hospital's two services. `true` when `name` named one.
fn hospital(world: &mut World, id: SessionId, name: &str) -> bool {
    let currency = world.config.usr_cur_name.clone();
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return true;
    };
    if starts_with_ignore_case(name, "healing") {
        // 2 copper per missing hit point, in a 16-bit int (negative: never affordable)
        let fee = (i32::from(plr.maxhp) - i32::from(plr.hp)).wrapping_mul(2) as i16;
        let price = economy::convert_currency(0, 0, 0, 0, i32::from(fee) as u32);
        let Some(paid) = economy::pay(plr.coins, price) else {
            tell_user(world, id, Msg::ShopHealPoor);
            return true;
        };
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.coins = paid.left;
            plr.hp = plr.maxhp;
        }
        let coins = coins_handed_over(paid.paid, &currency);
        tell_user(world, id, Msg::ShopHandOverHealed { coins });
        return true;
    }
    if starts_with_ignore_case(name, "curing") || starts_with_ignore_case(name, "cure poison") {
        let poisoned = plr.poison != 0;
        // 25 silver nobles, or 15 when he was not poisoned after all
        let fee = if poisoned { 25 } else { 15 };
        let price = economy::convert_currency(0, 0, 0, fee, 0);
        let Some(paid) = economy::pay(plr.coins, price) else {
            tell_user(world, id, Msg::ShopCurePoor);
            return true;
        };
        let coins = coins_handed_over(paid.paid, &currency);
        // every spell on him with the Poison ability ends; a slot whose spell no longer exists is
        // just emptied
        let ending: Vec<usize> = if poisoned {
            (0..NSPELLCAST)
                .filter(|&j| {
                    let n = plr.spells[j];
                    n != SpellNo(0)
                        && world
                            .spells
                            .get(&n)
                            .is_none_or(|spl| spell_has_ability(spl, Ability::Poison))
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.coins = paid.left;
            if poisoned {
                plr.poison = 0;
                for j in ending {
                    plr.spells[j] = SpellNo(0);
                    plr.spellvals[j] = 0;
                    plr.spellrnds[j] = 0;
                    // TODO(T-049): the spell's proper termination for a spell that exists
                    // (its end text and effects).
                }
            }
        }
        let msg = if poisoned {
            Msg::ShopHandOverCured { coins }
        } else {
            Msg::ShopHandOverNotPoisoned { coins }
        };
        tell_user(world, id, msg);
        return true;
    }
    false
}

/// The deed shop's rules: only a gang's leader, with no deed in his pack, whose gang has `gangexp *
/// 10000` experience, and who has not sold a deed before.
fn deed_sale(
    world: &mut World,
    id: SessionId,
    shopnum: ShopNo,
    item: ItemNo,
    room: RoomKey,
) -> Result<Sale, Refusal> {
    let debug = debug_on(world, id);
    let Some(plr) = world.session(id).and_then(|s| s.player()).cloned() else {
        return Err(Refusal::Unknown);
    };
    let gang = world.gangs.get(&plr.gang.to_ascii_uppercase()).cloned();
    let Some(gang) = gang.filter(|g| eq_ignore_case(&g.leader, &plr.firstname)) else {
        return Err(Refusal::NotLeader);
    };
    if debug {
        let msg = Msg::ShopDebugLeader {
            leader: gang.leader.clone(),
            name: plr.firstname.clone(),
        };
        tell_user(world, id, msg);
    }
    let mut refusal = None;
    // already carrying a gang house deed?
    let owns_deed = plr.items.iter().take(NINVITEMS).any(|&n| {
        n != ItemNo(0)
            && world
                .items
                .get(&n)
                .is_some_and(|i| item_has_ability(i, Ability::GangDeed))
    });
    if owns_deed {
        refusal = Some(Refusal::HouseOwner);
    }
    // `gangexp` times 10000, a long compared unsigned with the gang's experience
    let needed = (i32::from(world.config.gang_exp as i16)).wrapping_mul(10_000);
    let gangxp = if gang.flags.contains(GangFlags::OVERFLOW) && !owns_deed {
        if debug {
            tell_user(world, id, Msg::ShopDebugOverflow);
        }
        needed as u32
    } else {
        if debug {
            let msg = Msg::ShopDebugGangExp {
                exp: gang.exp as u16,
                base: i32::from(world.config.gang_exp as i16),
                req: needed,
            };
            tell_user(world, id, msg);
        }
        gang.exp
    };
    let mut sale = None;
    // (sic) a leader who already has a deed is refused as if his gang lacked experience
    if needed as u32 <= gangxp && !owns_deed {
        let Some(stock) = world.shops.get(&shopnum).map(|s| s.stock) else {
            return Err(Refusal::Unknown);
        };
        let mut taken: Option<Option<ShelfCopy>> = None;
        for (i, s) in stock.iter().enumerate() {
            if s.item != item || s.qty == 0 {
                continue;
            }
            // one shelf copy for the purchase, whichever slot it comes from
            let copy = *taken.get_or_insert_with(|| take_shelf_copy(world, item, room));
            if let Some(copy) = copy {
                let (price, currency) = world
                    .items
                    .get(&item)
                    .map_or((0, Currency::Copper), |i| (i.cost, i.currency));
                sale = Some(Sale {
                    slot: i,
                    price,
                    currency,
                    copy,
                });
            } else {
                // no free copy: one less in stock
                refusal = Some(Refusal::NotHere);
                sale = None;
                if let Some(shp) = world.shops.get_mut(&shopnum) {
                    shp.stock[i].qty -= 1;
                }
            }
        }
    } else {
        refusal = Some(Refusal::GangExp);
    }
    // a player who sold a deed may not buy another
    if plr.flags.contains(PlayerFlags::SOLDDEED) {
        if let Some(s) = sale {
            return_shelf_copy(world, item, room, s.copy);
        }
        return Err(Refusal::SoldDeed);
    }
    sale.ok_or(refusal.unwrap_or(Refusal::NotHere))
}

/// The slot an ordinary or gang shop sells `item` from. Every listing slot is looked at and the
/// last one with stock is the one sold from; a limited item with no free copy refuses (reason 1)
/// and, outside a gang shop, takes one copy off that slot's stock.
fn shelf_sale(
    world: &mut World,
    shopnum: ShopNo,
    item: ItemNo,
    room: RoomKey,
) -> Result<Sale, Refusal> {
    let Some(shp) = world.shops.get(&shopnum) else {
        return Err(Refusal::Unknown);
    };
    let gang = shp.shop_type == ShopType::GangShop;
    let stock = shp.stock;
    let (cost, currency) = world
        .items
        .get(&item)
        .map_or((0, Currency::Copper), |i| (i.cost, i.currency));
    let slots = if gang { GANG_SLOTS } else { stock.len() };
    let mut taken: Option<Option<ShelfCopy>> = None;
    let mut sale: Option<Sale> = None;
    for (i, s) in stock.iter().enumerate().take(slots) {
        if s.item != item || s.qty == 0 {
            // (sic) the deed-slot repair under `qty > 1` with qty 0 never runs
            continue;
        }
        // one shelf copy for the purchase, whichever slot it is sold from
        let Some(copy) = *taken.get_or_insert_with(|| take_shelf_copy(world, item, room)) else {
            sale = None;
            if !gang && let Some(shp) = world.shops.get_mut(&shopnum) {
                // no free copy: one less in stock
                shp.stock[i].qty -= 1;
            }
            // (sic) in a gang shop qty is above 0 here, so the slot is never cleared
            continue;
        };
        sale = Some(if gang {
            Sale {
                slot: i,
                price: s.regentime,
                currency: Currency::from_code(s.regennum as u16),
                copy,
            }
        } else {
            Sale {
                slot: i,
                price: cost,
                currency,
                copy,
            }
        });
    }
    sale.ok_or(Refusal::NotHere)
}

/// The BUY of `name` in shop `shopnum`.
///
/// A hospital sells `healing` and `curing`, a training hall `training` (`train_level`).
/// Anything else is looked for among the shop's goods: several names give the
/// "Please be more specific" list; none, `... is not a known item.`; then the slot with
/// stock, the price in copper (`economy::buy_price`: markup, then Charm), the purse, and
/// the item into the pack. The room sees the purchase, the buyer the coins handed over,
/// and the slot loses one copy (a gang shop slot sold out is cleared).
pub fn buy_item(world: &mut World, id: SessionId, shopnum: ShopNo, name: &str) {
    let Some(shop) = world.shops.get(&shopnum).cloned() else {
        return;
    };
    let Some(plr) = world.session(id).and_then(|s| s.player()).cloned() else {
        return;
    };
    // the class record is only checked for existence
    if !world.classes.contains_key(&plr.class) {
        return;
    }
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    if shop.shop_type == ShopType::Hospital {
        if hospital(world, id, name) {
            return;
        }
    } else if shop.shop_type == ShopType::Training && starts_with_ignore_case(name, "training") {
        crate::progression::train::train_level(world, id);
        return;
    }
    let found = get_shop_item_from_name(world, &shop, name);
    if found.cleared {
        clear_output(world, id);
    }
    let Some(item) = found.item else {
        if found.count > 0 {
            // the list was sent
            show_options(world, id, &found.options);
            return;
        }
        tell_user(world, id, Msg::ShopUnknown { item: name.into() });
        return;
    };
    let itm_name = world
        .items
        .get(&item)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    let result = if shop.shop_type == ShopType::DeedShop {
        deed_sale(world, id, shopnum, item, room)
    } else {
        shelf_sale(world, shopnum, item, room)
    };
    // the price in copper, then the purse
    let result = result.and_then(|sale| {
        let cost = economy::buy_price(sale.price, sale.currency, shop.markup, plr.cha) as u32;
        if economy::can_pay(plr.coins, cost) {
            Ok((sale, cost))
        } else {
            // cannot pay: the shelf copy goes back
            return_shelf_copy(world, item, room, sale.copy);
            Err(Refusal::Poor)
        }
    });
    let (sale, cost) = match result {
        Ok(ok) => ok,
        Err(refusal) => {
            let msg = match refusal {
                Refusal::Unknown => Msg::ShopUnknown { item: name.into() },
                Refusal::NotHere => Msg::ShopCannotBuy { item: itm_name },
                Refusal::Poor => Msg::ShopCannotAfford { item: itm_name },
                Refusal::HouseOwner => Msg::ShopHouseOwner,
                Refusal::GangExp => Msg::ShopHouseExp,
                Refusal::NotLeader => Msg::ShopHouseLeader,
                Refusal::SoldDeed => Msg::ShopHousePaperwork,
            };
            tell_user(world, id, msg);
            persist_if_changed(world, &shop);
            return;
        }
    };
    // hand the item over; it may not fit (a limited item is recorded for the buyer there)
    if !add_item_to_inventory(world, id, item, -2) {
        // the shelf copy goes back
        return_shelf_copy(world, item, room, sale.copy);
        return;
    }
    // the room (with what the prf buffer held), then the buyer, then the money
    clear_output(world, id);
    let who = plr.firstname.clone();
    tell_others(
        world,
        id,
        room,
        &Msg::ShopSeeBuy {
            who,
            item: itm_name.clone(),
        },
    );
    let currency = world.config.usr_cur_name.clone();
    let paid = world
        .session_mut(id)
        .and_then(|s| s.character.as_mut())
        .and_then(|plr| {
            let paid = economy::pay(plr.coins, cost)?;
            plr.coins = paid.left;
            Some(paid.paid)
        })
        .unwrap_or_default();
    tell_user(
        world,
        id,
        Msg::ShopBought {
            item: itm_name,
            coins: coins_handed_over(paid, &currency),
        },
    );
    // one less in stock; a sold-out gang shop slot is cleared
    let mut cleared = false;
    if let Some(shp) = world.shops.get_mut(&shopnum) {
        let s = &mut shp.stock[sale.slot];
        s.qty = s.qty.wrapping_sub(1);
        if shp.shop_type == ShopType::GangShop && s.qty == 0 {
            s.item = ItemNo(0);
            s.regentime = 0;
            s.regennum = GANG_SLOT_CLEARED_CURRENCY;
            cleared = true;
        }
    }
    if cleared && debug_on(world, id) {
        tell_user(world, id, Msg::ShopDebugRemove);
    }
    // a gang shop pays its owner, whose user-id lives in the slots' `maxqty`; its record changes as
    // a whole when a slot is cleared
    if world
        .shops
        .get(&shopnum)
        .is_some_and(|s| s.shop_type == ShopType::GangShop)
    {
        crate::gangs::deposit_gangleaders_account(world, id, shopnum, cost);
        if cleared {
            crate::gangs::save_gang_shop(world, shopnum);
            return;
        }
    }
    persist_stock(world, shopnum);
}

/// SELL (`appraise` false) or APPRAISE of a carried item at shop `shopnum`.
///
/// A gang shop buys nothing. The item is found in the pack (several names give the list);
/// a cursed item being worn sells only when a second copy is carried. Every slot that
/// lists the item and is below its ceiling takes the sold copy back on the shelf (a deed
/// shop instead marks the seller `PF_SOLDDEED`, on APPRAISE too, sic). The price is
/// `economy::sell_price` (Charm), always paid in copper and then changed up into the
/// fewest coins. A limited copy back on the shelf is recorded there (an `OWN_ROOM` record
/// of the shop's room marked as the shelf); when every allowed copy is in play the shelf
/// gives it up again.
pub fn sell_item(world: &mut World, id: SessionId, shopnum: ShopNo, name: &str, appraise: bool) {
    let Some(shop) = world.shops.get(&shopnum).cloned() else {
        return;
    };
    let Some(plr) = world.session(id).and_then(|s| s.player()).cloned() else {
        return;
    };
    if shop.shop_type == ShopType::GangShop {
        tell_user(world, id, Msg::ShopGangNoSell);
        return;
    }
    let found = find_item_in_inventory(world, &plr, name, Search::Any);
    if found.ambiguous() {
        // several matches: the list goes out
        show_options(world, id, &found.options);
        return;
    }
    // the match lookup ends: the prf buffer is emptied
    clear_output(world, id);
    let Some(item) = found.item else {
        tell_user(world, id, Msg::ShopDontHave { item: name.into() });
        return;
    };
    let Some(itm) = world.items.get(&item).cloned() else {
        return;
    };
    // a worn cursed item needs a second copy
    let cursed =
        item_has_ability(&itm, Ability::Cursed) || item_has_ability(&itm, Ability::MajorCurse);
    if cursed && user_is_wearing(&plr, item) {
        let copies = plr
            .items
            .iter()
            .take(NINVITEMS)
            .filter(|&&n| n == item)
            .count();
        if copies < 2 {
            tell_user(world, id, Msg::ShopMayNotSell);
            return;
        }
    }
    // the shop's slots for this item
    let mut restocked = false;
    let mut listed = false;
    let mut stock_changed = false;
    let deed = shop.shop_type == ShopType::DeedShop;
    if let Some(shp) = world.shops.get_mut(&shopnum) {
        for s in shp.stock.iter_mut().filter(|s| s.item == item) {
            if s.qty < s.maxqty {
                if !appraise && !deed {
                    restocked = true;
                    s.qty += 1;
                    stock_changed = true;
                } else if deed {
                    restocked = true;
                }
            }
            listed = true;
        }
    }
    if deed
        && restocked
        && let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut())
    {
        plr.flags.insert(PlayerFlags::SOLDDEED);
    }
    if !listed {
        tell_user(world, id, Msg::ShopCannotSell { item: itm.name });
        return;
    }
    let price = economy::sell_price(itm.cost, itm.currency, plr.cha) as u32;
    let coin = proper_currency_name(price, 0).to_string();
    if appraise {
        let msg = Msg::ShopAppraise {
            n: price,
            coin,
            item: itm.name,
        };
        tell_user(world, id, msg);
        return;
    }
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.coins.copper = plr.coins.copper.wrapping_add(price);
    }
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let see = Msg::ShopSeeSell {
        who: plr.firstname.clone(),
        item: itm.name.clone(),
    };
    tell_others(world, id, room, &see);
    let sold = Msg::ShopSold {
        item: itm.name.clone(),
        n: price,
        coin,
    };
    tell_user(world, id, sold);
    // the purse's coins are normalised
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.coins = economy::normalise(plr.coins);
    }
    // the seller's record of a limited copy leaves with it
    remove_item_from_inventory(world, id, item);
    // a restocked limited copy is recorded on the shelf; when every allowed copy is already in play
    // it cannot be, and every slot holding one gives one up
    if restocked
        && !limited::acquire(world, item, Holder::Shelf(room))
        && let Some(shp) = world.shops.get_mut(&shopnum)
    {
        for s in shp.stock.iter_mut().filter(|s| s.item == item && s.qty > 0) {
            s.qty -= 1;
            stock_changed = true;
        }
    }
    if stock_changed {
        persist_stock(world, shopnum);
    }
}
