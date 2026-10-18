//! [`can_use`]: may a character use (wear, wield, light, read) an item? Pure: the refusal
//! reason is data and the callers print their own text.

use silvermere_rules::abilities::item_has_ability;
use silvermere_rules::spells::{LegalBand, can_use_spell};
use silvermere_types::enums::{Ability, ItemType, WeaponClass, WeaponType, WornLocation};
use silvermere_types::ids::{ClassNo, ItemNo, RaceNo, SpellNo};
use silvermere_types::records::{Character, Class, Item, Spell};

/// `exemptitem1..3`, set at startup: the only weapons a `WCLS_LIMITED` class may use outside
/// its own class items.
pub const EXEMPT_ITEMS: [ItemNo; 3] = [ItemNo(100), ItemNo(68), ItemNo(0)];

/// The five classes allowed item type 11.
pub const TYPE11_CLASSES: [ClassNo; 5] =
    [ClassNo(1), ClassNo(2), ClassNo(3), ClassNo(4), ClassNo(9)];

/// Why `user_can_use` said no, in the order it tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CantUse {
    /// Item type 11 and a class not in [`TYPE11_CLASSES`].
    Type11,
    /// An alignment ability against the legal band.
    Alignment,
    /// The class list names classes, not this one.
    Class,
    /// The race list names races, not this one.
    Race,
    /// An Anti-Magic character and a Magical item.
    AntiMagic,
    /// Below "Minimum level allowed".
    MinLevel,
    /// Above "Maximum level allowed".
    MaxLevel,
    /// The weapon type is not the class's, or a Limited class and no exempt item.
    WeaponClass,
    /// Armour heavier than the class wears, or an off-hand item for a one-handed class.
    Armour,
    /// A scroll whose linked spell the character may not use.
    Scroll,
}

/// What `user_can_use` reads besides the records.
pub struct UseCheck<'a> {
    /// The character's class record; `None` refuses everything, as a missing class did
    /// in the original.
    pub class: Option<&'a Class>,
    /// Whether the character has Anti Magic, from any source.
    pub anti_magic: bool,
    /// The spell records, for a scroll's Link to Spell.
    pub spells: &'a dyn Fn(SpellNo) -> Option<&'a Spell>,
}

/// May `plr` use `item`? `Err` names the first refusal, in the original's order.
pub fn can_use(plr: &Character, item: &Item, check: &UseCheck<'_>) -> Result<(), CantUse> {
    let Some(cls) = check.class else {
        return Err(CantUse::Class);
    };
    // Item type 11 (one past ITEM_GENERAL) only for five classes
    if item.item_type.code() == 11 && !TYPE11_CLASSES.contains(&plr.class) {
        return Err(CantUse::Type11);
    }
    // Alignment by the legal level of the evil points
    let has = |a| item_has_ability(item, a);
    let forbidden = match LegalBand::of_evil(plr.evil) {
        LegalBand::Good => has(Ability::Evil) || has(Ability::NotGood) || has(Ability::Neutral),
        LegalBand::Neutral => has(Ability::Evil) || has(Ability::Good),
        LegalBand::Evil => has(Ability::Good) || has(Ability::NotEvil) || has(Ability::Neutral),
    };
    if forbidden {
        return Err(CantUse::Alignment);
    }
    // The class and race lists: a list that names anyone must name him
    let mut named = false;
    let classes: Vec<ClassNo> = item
        .classes
        .iter()
        .copied()
        .filter(|&c| c != ClassNo(0))
        .collect();
    if !classes.is_empty() {
        if !classes.contains(&plr.class) {
            return Err(CantUse::Class);
        }
        named = true;
    }
    let races: Vec<RaceNo> = item
        .races
        .iter()
        .copied()
        .filter(|&r| r != RaceNo(0))
        .collect();
    if !races.is_empty() {
        if !races.contains(&plr.race) {
            return Err(CantUse::Race);
        }
        named = true;
    }
    // Anti Magic characters cannot use Magical items
    if check.anti_magic && has(Ability::Magical) {
        return Err(CantUse::AntiMagic);
    }
    // The level abilities, slot by slot, compared as unsigned 16-bit numbers: a negative
    // minimum refuses everyone (sic).
    for &(ability, value) in &item.abilities {
        match ability {
            Ability::MinLevel if plr.level < value as u16 => {
                return Err(CantUse::MinLevel);
            }
            Ability::MaxLevel if plr.level > value as u16 => {
                return Err(CantUse::MaxLevel);
            }
            _ => {}
        }
    }
    // Named in a list: allowed whatever the type
    if named {
        return Ok(());
    }
    let mut ok = match item.item_type {
        ItemType::Weapon => weapon_allowed(item, cls.weapon),
        ItemType::Armour => {
            if item.armtype.code() > cls.armour.code() {
                false
            } else {
                !(matches!(
                    cls.weapon,
                    WeaponClass::OneHandBlunt | WeaponClass::OneHandSharp | WeaponClass::AnyOneHand
                ) && item.worn == WornLocation::OffHand)
            }
        }
        _ => true,
    };
    let refusal = match item.item_type {
        ItemType::Weapon => CantUse::WeaponClass,
        _ => CantUse::Armour,
    };
    // "Class Item Inclusion" names a class that may use it anyway
    if !ok {
        ok = item
            .abilities
            .iter()
            .any(|&(a, v)| a == Ability::ClassItem && plr.class.0 == v as u16);
    }
    if !ok {
        return Err(refusal);
    }
    // A scroll is usable only when its linked spell is; every Link to Spell slot
    // decides in turn and the last one's verdict stands
    if item.item_type == ItemType::Scroll {
        let mut verdict = Ok(());
        for &(a, v) in &item.abilities {
            if a == Ability::LinkSpell {
                verdict = match (check.spells)(SpellNo(v)) {
                    Some(spl) => can_use_spell(spl, cls, plr.level, LegalBand::of_evil(plr.evil))
                        .map_err(|_| CantUse::Scroll),
                    // A missing spell record refuses
                    None => Err(CantUse::Scroll),
                };
            }
        }
        return verdict;
    }
    Ok(())
}

/// The weapon-type table of [`can_use`]: a weapon type the table
/// does not know is refused before the class is looked at, so a Limited class may use an
/// exempt item only when its type is one of the four.
fn weapon_allowed(item: &Item, class: WeaponClass) -> bool {
    use WeaponClass as C;
    let known = matches!(
        item.wpntype,
        WeaponType::OneHandBlunt
            | WeaponType::OneHandSharp
            | WeaponType::TwoHandBlunt
            | WeaponType::TwoHandSharp
    );
    if class == C::Limited {
        return known && EXEMPT_ITEMS.contains(&item.number);
    }
    match item.wpntype {
        WeaponType::OneHandBlunt => {
            matches!(
                class,
                C::OneHandBlunt | C::AnyOneHand | C::AnyBlunt | C::All
            )
        }
        WeaponType::OneHandSharp => {
            matches!(
                class,
                C::OneHandSharp | C::AnyOneHand | C::AnySharp | C::All
            )
        }
        WeaponType::TwoHandBlunt => {
            matches!(
                class,
                C::TwoHandBlunt | C::AnyTwoHand | C::AnyBlunt | C::All
            )
        }
        WeaponType::TwoHandSharp => {
            matches!(
                class,
                C::TwoHandSharp | C::AnyTwoHand | C::AnySharp | C::All
            )
        }
        _ => false,
    }
}
