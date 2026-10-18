//! Layer 1 tests for `economy`: each test names the rule it checks.

use proptest::prelude::*;
use silvermere_rules::economy::{
    EncumbranceLevel, Payment, amount_to_copper, buy_price, can_pay, carrying_capacity,
    coin_weight, convert_currency, encumbrance_pct, from_copper, max_weight, normalise, pay,
    sell_price, to_copper,
};
use silvermere_types::enums::Currency;
use silvermere_types::records::Coins;

fn coins(runic: u32, platinum: u32, gold: u32, silver: u32, copper: u32) -> Coins {
    Coins {
        copper,
        silver,
        gold,
        platinum,
        runic,
    }
}

// Buying: cost*(markup+100)/100 * (110 - cha/5)/100.
#[test]
fn buy_price_with_charm_and_markup() {
    // A 200-copper estimate is charged 208 at Charm 30 (110 - 6 = 104%).
    assert_eq!(buy_price(200, Currency::Copper, 0, 30), 208);
    assert_eq!(buy_price(200, Currency::Copper, 0, 50), 200);
    // 110 - 85/5 = 93%
    assert_eq!(buy_price(200, Currency::Copper, 0, 85), 186);
    // 100 silver = 1,000 copper, x125/100, x100/100.
    assert_eq!(buy_price(100, Currency::Silver, 25, 50), 1_250);
}

// Buying: the markup multiply and its /100 come before the charm term.
#[test]
fn buy_price_truncates_after_each_step() {
    // 33 * 150 / 100 = 49 (49.5 dropped), 49 * 104 / 100 = 50 (50.96 dropped).
    assert_eq!(buy_price(33, Currency::Copper, 50, 30), 50);
    // cha/5 is integer division: Cha 34 is still 110 - 6.
    assert_eq!(buy_price(200, Currency::Copper, 0, 34), 208);
}

// Buying: over 100,000 copper the price is worked in hundreds.
#[test]
fn buy_price_large_prices_lose_the_low_digits() {
    // 11 platinum = 110,000 copper -> 1,100 hundreds, x104/100 = 1,144, x100.
    assert_eq!(buy_price(11, Currency::Platinum, 0, 30), 114_400);
    // 1,001 gold = 100,100 copper -> 1,001 x 104 / 100 = 1,041 -> 104,100 (not 104,104).
    assert_eq!(buy_price(1_001, Currency::Gold, 0, 30), 104_100);
    // exactly 100,000 is not scaled
    assert_eq!(buy_price(1_000, Currency::Gold, 0, 30), 104_000);
}

// Selling: value in copper times (cha/2 + 25) percent.
#[test]
fn sell_price_with_charm() {
    assert_eq!(sell_price(100, Currency::Copper, 30), 40);
    for cost in [1u16, 7, 100, 999, 65_535] {
        assert_eq!(
            sell_price(cost, Currency::Copper, 50),
            u64::from(cost) * 50 / 100
        );
    }
    assert_eq!(sell_price(3, Currency::Gold, 50), 150);
    assert_eq!(sell_price(3, Currency::Undefined, 50), 0);
}

// convert_currency with the rates 100/100/10/10.
#[test]
fn to_copper_rates() {
    assert_eq!(to_copper(coins(1, 0, 0, 0, 0)), 1_000_000);
    assert_eq!(to_copper(coins(0, 1, 0, 0, 0)), 10_000);
    assert_eq!(to_copper(coins(0, 0, 1, 0, 0)), 100);
    assert_eq!(to_copper(coins(0, 0, 0, 1, 0)), 10);
    assert_eq!(to_copper(coins(0, 0, 0, 0, 1)), 1);
    assert_eq!(to_copper(coins(1, 23, 45, 6, 7)), 1_234_567);
    assert_eq!(amount_to_copper(5, Currency::Platinum), 50_000);
}

// convert_currency: a product that wraps below the count is the count, then the sum guard.
// u32::MAX runic: x100 wraps below the count, so each step keeps u32::MAX.
#[test]
fn to_copper_huge_runic_follows_the_guard() {
    assert_eq!(to_copper(coins(u32::MAX, 0, 0, 0, 0)), u32::MAX);
    // A wrap that lands above the count is kept, worked by hand:
    // 50,000,000 x100 = 5e9 -> 705,032,704 (kept, >= 5e7); x100 -> 1,783,793,664 (kept);
    // x10 -> 658,067,456 < 1,783,793,664, so 1,783,793,664; and the same for copper.
    assert_eq!(to_copper(coins(50_000_000, 0, 0, 0, 0)), 1_783_793_664);
    // A sum that would wrap is not added: the copper stays.
    assert_eq!(convert_currency(0, 0, 0, 1, u32::MAX - 5), u32::MAX - 5);
}

// Changing coins up: copper into the largest coins first.
#[test]
fn from_copper_makes_change() {
    assert_eq!(from_copper(1_234_567), coins(1, 23, 45, 6, 7));
    assert_eq!(from_copper(0), Coins::default());
    assert_eq!(from_copper(9), coins(0, 0, 0, 0, 9));
}

// Changing coins up: each step converts at most 5,000 coins.
#[test]
fn normalise_stops_at_5000_per_step() {
    let n = normalise(coins(0, 0, 0, 60_000, 0));
    // 5,000 x 10 silver -> 5,000 gold; 5,000 gold -> 50 platinum.
    assert_eq!(n, coins(0, 50, 0, 10_000, 0));
}

