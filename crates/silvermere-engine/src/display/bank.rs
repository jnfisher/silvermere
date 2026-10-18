//! The bank balance screens (T-042): [`display_bank_balance`], [`display_users_bankbooks`]
//! and [`format_number_with_commas`].

use silvermere_text::Msg;
use silvermere_types::enums::ShopType;
use silvermere_types::ids::{SessionId, ShopNo};
use silvermere_types::names::{CURRENCY_NAMES, CURRENCY_NAMES_SINGULAR};

use crate::commands::bank::BankBooks;
use crate::world::World;

/// `1234567` as `1,234,567`.
pub fn format_number_with_commas(number: u32) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The singular for exactly one coin, otherwise the plural (also for none). `cur` is the
/// `CUR_*` index (copper 0, gold 2).
pub fn proper_currency_name(amount: u32, cur: usize) -> &'static str {
    if amount == 1 {
        CURRENCY_NAMES_SINGULAR[cur]
    } else {
        CURRENCY_NAMES[cur]
    }
}

/// The account at one bank, two lines. Nothing is shown when the shop does not exist or is
/// not a bank; a player with no bankbook there has one made on the spot (reading a bankbook
/// never fails), so the screen shows a balance of 0. `maximum_bank_balance` caps a balance
/// on the way in.
pub fn display_bank_balance(world: &mut World, id: SessionId, shopnum: Option<ShopNo>) {
    let Some(account) = world
        .session(id)
        .and_then(|s| s.account.as_ref())
        .map(|a| a.id)
    else {
        return;
    };
    let Some(shopnum) = shopnum else { return };
    let Some(shop) = world.shops.get(&shopnum) else {
        return;
    };
    if shop.shop_type != ShopType::Bank {
        return;
    }
    let bank = shop.name.clone();
    let cap = world.config.maximum_bank_balance;
    let balance = world.bank_books.balance_at(account, shopnum, cap);
    let msg = Msg::BankBalance {
        bank,
        num: i64::from(shopnum.0),
        copper: balance,
        copper_name: proper_currency_name(balance, 0).to_string(),
        gold: format!(
            "{}.{:02}",
            format_number_with_commas(balance / 100),
            balance % 100
        ),
        gold_name: proper_currency_name(balance, 2).to_string(),
    };
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// The `SHOP_BANK` case of the shop list, for LIST in a bank.
pub fn display_bank_services(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::BankServices);
    }
}

/// The balance screen of each bankbook on file, in bank order, at most `MAXBOOKS` (30). A
/// book exists once a deposit has made it (`dirty`), so a player who never banked sees
/// nothing.
pub fn display_users_bankbooks(world: &mut World, id: SessionId) {
    let Some(account) = world
        .session(id)
        .and_then(|s| s.account.as_ref())
        .map(|a| a.id)
    else {
        return;
    };
    let banks: Vec<ShopNo> = world
        .bank_books
        .for_account(account)
        .iter()
        .take(MAXBOOKS)
        .map(|b| b.bank)
        .collect();
    for bank in banks {
        display_bank_balance(world, id, Some(bank));
    }
}

/// The most bankbooks listed.
pub const MAXBOOKS: usize = 30;

#[cfg(test)]
mod tests {
    use super::*;

    // "1234567" -> "1,234,567"
    #[test]
    fn commas() {
        assert_eq!(format_number_with_commas(0), "0");
        assert_eq!(format_number_with_commas(999), "999");
        assert_eq!(format_number_with_commas(1000), "1,000");
        assert_eq!(format_number_with_commas(1_234_567), "1,234,567");
        assert_eq!(format_number_with_commas(u32::MAX), "4,294,967,295");
    }

    #[test]
    fn currency_names() {
        assert_eq!(proper_currency_name(1, 0), "copper farthing");
        assert_eq!(proper_currency_name(0, 0), "copper farthings");
        assert_eq!(proper_currency_name(523, 2), "gold crowns");
    }
}
