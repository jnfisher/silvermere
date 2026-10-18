//! The LIST screen.
//!
//! Gang shops (their first ten slots, priced at the gang's asking price) and every other
//! shop (all twenty slots at the item's cost) show the same heading, the same row format
//! and the same ending, so one loop over [`SlotKind`] renders both.

use silvermere_rules::spells::{CannotUse, LegalBand, can_use_spell};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Currency, DebugLevel, ItemType, ShopType};
use silvermere_types::ids::{ItemNo, SessionId, ShopNo, SpellNo};
use silvermere_types::names::CURRENCY_NAMES;
use silvermere_types::records::{Character, Item};
use silvermere_types::sizes::NSHOPITEMS;

use crate::items::can_use::{UseCheck, can_use};
use crate::items::inventory::has_ability;
use crate::shops::{GANG_SLOTS, SlotKind, slot_kind};
use crate::tell::tell_user;
use crate::world::World;

/// How a row ends: whether the looker can use the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usable {
    /// `user_can_use`: the row just ends.
    Yes,
    /// Not usable and the too-powerful flag set: ` (Too powerful)`.
    TooPowerful,
    /// Not usable: ` (You can't use)`.
    No,
}

impl Usable {
    fn flag(self) -> &'static str {
        match self {
            Self::Yes => "",
            Self::TooPowerful => " (Too powerful)",
            Self::No => " (You can't use)",
        }
    }
}

/// `user_can_use` and the too-powerful flag it leaves behind. The flag is cleared on entry
/// and set only by `user_can_use_spell` for a scroll whose linked spell the class could
/// cast but whose level is above the character's: an item's own `level` field is never
/// read, and its "Minimum level" ability refuses with ` (You can't use)`.
pub fn usable(world: &World, plr: &Character, itm: &Item) -> Usable {
    let spells = |n| world.spells.get(&n);
    let class = world.classes.get(&plr.class);
    let check = UseCheck {
        class,
        anti_magic: has_ability(world, plr, Ability::AntiMagic),
        spells: &spells,
    };
    if can_use(plr, itm, &check).is_ok() {
        return Usable::Yes;
    }
    let too_powerful = itm.item_type == ItemType::Scroll
        && class.is_some_and(|cls| {
            itm.abilities.iter().any(|&(a, v)| {
                a == Ability::LinkSpell
                    && world.spells.get(&SpellNo(v)).is_some_and(|spl| {
                        can_use_spell(spl, cls, plr.level, LegalBand::of_evil(plr.evil))
                            == Err(CannotUse::TooPowerful)
                    })
            })
        });
    if too_powerful {
        Usable::TooPowerful
    } else {
        Usable::No
    }
}

/// The currency's name, with the sysop's name for the runic coin.
fn currency_name(world: &World, currency: u16) -> String {
    if currency == Currency::Runic.code() {
        return world.config.usr_cur_name.clone();
    }
    CURRENCY_NAMES
        .get(usize::from(currency))
        .map_or_else(|| "(null)".to_string(), |s| (*s).to_string())
}

/// One row (`%-29.29s %-8d %5d %-8s` or `%-29.29s %-8d     Free`, then the ending).
/// `price` is `None` for Free.
pub fn shop_row(name: &str, qty: u16, price: Option<(String, String)>, usable: Usable) -> Msg {
    let name: String = name.chars().take(29).collect();
    let price = match price {
        Some((amount, cur)) => format!("{amount:>5} {cur:<8}"),
        None => "    Free".to_string(),
    };
    Msg::ShopRow {
        name: format!("{name:<29}"),
        // `%-8d` of the unsigned quantity: a 16-bit int
        qty: format!("{:<8}", qty as i16),
        price,
        flag: usable.flag().to_string(),
    }
}

