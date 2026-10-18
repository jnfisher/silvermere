//! Economy: currency conversion and change, paying, shop buy and sell prices with Charm
//! and markup, carrying capacity, coin weight and encumbrance.
//!
//! The game's integer widths are kept at each step: 32-bit unsigned is `u32` and wraps,
//! 32-bit signed is `i32`, the rest are 16-bit.

use silvermere_types::enums::Currency;
use silvermere_types::records::Coins;

/// Platinum pieces per runic coin.
pub const PLATPERRUNIC: u32 = 100;
/// Gold crowns per platinum piece.
pub const GOLDPERPLAT: u32 = 100;
/// Silver nobles per gold crown.
pub const SILVPERGOLD: u32 = 10;
/// Copper farthings per silver noble.
pub const COPPERPERSILV: u32 = 10;

/// Copper in one gold crown.
const CPGOLD: u32 = SILVPERGOLD * COPPERPERSILV;
/// Copper in one platinum piece.
const CPPLAT: u32 = CPGOLD * GOLDPERPLAT;
/// Copper in one runic coin.
const CPRUNIC: u32 = CPPLAT * PLATPERRUNIC;

/// Most coins one step of changing coins up ([`normalise`]) converts.
const CLEANUP_STEP_LIMIT: u32 = 5000;

/// The value of an amount in copper.
///
/// Coins are carried down one type at a time in 32-bit unsigned arithmetic. The game's
/// guards are reproduced exactly, not as saturation: a product is replaced by the coin count
/// only when the wrapped product came out *smaller* than it (a wrap that lands higher is
/// kept), and a sum that would wrap is skipped.
pub fn convert_currency(runic: u32, platinum: u32, gold: u32, silver: u32, copper: u32) -> u32 {
    // One step: a wrapped product smaller than the count is the count; a wrapping sum is
    // skipped.
    fn carry_down(from: u32, rate: u32, to: u32) -> u32 {
        let mut tmp = from.wrapping_mul(rate);
        if tmp < from {
            tmp = from;
        }
        match to.checked_add(tmp) {
            Some(sum) => sum,
            None => to,
        }
    }
    let platinum = carry_down(runic, PLATPERRUNIC, platinum);
    let gold = carry_down(platinum, GOLDPERPLAT, gold);
    let silver = carry_down(gold, SILVPERGOLD, silver);
    carry_down(silver, COPPERPERSILV, copper)
}

/// The value of `coins` in copper: the whole purse through [`convert_currency`].
pub fn to_copper(coins: Coins) -> u32 {
    convert_currency(
        coins.runic,
        coins.platinum,
        coins.gold,
        coins.silver,
        coins.copper,
    )
}

/// An amount of one currency in copper: what buying and selling do with an item's price,
/// passing it as the amount of its own currency and 0 for the others. `Undefined` is worth
/// nothing.
pub fn amount_to_copper(amount: u32, currency: Currency) -> u32 {
    let pick = |c: Currency| if currency == c { amount } else { 0 };
    convert_currency(
        pick(Currency::Runic),
        pick(Currency::Platinum),
        pick(Currency::Gold),
        pick(Currency::Silver),
        pick(Currency::Copper),
    )
}

/// Change small coins up into larger ones. Copper goes into runic, platinum, gold and
/// silver (largest first), then silver into gold, gold into platinum and platinum into
/// runic. Each of the seven steps converts at most 5,000 coins, as in the game.
pub fn normalise(coins: Coins) -> Coins {
    // Up to 5,000 conversions of `from` units into one `to` coin.
    fn step(from: &mut u32, to: &mut u32, rate: u32) {
        let n = (*from / rate).min(CLEANUP_STEP_LIMIT);
        *from -= n * rate;
        *to = to.wrapping_add(n);
    }
    let mut c = coins;
    step(&mut c.copper, &mut c.runic, CPRUNIC);
    step(&mut c.copper, &mut c.platinum, CPPLAT);
    step(&mut c.copper, &mut c.gold, CPGOLD);
    step(&mut c.copper, &mut c.silver, COPPERPERSILV);
    step(&mut c.silver, &mut c.gold, SILVPERGOLD);
    step(&mut c.gold, &mut c.platinum, GOLDPERPLAT);
    step(&mut c.platinum, &mut c.runic, PLATPERRUNIC);
    c
}

