//! The `SpellEffect` vocabulary: one variant per thing the casting engines do for an
//! ability, built from a spell record's ten ability slots.
//!
//! `census.md` beside this module lists every ability and the variant it became. The slot's
//! own value replaces the rolled strength when it is not 0, in every engine, so an
//! [`EffectSlot`] keeps the value beside the effect and [`EffectSlot::amount`] picks.

use silvermere_types::enums::Ability;
use silvermere_types::records::Spell;

/// A character stat a spell alters (ABL_INTELLECT..ABL_CHARM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stat {
    /// "Alter Intellect": `player.intel`.
    Intellect,
    /// "Alter Wisdom": `player.wil` (the record calls the stat willpower).
    Wisdom,
    /// "Alter Strength": `player.str`.
    Strength,
    /// "Alter Health": `player.hea`.
    Health,
    /// "Alter Agility": `player.agi`.
    Agility,
    /// "Alter Charm": `player.cha`.
    Charm,
}

/// Which half of a teleport a slot carries: both are needed before the engine moves anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeleportPart {
    /// ABL_TELEPORTROOM: the room number.
    Room,
    /// ABL_TELEPORTMAP: the map number.
    Map,
}

/// What one ability slot of a spell does when the spell is cast.
///
/// The variants that carry a number (a monster, item, room, spell or text-block number) read
/// it from the slot value, or from the rolled strength when the slot value is 0, exactly as
/// the engines did; [`EffectSlot::amount`] makes that choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpellEffect {
    /// ABL_DAMAGENOARMOUR (`ignore_armour: true`): hit points lost outright, scaled by the
    /// caster's "Alter spell damage" on self and single targets; ABL_DAMAGEARMOUR (`false`):
    /// the same after the target's magic-resistance multiplier (never a lasting spell).
    Damage {
        /// Whether the damage skips the magic-resistance multiplier.
        ignore_armour: bool,
    },
    /// ABL_HEAL: hit points regained, up to the maximum.
    Heal,
    /// ABL_HEALMANA: mana regained, kept between 0 and the maximum.
    HealMana,
    /// ABL_POISON: the poison level is raised to the amount if lower.
    Poison,
    /// ABL_CUREPOISON: the first poison spell ends and the poison level falls by the amount.
    CurePoison,
    /// ABL_STEALHP: the target loses the amount and the caster gains it, capped at maximum.
    StealHp,
    /// ABL_ENERGY: energy this round changes by the amount.
    Energy,
    /// ABL_HUNGER: the food level changes by the amount.
    Hunger,
    /// ABL_THIRST: the drink level changes by the amount.
    Thirst,
    /// ABL_SUMMON: a known monster (the amount is its number) is generated in the room.
    Summon,
    /// ABL_TELEPORTROOM / ABL_TELEPORTMAP: half of a destination.
    Teleport(TeleportPart),
    /// ABL_CLEARITEM: every copy of item number `amount` leaves the caster's room.
    ClearItem,
    /// ABL_SCATTER: the room's contents move to room `amount`.
    Scatter,
    /// ABL_TEXTBLOCK: text block `amount` is run for the target.
    TextBlock,
    /// ABL_SHATTER: the target's wielded weapon breaks when the amount beats its Quality.
    Shatter,
    /// ABL_REMOVECURSE: cursed worn items come off.
    RemoveCurse,
    /// ABL_FREEDOM: every active spell with Hold person or Paralyze ends.
    Freedom,
    /// ABL_DISPELMAGIC: every active spell with ability `amount` ends (-1: every spell).
    DispelMagic,
    /// ABL_SLAY: hit points fall to one below death (players) or to -2 (monsters).
    Slay,
    /// ABL_INTELLECT..ABL_CHARM: a stat rises by the amount while the spell lasts.
    StatAlter(Stat),
    /// ABL_ALTERHP: maximum and current hit points rise by the amount while the spell lasts.
    AlterHp,
    /// ABL_GIVESPELL: spell number `amount` joins the spellbook while the spell lasts.
    GiveSpell,
    /// ABL_ENSLAVE: a monster target becomes the caster's charmed follower.
    Enslave,
    /// ABL_ENDCAST: spell number `amount` is cast when this one ends (instant: right after).
    EndCast,
    /// ABL_ENDCASTCHANCE: the percent chance of that chain cast (default 100).
    EndCastChance,
    /// ABL_OPENDOOR: the doors of the room are forced with chance `(spellcast - diff)/2`.
    OpenDoor,
    /// ABL_LORE: an item's hidden descriptions, when its level is at most the amount.
    Lore,
    /// ABL_DETECTMAGIC: the band of an item's "Magical" value (a marker on living targets).
    DetectMagic,
    /// ABL_KILLDEAD: a monster target must be undead; a marker on other targets.
    KillDead,
    /// ABL_REMOVESPELL (`chain: true`) / ABL_TERMSPELL (`chain: false`): spell number
    /// `amount` is ended on the target before the other slots apply.
    RemoveSpell {
        /// Whether the ended spell may cast its "Cast on ending" spell.
        chain: bool,
    },
    /// ABL_COMPONENT: item number `value` is used up before the cast (a prelude check).
    Component,
    /// ABL_GUARDED: read from the monster, not the spell; kept so a spell slot holding it
    /// is named.
    Guarded,
    /// The "no effect" lists: the ability describes or gates the spell and does nothing
    /// when applied (EVILINCOMBAT, AFFECTSANIMAL, DESCMSG, NONMAGICAL, ...).
    Marker(Ability),
    /// Every other ability: what the engines do for an ability they do not list. Instant
    /// casts are silly ([`Outcome::Silly`](super::Outcome::Silly));
    /// lasting casts put the spell in a slot, where the ability acts through the ability
    /// lookup (T-032) and the per-round code.
    Lasting(Ability),
    /// An ability number the game never defined.
    Unknown(u16),
}

