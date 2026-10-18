//! Ability totals over every source, for a character and for a monster.
//!
//! One iterator, [`Source`], yields the sources in the game's order and one accumulator,
//! [`Total`], applies the stacking rule; the pair variant runs two accumulators over one
//! pass, and [`dynamic_stats`](super::dynamic_stats) walks the same sources.

use silvermere_types::enums::{Ability, ItemType, WornLocation};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, SpellNo};
use silvermere_types::records::{
    AbilitySlot, ActiveMonster, Character, Class, Item, KnownMonster, Race, Spell,
};

use super::table::{BEST_VALUE, MONSTER_BEST_VALUE};

/// Ability totals of one character or monster, counting every source.
///
/// The totals are `i32` so consumers (combat, secondary stats) add them to their own
/// arithmetic without casts. The game sums in 16 bits; no stock data comes near the width,
/// so the wrap is not reproduced.
pub trait AbilityLookup {
    /// The total of `ability` over every source, for a character or for a monster.
    ///
    /// 0 when nothing gives the ability, when a spell negates it, or when `ability` is
    /// [`Ability::None`].
    fn value(&self, ability: Ability) -> i32;

    /// Whether anything gives the ability at all, without adding values. Unlike [`value`],
    /// a "Negate ability" spell is not looked at.
    ///
    /// [`value`]: AbilityLookup::value
    fn has(&self, ability: Ability) -> bool;

    /// Two totals from one pass over the sources. Each total is what [`value`] gives for its
    /// ability.
    ///
    /// [`value`]: AbilityLookup::value
    fn pair(&self, first: Ability, second: Ability) -> (i32, i32);

    /// The total before its last step: `None` for a best-value ability ([`BEST_VALUE`]) no
    /// source carries and no spell negates. [`value`] maps that `None` to 0.
    ///
    /// [`value`]: AbilityLookup::value
    fn best(&self, ability: Ability) -> Option<i32>;
}

/// The value of an ability on one item: the first of the item's twenty slots holding the
/// ability, or 0 when none does. An item carrying the same ability twice counts only the
/// first slot.
pub fn item_ability_value(item: &Item, ability: Ability) -> i32 {
    first_slot_value(&item.abilities, ability)
}

/// Whether an item has an ability in any of its slots.
pub fn item_has_ability(item: &Item, ability: Ability) -> bool {
    item.abilities.iter().any(|&(a, _)| a == ability)
}

/// The value of an ability on one spell record: the first of the spell's ten slots holding
/// the ability, or 0 when none does. A 0 in a spell slot means "the value rolled when the
/// spell was cast" to the totals, so a caller
/// that needs the amount must allow for it.
pub fn spell_ability_value(spell: &Spell, ability: Ability) -> i32 {
    first_slot_value(&spell.abilities, ability)
}

/// Whether a spell has an ability in any of its slots.
pub fn spell_has_ability(spell: &Spell, ability: Ability) -> bool {
    spell.abilities.iter().any(|&(a, _)| a == ability)
}

fn first_slot_value(slots: &[AbilitySlot], ability: Ability) -> i32 {
    slots
        .iter()
        .find(|&&(a, _)| a == ability)
        .map_or(0, |&(_, value)| i32::from(value))
}

/// The ability an ability slot's value names, for a "Negate ability" slot whose value is the
/// number of the ability to cancel. A negative value names nothing.
pub(crate) fn negate_target(value: i16) -> Option<Ability> {
    u16::try_from(value).ok().map(Ability::from_code)
}