/// A bank's rate card, or the goods in stock with the heading before the first one. A shop
/// with nothing in stock (a hospital, a training hall) shows nothing at all.
pub fn display_shop_items(world: &mut World, id: SessionId, shopnum: ShopNo) {
    let plr = world.session(id).and_then(|s| s.player()).cloned();
    let (Some(shop), Some(plr)) = (world.shops.get(&shopnum).cloned(), plr) else {
        // An internal error: nothing is shown
        tracing::error!(shop = shopnum.0, "display_shop_items: no shop or no player");
        return;
    };
    if shop.shop_type == ShopType::Bank {
        crate::display::bank::display_bank_services(world, id);
        return;
    }
    let debug = world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off);
    let gang = shop.shop_type == ShopType::GangShop;
    let slots = if gang { GANG_SLOTS } else { NSHOPITEMS };
    let mut shown = false;
    for i in 0..slots {
        let (item, qty, price) = match slot_kind(&shop, i) {
            SlotKind::Stock(s) => (s.item, s.qty, None),
            SlotKind::GangListing {
                item,
                qty,
                price,
                currency,
            } => (item, qty, Some((price, currency))),
        };
        if item == ItemNo(0) || qty == 0 {
            continue;
        }
        let Some(itm) = world.items.get(&item).cloned() else {
            continue;
        };
        if !shown {
            tell_user(world, id, Msg::ShopListIntro);
            tell_user(world, id, Msg::ShopListHeader);
            tell_user(world, id, Msg::ShopListRule);
            shown = true;
        }
        let price = match price {
            // the gang's asking price, plus the markup, cut to 16 bits unsigned
            Some((cost, currency)) => {
                let code = currency.code();
                if debug {
                    tell_user(
                        world,
                        id,
                        Msg::ShopDebugGangCost {
                            slot: i as u8,
                            cost,
                        },
                    );
                    if cost == 0 {
                        tell_user(world, id, Msg::ShopDebugNoCost);
                    }
                    tell_user(
                        world,
                        id,
                        Msg::ShopDebugGangCurrency {
                            slot: i as u8,
                            index: code,
                            name: format!("{:<8}", currency_name(world, code)),
                        },
                    );
                    if code == 0 {
                        tell_user(world, id, Msg::ShopDebugNoCurrency);
                    }
                }
                (cost != 0).then(|| {
                    let amount = u32::from(cost) * (u32::from(shop.markup) + 100) / 100;
                    ((amount as u16).to_string(), currency_name(world, code))
                })
            }
            // the item's cost plus the markup, cut to 16 bits signed
            None => (itm.cost != 0).then(|| {
                let amount = u32::from(itm.cost) * (u32::from(shop.markup) + 100) / 100;
                (
                    (amount as i16).to_string(),
                    currency_name(world, itm.currency.code()),
                )
            }),
        };
        let usable = usable(world, &plr, &itm);
        tell_user(world, id, shop_row(&itm.name, qty, price, usable));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silvermere_text::colour::Palette;
    use silvermere_text::render::render;
    use silvermere_text::strip_ansi_str;

    fn plain(msg: &Msg) -> String {
        let bytes = render(msg, Palette::default()).bytes;
        strip_ansi_str(&String::from_utf8_lossy(&bytes))
    }

    // The rows of the Newhaven captures (T-005 `shop`, Thornwyn)
    #[test]
    fn rows_have_the_original_widths() {
        let free = shop_row("quarterstaff", 30, None, Usable::Yes);
        assert_eq!(
            plain(&free),
            "quarterstaff                  30           Free\r\n"
        );
        let priced = shop_row(
            "lantern",
            63,
            Some(("2".into(), "gold crowns".into())),
            Usable::Yes,
        );
        assert_eq!(
            plain(&priced),
            "lantern                       63           2 gold crowns\r\n"
        );
        let mend = shop_row(
            "scroll of mend",
            80,
            Some(("8".into(), "gold crowns".into())),
            Usable::TooPowerful,
        );
        assert_eq!(
            plain(&mend),
            "scroll of mend                80           8 gold crowns (Too powerful)\r\n"
        );
        let blur = shop_row("scroll of blur", 105, None, Usable::No);
        assert_eq!(
            plain(&blur),
            "scroll of blur                105          Free (You can't use)\r\n"
        );
    }

    // `%-29.29s` cuts a long name; `%-8s` pads a short currency name
    #[test]
    fn long_names_are_cut_and_short_currencies_padded() {
        let long = shop_row(
            "an extraordinarily long item name indeed",
            1,
            Some(("12345".into(), "rune".into())),
            Usable::Yes,
        );
        assert_eq!(
            plain(&long),
            "an extraordinarily long item  1        12345 rune    \r\n"
        );
    }
}