impl SpellEffect {
    /// The effect of one ability slot. The value is not needed to classify the slot; it is
    /// kept in the [`EffectSlot`] built by [`SpellEffects::from_record`].
    pub fn from_slot(ability: Ability, _value: i16) -> Self {
        use Ability as A;
        match ability {
            A::DamageNoArmour => Self::Damage {
                ignore_armour: true,
            },
            A::DamageArmour => Self::Damage {
                ignore_armour: false,
            },
            A::Heal => Self::Heal,
            A::HealMana => Self::HealMana,
            A::Poison => Self::Poison,
            A::CurePoison => Self::CurePoison,
            A::StealHp => Self::StealHp,
            A::Energy => Self::Energy,
            A::Hunger => Self::Hunger,
            A::Thirst => Self::Thirst,
            A::Summon => Self::Summon,
            A::TeleportRoom => Self::Teleport(TeleportPart::Room),
            A::TeleportMap => Self::Teleport(TeleportPart::Map),
            A::ClearItem => Self::ClearItem,
            A::Scatter => Self::Scatter,
            A::TextBlock => Self::TextBlock,
            A::Shatter => Self::Shatter,
            A::RemoveCurse => Self::RemoveCurse,
            A::Freedom => Self::Freedom,
            A::DispelMagic => Self::DispelMagic,
            A::Slay => Self::Slay,
            A::Intellect => Self::StatAlter(Stat::Intellect),
            A::Wisdom => Self::StatAlter(Stat::Wisdom),
            A::Strength => Self::StatAlter(Stat::Strength),
            A::Health => Self::StatAlter(Stat::Health),
            A::Agility => Self::StatAlter(Stat::Agility),
            A::Charm => Self::StatAlter(Stat::Charm),
            A::AlterHp => Self::AlterHp,
            A::GiveSpell => Self::GiveSpell,
            A::Enslave => Self::Enslave,
            A::EndCast => Self::EndCast,
            A::EndCastChance => Self::EndCastChance,
            A::OpenDoor => Self::OpenDoor,
            A::Lore => Self::Lore,
            A::DetectMagic => Self::DetectMagic,
            A::KillDead => Self::KillDead,
            A::RemoveSpell => Self::RemoveSpell { chain: true },
            A::TermSpell => Self::RemoveSpell { chain: false },
            A::Component => Self::Component,
            A::Guarded => Self::Guarded,
            A::None
            | A::EvilInCombat
            | A::AffectsAnimal
            | A::Quality
            | A::Good
            | A::Evil
            | A::ConfuseMsg
            | A::AffectsLiving
            | A::NonLiving
            | A::NotGood
            | A::NotEvil
            | A::Neutral
            | A::NotNeutral
            | A::AutoUse
            | A::DescMsg
            | A::StartMsg
            | A::NonMagical => Self::Marker(ability),
            A::Unknown(code) => Self::Unknown(code),
            other => Self::Lasting(other),
        }
    }