/// One source of ability slots, in the order the game counts them.
pub(crate) enum Source<'a> {
    /// The character's own slots (`abils`), its race's or class's slots, or a
    /// known monster's slots: every slot counts.
    Slots(&'a [AbilitySlot]),
    /// A spell in effect, with the value rolled when it was cast (`spellvals`), which a
    /// slot whose own value is 0 takes.
    Spell { spell: &'a Spell, rolled: i16 },
    /// The character's race, class, or a worn, wielded or carried item, read by record:
    /// `None` is an item number whose record is missing, which the monster total still
    /// counts as a value of 0 (a character's missing item is skipped and never yielded).
    Item(Option<&'a Item>),
}

/// The running total of one ability over the sources: `Some` once a source gave it, `None`
/// while nothing has. Additive abilities start at `Some(0)`.
struct Total {
    ability: Ability,
    additive: bool,
    value: Option<i32>,
    negated: bool,
}

impl Total {
    fn new(ability: Ability, best_value: &[Ability]) -> Self {
        let additive = !best_value.contains(&ability);
        Self {
            ability,
            additive,
            value: additive.then_some(0),
            negated: false,
        }
    }

    /// Adds a value, or keeps the higher one for a best-value ability.
    ///
    /// In the game a value of -32000 or below never counts towards a best value; such a
    /// value cannot come from stock data and is kept here.
    fn add(&mut self, value: i32) {
        self.value = Some(match self.value {
            Some(total) if self.additive => total + value,
            Some(total) => total.max(value),
            None => value,
        });
    }

    /// Every slot holding the ability counts.
    fn slots(&mut self, slots: &[AbilitySlot]) {
        for &(ability, value) in slots {
            if ability == self.ability {
                self.add(i32::from(value));
            }
        }
    }

    /// A spell's slots, with the rolled value standing in for a 0 and a "Negate ability"
    /// slot naming this ability marking the negation.
    fn spell(&mut self, spell: &Spell, rolled: i16) {
        for &(ability, value) in &spell.abilities {
            if ability == self.ability {
                let value = if value != 0 { value } else { rolled };
                self.add(i32::from(value));
            }
            if ability == Ability::Negate && i32::from(value) == i32::from(self.ability.code()) {
                self.negated = true;
            }
        }
    }

    /// An item contributes the value of its first slot with the ability, 0 when it has none:
    /// that 0 is still a candidate for a best-value total.
    fn item(&mut self, item: Option<&Item>) {
        self.add(item.map_or(0, |item| item_ability_value(item, self.ability)));
    }

    fn source(&mut self, source: &Source<'_>) {
        match *source {
            Source::Slots(slots) => self.slots(slots),
            Source::Spell { spell, rolled } => self.spell(spell, rolled),
            Source::Item(item) => self.item(item),
        }
    }

    /// The last step: a negation wins, then a best-value total that found nothing becomes 0
    /// (in [`AbilityLookup::value`]).
    fn finish(self) -> Option<i32> {
        if self.negated { Some(0) } else { self.value }
    }
}

/// A character and the records its ability totals read, borrowed from the engine.
///
/// `race` and `class` are `None` when the record is missing, which the totals skip.
/// `spells` and `items` resolve a number to its record and return `None` for one that no
/// longer exists, which the totals skip too.
pub struct CharacterAbilities<'a> {
    pub character: &'a Character,
    pub race: Option<&'a Race>,
    pub class: Option<&'a Class>,
    pub spells: &'a dyn Fn(SpellNo) -> Option<&'a Spell>,
    pub items: &'a dyn Fn(ItemNo) -> Option<&'a Item>,
}

impl<'a> CharacterAbilities<'a> {
    /// The sources in the game's order (the same for the dynamic stats): own slots, spells
    /// cast, race, class, worn items, the wielded weapon, then the pack items that are
    /// neither weapons nor wearable. A spare sword or an unworn ring in the pack gives
    /// nothing.
    pub(crate) fn sources(&self) -> impl Iterator<Item = Source<'a>> + '_ {
        let plr = self.character;
        let own = core::iter::once(Source::Slots(&plr.abils));
        let spells = plr
            .spells
            .iter()
            .zip(&plr.spellvals)
            .filter(|&(&spell, _)| spell != SpellNo(0))
            .filter_map(|(&spell, &rolled)| {
                (self.spells)(spell).map(|spell| Source::Spell { spell, rolled })
            });
        let race = self.race.map(|r| Source::Slots(&r.abilities));
        let class = self.class.map(|c| Source::Slots(&c.abilities));
        let worn = plr.worn.iter().chain(core::iter::once(&plr.weapon));
        let pack = plr.items.iter();
        let items = worn
            .map(|&item| (item, false))
            .chain(pack.map(|&item| (item, true)))
            .filter(|&(item, _)| item != ItemNo(0))
            .filter_map(move |(item, carried)| (self.items)(item).map(|rec| (rec, carried)))
            .filter(|&(rec, carried)| {
                !carried || (rec.item_type != ItemType::Weapon && rec.worn == WornLocation::Nowhere)
            })
            .map(|(rec, _)| Source::Item(Some(rec)));
        own.chain(spells).chain(race).chain(class).chain(items)
    }

    /// The spells in effect, each with its rolled value (`spells`/`spellvals`), skipping
    /// empty slots and spells whose record no longer exists.
    pub(crate) fn spells_cast(&self) -> impl Iterator<Item = (&'a Spell, i16)> + '_ {
        self.sources().filter_map(|source| match source {
            Source::Spell { spell, rolled } => Some((spell, rolled)),
            _ => None,
        })
    }

    fn total(&self, ability: Ability) -> Option<i32> {
        if ability == Ability::None {
            return Some(0);
        }
        let mut total = Total::new(ability, &BEST_VALUE);
        for source in self.sources() {
            total.source(&source);
        }
        total.finish()
    }
}

impl AbilityLookup for CharacterAbilities<'_> {
    /// A character's total of one ability.
    fn value(&self, ability: Ability) -> i32 {
        self.total(ability).unwrap_or(0)
    }

    /// Whether a character has the ability.
    ///
    /// Keeps the game's slip (sic): a spell's abilities are found only while the spell's own
    /// slot `i` is in use, `i` being the index of the spell among the ten cast on the
    /// character, not the index of the ability slot.
    fn has(&self, ability: Ability) -> bool {
        if ability == Ability::None {
            return false;
        }
        let plr = self.character;
        let spells = plr
            .spells
            .iter()
            .enumerate()
            .filter(|&(_, &spell)| spell != SpellNo(0))
            .filter_map(|(i, &spell)| (self.spells)(spell).map(|spell| (i, spell)))
            .any(|(i, spell)| {
                spell.abilities[i].0 != Ability::None && spell_has_ability(spell, ability)
            });
        let records = self.sources().any(|source| match source {
            Source::Slots(slots) => slots.iter().any(|&(a, _)| a == ability),
            Source::Item(Some(item)) => item_has_ability(item, ability),
            Source::Item(None) | Source::Spell { .. } => false,
        });
        spells || records
    }

    /// A character's totals of two abilities from one pass, each negated only by a "Negate
    /// ability" slot naming it. With `first` [`Ability::None`] the game works out neither;
    /// both totals are 0 here.
    fn pair(&self, first: Ability, second: Ability) -> (i32, i32) {
        if first == Ability::None {
            return (0, 0);
        }
        let mut a = Total::new(first, &BEST_VALUE);
        let mut b = Total::new(second, &BEST_VALUE);
        for source in self.sources() {
            a.source(&source);
            b.source(&source);
        }
        (a.finish().unwrap_or(0), b.finish().unwrap_or(0))
    }

    fn best(&self, ability: Ability) -> Option<i32> {
        self.total(ability)
    }
}

/// An active monster, its known-monster record and the records its totals read.
///
/// `known` is the template the monster was generated from; `None` when it is missing,
/// which the totals skip.
pub struct MonsterAbilities<'a> {
    pub monster: &'a ActiveMonster,
    pub known: Option<&'a KnownMonster>,
    pub spells: &'a dyn Fn(SpellNo) -> Option<&'a Spell>,
    pub items: &'a dyn Fn(ItemNo) -> Option<&'a Item>,
}