/// `copper` farthings changed up into the fewest coins: [`normalise`] of a purse holding
/// only copper (the change after a sale).
pub fn from_copper(copper: u32) -> Coins {
    normalise(Coins {
        copper,
        ..Coins::default()
    })
}

/// Can a purse pay `price` copper?
pub fn can_pay(coins: Coins, price: u32) -> bool {
    price <= to_copper(coins)
}

/// The coins handed over and the purse left after paying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payment {
    /// The coins handed over (what the game lists after "You hand over").
    pub paid: Coins,
    /// The purse afterwards, including the change of any coin broken.
    pub left: Coins,
}

/// Pay `price` copper from `coins`.
///
/// The price is paid by value with whole coins, the largest first, as long as a coin still
/// fits into what is owed. When something is still owed but no coin left fits, one coin is
/// broken into the next smaller type (a silver noble into copper, else a gold crown into
/// silver, a platinum piece into gold, a runic coin into platinum) and paying goes on.
/// `None` when the purse cannot pay ([`can_pay`]); then nothing is taken.
pub fn pay(coins: Coins, price: u32) -> Option<Payment> {
    if !can_pay(coins, price) {
        return None;
    }
    // Pay up to `have` coins worth `unit` each, while one still fits.
    fn take(amount: &mut u32, have: &mut u32, paid: &mut u32, unit: u32) {
        let n = (*amount / unit).min(*have);
        *amount -= n * unit;
        *have -= n;
        *paid += n;
    }
    let mut left = coins;
    let mut paid = Coins::default();
    let mut amount = price;
    loop {
        // Whole coins, the largest first.
        take(&mut amount, &mut left.runic, &mut paid.runic, CPRUNIC);
        take(&mut amount, &mut left.platinum, &mut paid.platinum, CPPLAT);
        take(&mut amount, &mut left.gold, &mut paid.gold, CPGOLD);
        take(
            &mut amount,
            &mut left.silver,
            &mut paid.silver,
            COPPERPERSILV,
        );
        take(&mut amount, &mut left.copper, &mut paid.copper, 1);
        if amount == 0 {
            break;
        }
        // Break the smallest coin there is into the next type down.
        if left.silver > 0 {
            left.copper = left.copper.wrapping_add(COPPERPERSILV);
            left.silver -= 1;
        } else if left.gold > 0 {
            left.silver = left.silver.wrapping_add(SILVPERGOLD);
            left.gold -= 1;
        } else if left.platinum > 0 {
            left.gold = left.gold.wrapping_add(GOLDPERPLAT);
            left.platinum -= 1;
        } else if left.runic > 0 {
            left.platinum = left.platinum.wrapping_add(PLATPERRUNIC);
            left.runic -= 1;
        } else {
            // Nothing left at all: give up rather than loop.
            break;
        }
    }
    Some(Payment { paid, left })
}

/// The price in copper of an item costing `cost` in `currency` at a shop with `markup`
/// percent, for a buyer with Charm `cha`:
///
/// `cost * (markup+100)/100 * (110 - cha/5)/100`, each `/100` on its own, in unsigned
/// 32-bit arithmetic (the charm term too). Prices over 100,000 copper are worked out in
/// hundreds of copper and multiplied back, which drops the low two digits. For a gang shop
/// pass the slot's currency (`regennum`).
pub fn buy_price(cost: u16, currency: Currency, markup: u16, cha: u16) -> u64 {
    let mut copper = amount_to_copper(u32::from(cost), currency);
    let scaled = copper > 100_000;
    if scaled {
        copper /= 100;
    }
    // Markup, then Charm, each divided by 100 on its own.
    let copper = copper.wrapping_mul(u32::from(markup) + 100) / 100;
    let copper = copper.wrapping_mul(110u32.wrapping_sub(u32::from(cha) / 5)) / 100;
    let copper = if scaled {
        copper.wrapping_mul(100)
    } else {
        copper
    };
    u64::from(copper)
}

