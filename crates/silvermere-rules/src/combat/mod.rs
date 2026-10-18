//! Combat: energy and blows, attack value, hit chance, damage, criticals, BASH and SMASH,
//! dodge, armour and damage reduction.
//!
//! Every melee formula as pure functions. A [`Fighter`] is built from the records and
//! ability totals by [`player_fighter`] or [`monster_fighter`]; one blow is then
//! [`resolve`]d with the attacker's [`Move`] and the [`Rng`](crate::Rng). No I/O, no
//! messages, no session state: T-033 (orchestration) and T-036 (monster AI) call this
//! module for numbers and render the texts themselves.
//!
//! Arithmetic keeps the game's integer order and truncation toward zero: its 16- and 32-bit
//! values are both `i32` here (the 32-bit fields never leave `i32` for sane sheets, and
//! products such as `(av+hitmod)^2` are carried in 32 bits where 16 would overflow). `dr` is
//! in tenths of a point on both sides.
//!
//! The random source is the host's `genrdn(lo, hi)`: a uniform integer in `[lo, hi)`,
//! never `hi`, which is [`Rng::range`](crate::Rng::range). So the hit roll `genrdn(1,100)`
//! is 1..=99 and the damage roll `genrdn(0, max-min+1)` is `0..=max-min`.

mod attack;
mod energy;
mod fighter;

#[cfg(test)]
mod tests;

pub use attack::{
    AttackResult, MoveMods, Outcome, dodge_chance, effective_crit, hit_chance, move_mods, resolve,
};
pub use energy::{
    EnergyRound, MAX_BLOWS, MAXENERGY, MIN_ENERGY, TICK_ENERGY, compute_energy_used, energy_round,
};
pub use fighter::{
    BLIND_LIGHT, Blow, FISTS_SPEED, Fighter, JUMPKICK_SPEED, KICK_SPEED, MonsterAbilities,
    PUNCH_SPEED, PlayerAbilities, is_weapon_too_fast_for_user, isqrt, monster_fighter,
    player_fighter,
};

use silvermere_types::enums::Alignment;

/// The attack move. The discriminants are the game's move numbers; move 8 has no name, only
/// a row in the move table ([`move_mods`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Move {
    /// 1: a martial-arts punch, "fists of fury".
    Punch = 1,
    /// 2: "lightning feet".
    Kick = 2,
    /// 3: "flying feet".
    JumpKick = 3,
    /// 4: "surprise" verbs.
    Backstab = 4,
    /// 5: a plain attack with the wielded weapon or fists, no special move.
    #[default]
    Attack = 5,
    /// 6.
    Bash = 6,
    /// 7.
    Smash = 7,
    /// Move 8: no command sets it; the table gives it -75 AV, +125% damage, no critical.
    Unused8 = 8,
}

impl Move {
    /// BASH or SMASH: the two moves that never critical and multiply damage after damage
    /// reduction.
    pub const fn is_bash_or_smash(self) -> bool {
        matches!(self, Move::Bash | Move::Smash)
    }
}

/// The player's group rank (the record's `rank`): 1 front, 2 back, 0 unranked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Rank {
    /// 0: not in a group, or no rank.
    #[default]
    None,
    /// 1.
    Front,
    /// 2.
    Back,
}

impl From<u8> for Rank {
    fn from(rank: u8) -> Self {
        match rank {
            1 => Rank::Front,
            2 => Rank::Back,
            _ => Rank::None,
        }
    }
}

/// An alignment group: what [`user_align_group`] and [`monster_align_group`] sort the
/// alignments into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignGroup {
    Good,
    Neutral,
    Evil,
}

/// A monster's alignment group: Good and Lawful Good are good; Evil, Chaotic Evil and
/// Lawful Evil are evil; Neutral, Neutral Evil and any unknown code count as neutral
/// (Neutral Evil counting as neutral is what the game does).
pub fn monster_align_group(alignment: Alignment) -> AlignGroup {
    match alignment {
        Alignment::Evil | Alignment::ChaoticEvil | Alignment::LawfulEvil => AlignGroup::Evil,
        Alignment::Good | Alignment::LawfulGood => AlignGroup::Good,
        Alignment::Neutral | Alignment::NeutralEvil | Alignment::Unknown(_) => AlignGroup::Neutral,
    }
}

/// A player's alignment group: the player's legal level (by evil points,
/// [`crate::evil::legal_level`]) sorted into a group. Good (-200 to -51 evil points) and
/// Saint (below -200) are good; Outlaw (40 to 79), Criminal, Villain and FIEND (210 and
/// more) are evil; Neutral (-50 to 29) and Seedy (30 to 39) are neutral.
pub fn user_align_group(evil: i16) -> AlignGroup {
    if evil <= -51 {
        AlignGroup::Good
    } else if evil >= 40 {
        AlignGroup::Evil
    } else {
        AlignGroup::Neutral
    }
}

/// Hit Magical (`attack_user_monster`): the player's Hit Magical total covers part or all
/// of the monster's magic requirement `magreq` first; what is left must be made up by the
/// weapon's Magical value, `weapon_magical`. Hands and
/// feet (`None`: no weapon, or a punch, kick or jumpkick) are never magical. `false` is
/// "Your weapon/fists/feet have no effect against this monster!" (T-033 prints it).
pub const fn can_hit(hitmag: i32, magreq: i32, weapon_magical: Option<i32>) -> bool {
    let left = if hitmag >= magreq { 0 } else { magreq - hitmag };
    if left == 0 {
        return true;
    }
    match weapon_magical {
        None => false,
        Some(magical) => magical >= left,
    }
}

/// Rounds a SMASH knockdown lasts.
pub const SMASH_ROUNDS: i16 = 15;
/// The AV and DV a knocked-down monster loses.
pub const SMASH_PENALTY: i32 = 35;

/// The SMASH knockdown predicate of `attack_user_monster`: `smashpow = str + level`,
/// `smashres = mon.level + magicres + 30`; the monster is knocked down for
/// [`SMASH_ROUNDS`] when `smashpow > smashres`. The comparison is unsigned 16-bit, on the
/// low 16 bits of the monster's level.
pub fn smash_knockdown(str: u16, level: u16, mon_level: u32, magicres: i16) -> bool {
    let smashpow = str.wrapping_add(level);
    let smashres = (mon_level as u16)
        .wrapping_add(magicres as u16)
        .wrapping_add(30);
    smashpow > smashres
}