proptest! {
    // A purse's value in copper changes up to the same coins as the purse itself.
    #[test]
    fn from_copper_of_to_copper_is_normalise(
        r in 0u32..1000, p in 0u32..1000, g in 0u32..1000, s in 0u32..1000, c in 0u32..1000
    ) {
        let purse = coins(r, p, g, s, c);
        prop_assert_eq!(from_copper(to_copper(purse)), normalise(purse));
    }
}

// A purse pays by value.
#[test]
fn can_pay_by_value() {
    assert!(can_pay(coins(0, 0, 1, 0, 0), 100));
    assert!(!can_pay(coins(0, 0, 1, 0, 0), 101));
    assert!(pay(coins(0, 0, 1, 0, 0), 101).is_none());
}

// Paying: exact change, largest coins first.
#[test]
fn pay_exact_change() {
    let Payment { paid, left } = pay(coins(0, 0, 2, 5, 0), 250).unwrap();
    assert_eq!(paid, coins(0, 0, 2, 5, 0));
    assert_eq!(left, Coins::default());
}

// Paying: silver goes before copper even with copper enough.
#[test]
fn pay_with_change_left() {
    let Payment { paid, left } = pay(coins(0, 0, 0, 3, 25), 28).unwrap();
    assert_eq!(paid, coins(0, 0, 0, 2, 8));
    assert_eq!(left, coins(0, 0, 0, 1, 17));
}

// Paying: a gold crown is broken into silver, then a silver
// noble into copper; the broken coins are not listed as paid.
#[test]
fn pay_breaks_a_larger_coin() {
    let Payment { paid, left } = pay(coins(0, 0, 1, 0, 0), 37).unwrap();
    assert_eq!(paid, coins(0, 0, 0, 3, 7));
    assert_eq!(left, coins(0, 0, 0, 6, 3));
    // A runic coin broken all the way down for 5 copper.
    let Payment { paid, left } = pay(coins(1, 0, 0, 0, 0), 5).unwrap();
    assert_eq!(paid, coins(0, 0, 0, 0, 5));
    assert_eq!(left, coins(0, 99, 99, 9, 5));
    assert_eq!(to_copper(left), 1_000_000 - 5);
}

// Carrying capacity = str*48 (+36 per point over 100).
#[test]
fn carrying_capacity_from_strength() {
    // The game shows "Encumbrance: 520/3792" at Str 79.
    assert_eq!(carrying_capacity(79), 3_792);
    assert_eq!(carrying_capacity(100), 4_800);
    assert_eq!(carrying_capacity(101), 4_884);
}

// Maximum weight: carrying capacity raised by Alter Encumbrance percent.
#[test]
fn max_weight_with_alter_encumbrance() {
    assert_eq!(max_weight(79, 0), 3_792);
    assert_eq!(max_weight(101, 0), 4_884);
    assert_eq!(max_weight(79, 10), 3_792 * 110 / 100);
    assert_eq!(max_weight(79, -10), 3_792 * 90 / 100);
    // never 0
    assert_eq!(max_weight(0, 0), 1);
    assert_eq!(max_weight(79, -100), 1);
}

// Coin weight: three coins of a type weigh one, per type.
#[test]
fn coin_weight_per_type() {
    assert_eq!(coin_weight(Coins::default()), 0);
    assert_eq!(coin_weight(coins(2, 2, 2, 2, 2)), 0);
    assert_eq!(coin_weight(coins(3, 0, 0, 0, 0)), 1);
    assert_eq!(coin_weight(coins(0, 0, 5, 7, 300)), 1 + 2 + 100);
}

// Encumbrance: (wgt + coin weight)*100 / max weight.
#[test]
fn encumbrance_percent() {
    assert_eq!(encumbrance_pct(520, Coins::default(), 3_792), 13);
    assert_eq!(encumbrance_pct(520, coins(0, 0, 0, 0, 300), 3_792), 16);
    assert_eq!(encumbrance_pct(3_792, Coins::default(), 3_792), 100);
}

// The inventory's encumbrance word: >66 Heavy, >33 Medium, >16 Light.
#[test]
fn encumbrance_levels() {
    use EncumbranceLevel::*;
    for (pct, want) in [
        (0, None),
        (13, None),
        (16, None),
        (17, Light),
        (33, Light),
        (34, Medium),
        (66, Medium),
        (67, Heavy),
        (150, Heavy),
    ] {
        assert_eq!(EncumbranceLevel::from_pct(pct), want, "{pct}%");
    }
    assert_eq!(None.name(), "None");
}

// T-041: the shop formulas over Charm 30/50/85 and markups 0/100/400, worked by hand from
// `cost*(markup+100)/100*(110-cha/5)/100` and `price*(cha/2+25)/100` for an item of 75
// silver nobles (750 copper)
#[test]
fn shop_price_table() {
    let table: [(u16, u16, u64); 9] = [
        // markup, charm, buy price
        (0, 30, 780),     // 750 * 104/100
        (0, 50, 750),     // 750 * 100/100
        (0, 85, 697),     // 750 * 93/100 = 697.5
        (100, 30, 1_560), // 1500 * 104/100
        (100, 50, 1_500),
        (100, 85, 1_395),
        (400, 30, 3_900), // 3750 * 104/100
        (400, 50, 3_750),
        (400, 85, 3_487), // 3487.5
    ];
    for (markup, cha, price) in table {
        assert_eq!(
            buy_price(75, Currency::Silver, markup, cha),
            price,
            "markup {markup} charm {cha}"
        );
    }
    // selling ignores the markup: (cha/2 + 25)%
    assert_eq!(sell_price(75, Currency::Silver, 30), 300); // 40%
    assert_eq!(sell_price(75, Currency::Silver, 50), 375); // 50%
    assert_eq!(sell_price(75, Currency::Silver, 85), 502); // 67%: 502.5
}