/// The active number that means "no real monster in this slot" (65535): such a monster has
/// no abilities.
const NO_MONSTER: ActiveMonsterId = ActiveMonsterId(65535);

impl<'a> MonsterAbilities<'a> {
    /// The sources in the game's order: the spells on the monster,
    /// its known-monster record, every item it carries, its weapon and its worn item. Unlike
    /// a character's pack, every carried item counts.
    pub(crate) fn sources(&self) -> impl Iterator<Item = Source<'a>> + '_ {
        let mon = self.monster;
        let spells = mon
            .spells
            .iter()
            .filter(|cast| cast.spell != SpellNo(0))
            .filter_map(|cast| {
                (self.spells)(cast.spell).map(|spell| Source::Spell {
                    spell,
                    rolled: cast.value,
                })
            });
        let known = self.known.map(|k| Source::Slots(&k.abilities));
        let items = mon
            .items
            .iter()
            .map(|&(item, _)| item)
            .chain([mon.weapon, mon.wornitem])
            .filter(|&item| item != ItemNo(0))
            .map(|item| Source::Item((self.items)(item)));
        spells.chain(known).chain(items)
    }

    fn total(&self, ability: Ability) -> Option<i32> {
        let mut total = Total::new(ability, &MONSTER_BEST_VALUE);
        for source in self.sources() {
            total.source(&source);
        }
        total.finish()
    }
}

impl AbilityLookup for MonsterAbilities<'_> {
    /// A monster's total of one ability. A "Negate ability" spell naming the ability makes
    /// it 0; unlike a character's, [`Ability::None`] is not special.
    fn value(&self, ability: Ability) -> i32 {
        self.total(ability).unwrap_or(0)
    }

    /// Whether a monster has the ability: false for the 65535 "no monster" slot, else
    /// whether any spell, the known record, a carried item, the weapon or the
    /// worn item has the ability.
    fn has(&self, ability: Ability) -> bool {
        if self.monster.number == NO_MONSTER {
            return false;
        }
        self.sources().any(|source| match source {
            Source::Spell { spell, .. } => spell_has_ability(spell, ability),
            Source::Slots(slots) => slots.iter().any(|&(a, _)| a == ability),
            Source::Item(item) => item.is_some_and(|item| item_has_ability(item, ability)),
        })
    }

    /// Monsters have no two-ability form in the game; this is two passes.
    fn pair(&self, first: Ability, second: Ability) -> (i32, i32) {
        (self.value(first), self.value(second))
    }

    fn best(&self, ability: Ability) -> Option<i32> {
        self.total(ability)
    }
}