/// What a shop pays (or offers on APPRAISE) in copper for an item costing `cost` in
/// `currency`, to a seller with Charm `cha`: the cost in copper times `(cha/2 + 25)`
/// percent, unsigned 32-bit.
pub fn sell_price(cost: u16, currency: Currency, cha: u16) -> u64 {
    let copper = amount_to_copper(u32::from(cost), currency);
    u64::from(copper.wrapping_mul(u32::from(cha) / 2 + 25) / 100)
}

/// Carrying capacity from Strength, worked out with the secondary stats: 48 per point, and
/// 36 more per point above 100, in a signed 16-bit value.
pub fn carrying_capacity(str: u16) -> i16 {
    // unsigned 16-bit product, kept signed
    let mut maxwgt = str.wrapping_mul(48) as i16;
    if str > 100 {
        // signed 16-bit
        maxwgt = maxwgt.wrapping_add(((str - 100) as i16).wrapping_mul(36));
    }
    maxwgt
}

/// The most weight a character with Strength `str` can carry, raised or lowered by his
/// "Alter Encumbrance" ability (`alter_encumbrance_pct`, percent). The carrying capacity is
/// read zero-extended; never 0 (1 then), because callers divide by it.
pub fn max_weight(str: u16, alter_encumbrance_pct: i32) -> i32 {
    // zero-extended
    let mut maxw = i32::from(carrying_capacity(str) as u16);
    if alter_encumbrance_pct != 0 {
        // in 32 bits
        maxw = maxw.wrapping_mul(alter_encumbrance_pct.wrapping_add(100)) / 100;
    }
    if maxw != 0 { maxw } else { 1 }
}

/// Three coins of a type weigh one unit, each type counted on its own (so up to two coins
/// of each type weigh nothing).
pub fn coin_weight(coins: Coins) -> i32 {
    // The sum is 32-bit unsigned, kept signed.
    (coins.runic / 3)
        .wrapping_add(coins.platinum / 3)
        .wrapping_add(coins.gold / 3)
        .wrapping_add(coins.silver / 3)
        .wrapping_add(coins.copper / 3) as i32
}

/// The weight `carried` plus the coins' weight, as a percentage of `max` ([`max_weight`]),
/// truncated, in a signed 16-bit value. A `max` of 0 is taken as 1, as [`max_weight`]
/// guarantees.
pub fn encumbrance_pct(carried: u16, coins: Coins, max: i32) -> i16 {
    let load = i32::from(carried).wrapping_add(coin_weight(coins));
    let max = if max == 0 { 1 } else { max };
    load.wrapping_mul(100).wrapping_div(max) as i16
}

/// The word the inventory shows for an encumbrance percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncumbranceLevel {
    /// 16% or less ("None", grey).
    None,
    /// Over 16% ("Light", green).
    Light,
    /// Over 33% ("Medium", brown).
    Medium,
    /// Over 66% ("Heavy", red).
    Heavy,
}

impl EncumbranceLevel {
    /// The level for a percentage from [`encumbrance_pct`].
    pub fn from_pct(pct: i16) -> Self {
        if pct > 66 {
            Self::Heavy
        } else if pct > 33 {
            Self::Medium
        } else if pct > 16 {
            Self::Light
        } else {
            Self::None
        }
    }

    /// The word shown: "None", "Light", "Medium" or "Heavy".
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Heavy => "Heavy",
        }
    }
}