    /// Whether the slot does nothing when applied: the "no effect" lists.
    pub fn is_marker(self) -> bool {
        matches!(self, Self::Marker(_))
    }

    /// A short stable name for histograms and logs.
    pub fn kind_name(self) -> &'static str {
        match self {
            Self::Damage {
                ignore_armour: true,
            } => "Damage{ignore_armour}",
            Self::Damage {
                ignore_armour: false,
            } => "Damage{armour}",
            Self::Heal => "Heal",
            Self::HealMana => "HealMana",
            Self::Poison => "Poison",
            Self::CurePoison => "CurePoison",
            Self::StealHp => "StealHp",
            Self::Energy => "Energy",
            Self::Hunger => "Hunger",
            Self::Thirst => "Thirst",
            Self::Summon => "Summon",
            Self::Teleport(TeleportPart::Room) => "Teleport{room}",
            Self::Teleport(TeleportPart::Map) => "Teleport{map}",
            Self::ClearItem => "ClearItem",
            Self::Scatter => "Scatter",
            Self::TextBlock => "TextBlock",
            Self::Shatter => "Shatter",
            Self::RemoveCurse => "RemoveCurse",
            Self::Freedom => "Freedom",
            Self::DispelMagic => "DispelMagic",
            Self::Slay => "Slay",
            Self::StatAlter(_) => "StatAlter",
            Self::AlterHp => "AlterHp",
            Self::GiveSpell => "GiveSpell",
            Self::Enslave => "Enslave",
            Self::EndCast => "EndCast",
            Self::EndCastChance => "EndCastChance",
            Self::OpenDoor => "OpenDoor",
            Self::Lore => "Lore",
            Self::DetectMagic => "DetectMagic",
            Self::KillDead => "KillDead",
            Self::RemoveSpell { .. } => "RemoveSpell",
            Self::Component => "Component",
            Self::Guarded => "Guarded",
            Self::Marker(_) => "Marker",
            Self::Lasting(_) => "Lasting",
            Self::Unknown(_) => "Unknown",
        }
    }
}

/// One ability slot of a spell: the effect and the slot's own value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectSlot {
    /// The ability number the slot holds.
    pub ability: Ability,
    /// What it does.
    pub effect: SpellEffect,
    /// The slot's own value, 0 when the rolled strength is to be used.
    pub value: i16,
}

impl EffectSlot {
    /// The amount this slot works with: its own value when that is not 0, else the strength
    /// the spell was cast with.
    pub fn amount(&self, power: i32) -> i32 {
        if self.value != 0 {
            i32::from(self.value)
        } else {
            power
        }
    }
}

/// The effects of a spell record, in slot order, empty slots left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellEffects {
    /// The slots whose ability is not "No ability", in record order.
    pub slots: Vec<EffectSlot>,
}

impl SpellEffects {
    /// Builds the effects of a record from its ten ability slots.
    pub fn from_record(spell: &Spell) -> Self {
        let slots = spell
            .abilities
            .iter()
            .filter(|(ability, _)| *ability != Ability::None)
            .map(|&(ability, value)| EffectSlot {
                ability,
                effect: SpellEffect::from_slot(ability, value),
                value,
            })
            .collect();
        Self { slots }
    }

    /// The slots, in record order.
    pub fn iter(&self) -> impl Iterator<Item = &EffectSlot> {
        self.slots.iter()
    }

    /// Whether any slot holds the ability
    /// ([`spell_has_ability`](crate::abilities::spell_has_ability)).
    pub fn has(&self, ability: Ability) -> bool {
        self.slots.iter().any(|s| s.ability == ability)
    }

    /// The value of the first slot holding the ability, 0 when none does
    /// ([`spell_ability_value`](crate::abilities::spell_ability_value)).
    pub fn value_of(&self, ability: Ability) -> i32 {
        self.slots
            .iter()
            .find(|s| s.ability == ability)
            .map_or(0, |s| i32::from(s.value))
    }

    /// Whether any slot is an ability the game never defined.
    pub fn has_unknown(&self) -> bool {
        self.slots
            .iter()
            .any(|s| matches!(s.effect, SpellEffect::Unknown(_)))
    }
}
