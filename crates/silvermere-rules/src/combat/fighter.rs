//! The fighter record and its two builders: [`player_fighter`] and [`monster_fighter`].

use silvermere_types::enums::{ArmourType, ItemType};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2,
};
use silvermere_types::ids::MessageNo;
use silvermere_types::records::{ActiveMonster, Character, Class, Item, KnownMonster};

use super::energy::{MIN_ENERGY, compute_energy_used};
use super::{AlignGroup, Move, SMASH_PENALTY};

/// One side of a blow: the numbers [`resolve`](super::resolve) reads. Built by
/// [`player_fighter`] or [`monster_fighter`], or filled by hand from a captured sheet
/// (T-040). The game's 16-bit fields are `i32` here; the two 32-bit unsigned ones are `u32`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fighter {
    /// Attack value, "AV".
    pub av: i32,
    /// Defence value, "DV": a player's `ac/10`, a monster's raw `ac`.
    pub dv: i32,
    /// Defence against the foe's alignment: the Defence vs Good/Evil abilities,
    /// Shadow Stealth, the group rank and blindness; added to `dv` in the hit chance.
    pub defalign: i32,
    /// Damage resistance in tenths of a point, "DR": `dr/10` comes off each hit.
    pub dr: i32,
    /// Experience for beating it (32-bit unsigned): a monster's `exp*level`,
    /// 0 for a player.
    pub expval: u32,
    /// Energy one blow costs, "EU" (32-bit unsigned): for a player after every
    /// modifier, at least 200 and at most `maxenergy`.
    pub energy: u32,
    /// Least damage of a hit, "D", before the move's percentage.
    pub mindmg: i32,
    /// Most damage of a hit, before the move's percentage.
    pub maxdmg: i32,
    /// Dodge rating, "DG": positive is a chance to dodge, negative a chance to be
    /// hit at 99%. A monster's is its Dodge ability.
    pub dodge: i32,
    /// The hit verbs' message record: the weapon's
    /// `hitmsg`, carried for T-033. Zero for fists or a monster without a weapon.
    pub hitverbs: MessageNo,
    /// The miss verbs' message record, carried for T-033. Zero when
    /// the hit verbs serve.
    pub missverbs: MessageNo,
    /// Critical hit chance in percent, "CR", before the over-40 compression of
    /// [`effective_crit`](super::effective_crit). Always 0 for a monster.
    pub crit: i32,
    /// "Defence Modifier": added to the attacker's hit modifier when this fighter
    /// defends.
    pub defmod: i32,
    /// Whether a player's blow was fast enough for the critical bonus. Not part of the
    /// game's fighter record; [`is_weapon_too_fast_for_user`] reads it after a normal blow.
    pub fast: bool,
}

/// The ability totals [`player_fighter`] reads, summed by T-032 and passed here as plain
/// integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerAbilities {
    /// Dodge.
    pub dodge: i32,
    /// Defence vs Good.
    pub def_vs_good: i32,
    /// Defence vs Evil.
    pub def_vs_evil: i32,
    /// Shadow Stealth held.
    pub shadow_stealth: bool,
    /// Punch: the martial-arts level `n` of a punch.
    pub punch: i32,
    /// Kick.
    pub kick: i32,
    /// Jumpkick.
    pub jumpkick: i32,
    /// Punch AV and Punch damage.
    pub punch_av: i32,
    pub punch_dmg: i32,
    /// Kick AV and Kick damage.
    pub kick_av: i32,
    pub kick_dmg: i32,
    /// Jumpkick AV and Jumpkick damage.
    pub jumpkick_av: i32,
    pub jumpkick_dmg: i32,
    /// Backstab AV, halved into a backstab's attack value.
    pub backstab_av: i32,
    /// Backstab Min. and Backstab Max.
    pub backstab_min: i32,
    pub backstab_max: i32,
}

/// What a blow is and who it is against: the move the values are built for, the move the
/// damage is built for, the opponent's group, plus the light level the blindness test reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blow {
    /// `atkmove`: the move the attack and defence values are built for.
    pub atkmove: Move,
    /// `atktype`: the move the damage and energy are built for. `cmd_backstab` sets it to
    /// `Backstab` with `atkmove` a plain blow when the wielded weapon is not made for
    /// backstabbing; otherwise the two agree.
    pub atktype: Move,
    /// `style`: the opponent's alignment group.
    pub style: AlignGroup,
    /// The light level of the player's room: above 900 is as good as blind.
    pub light_level: i32,
}

impl Blow {
    /// A blow of `mv` against an opponent of `style`, in ordinary light.
    pub const fn new(mv: Move, style: AlignGroup) -> Self {
        Self {
            atkmove: mv,
            atktype: mv,
            style,
            light_level: 0,
        }
    }
}

/// The monster ability totals [`monster_fighter`] reads (T-032 sums the record's slots;
/// plain integers here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonsterAbilities {
    /// Defence vs Good.
    pub def_vs_good: i32,
    /// Defence vs Evil.
    pub def_vs_evil: i32,
    /// Shadow Stealth held.
    pub shadow_stealth: bool,
    /// Dodge.
    pub dodge: i32,
    /// Alter AV (Bless), Alter AV (Curse) and the third attack value.
    pub av_bless: i32,
    pub av_curse: i32,
    pub av_third: i32,
    /// Alter Defence Value.
    pub defence: i32,
    /// Alter Damage.
    pub damage: i32,
    /// Alter Damage Resistance.
    pub damage_resist: i32,
    /// Defence Modifier.
    pub defence_mod: i32,
    /// Alter Energy Usage.
    pub energyuse: i32,
}

/// The speed figures [`player_fighter`] gives [`compute_energy_used`]: normal and slowed
/// (`PlayerStatus2::SLOWED`) for a punch, a kick, a jumpkick and bare fists; a weapon
/// uses its own speed, times 3/2 when slowed.
pub const PUNCH_SPEED: (i32, i32) = (1150, 1750);
pub const KICK_SPEED: (i32, i32) = (1400, 2000);
pub const JUMPKICK_SPEED: (i32, i32) = (1900, 2650);
pub const FISTS_SPEED: (i32, i32) = (1200, 1800);

/// The light level above which a player fights as if blind.
pub const BLIND_LIGHT: i32 = 900;

/// Integer square root: the whole part of `sqrt(n)`, as the game works it out for the
/// training term of the attack value.
pub fn isqrt(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut root = 0;
    while (root + 1) * (root + 1) <= n {
        root += 1;
    }
    root
}

/// The "Alter Energy Usage" step, for a player and for a monster: the percentage is taken
/// as 32-bit unsigned, so a negative value counts as a huge positive one and gives a
/// nonsense cost; only positive percentages are meant. The arithmetic wraps as the game's
/// does.
fn energy_use(energy: u32, pct: i32) -> u32 {
    let eupct = pct as u32;
    if eupct > 0 {
        energy.wrapping_mul(eupct) / 100
    } else {
        energy
    }
}

/// Would the player's normal blow with `weapon` be fast enough for the critical bonus? It
/// builds a throw-away fighter for a plain attack against the player's own alignment group
/// and returns its [`Fighter::fast`]. The armour and abilities do not enter the energy
/// of a normal blow, so none are passed.
pub fn is_weapon_too_fast_for_user(plr: &Character, class: &Class, weapon: Option<&Item>) -> bool {
    let style = super::user_align_group(plr.evil);
    player_fighter(
        plr,
        class,
        weapon,
        &[],
        PlayerAbilities::default(),
        Blow::new(Move::Attack, style),
    )
    .fast
}

/// The player's side of a blow with `weapon` wielded (`None` fights with fists, as the game
/// does when the weapon's item record is gone), `armour` worn, the ability totals in
/// `abilities` and the blow in `blow`. The same builder serves the player as attacker and
/// as defender: it fills every field, and `resolve` reads the ones its side needs.
///
/// The secondary stats of the player record (`mabase`, `critbonus`, `avbonus`, `acbonus`,
/// `dmgbonus`, `drbonus`, `drpct`, `energyuse`, `shield`, `defmod`, `encumpct`) are read off
/// [`Character`]; T-023 and T-032 keep them current ([`crate::abilities::dynamic_stats`]).
/// The status flags (blind, spotted, slowed, knocked down, can cast, race and class
/// stealth) are read off it too.
pub fn player_fighter(
    plr: &Character,
    class: &Class,
    weapon: Option<&Item>,
    armour: &[&Item],
    abilities: PlayerAbilities,
    blow: Blow,
) -> Fighter {
    let level = i32::from(plr.level);
    let str = i32::from(plr.str);
    let agi = i32::from(plr.agi);
    let cha = i32::from(plr.cha);
    let combat = i32::from(class.combat);
    let encumpct = i32::from(plr.encumpct);
    let maxenergy = u32::from(plr.maxenergy);
    let slowed = plr.status2.contains(PlayerStatus2::SLOWED);
    let speed = |(normal, slow): (i32, i32)| if slowed { slow } else { normal };
    let mv = blow.atkmove;

    // start from nothing: 5 energy, 1-4 damage.
    let mut f = Fighter {
        energy: 5,
        mindmg: 1,
        maxdmg: 4,
        ..Fighter::default()
    };

    // defence against the opponent's alignment, when the opponent is in another
    // group. (sic) Against a good foe the "defence against Good" value is at once replaced
    // by the "defence against Evil" one.
    if super::user_align_group(plr.evil) != blow.style {
        match blow.style {
            AlignGroup::Good => {
                f.defalign = abilities.def_vs_good;
                f.defalign = abilities.def_vs_evil;
            }
            AlignGroup::Evil => f.defalign = abilities.def_vs_evil,
            AlignGroup::Neutral => {}
        }
    }
    // Shadow Stealth adds 10 to that defence, whatever the foe.
    if abilities.shadow_stealth {
        f.defalign += 10;
    }

    // crit base: martial-arts base plus bonuses, at least 1.
    f.crit = (i32::from(plr.mabase) + i32::from(plr.critbonus)).max(1);

    // dodge: the ability plus a fifth of charm above 50, a fifth of the level and
    // a third of agility above 50. The dead cannot dodge.
    if plr.hp > 0 {
        f.dodge = abilities.dodge + (cha - 50) / 5 + level / 5 + (agi - 50) / 3;
    } else {
        f.dodge = -1;
    }

    // the weapon. The martial-arts moves cost a fixed speed figure and take their
    // damage from the move's ability and the level, which stops growing at 20; a weapon
    // brings its own figures; without one the player fights with his fists.
    let mut ac = 0;
    let mut accuracy = 0;
    let mut tooweak = false;
    let lvl = level.min(20);
    let martial = |n: i32, max: i32| (lvl * n / 8 + 2, max);
    match mv {
        Move::Punch => {
            f.energy = compute_energy_used(level, combat, agi, speed(PUNCH_SPEED), 0) as u32;
            let n = abilities.punch;
            (f.mindmg, f.maxdmg) = martial(n, (lvl + 3) * n / 4 + 6);
        }
        Move::Kick => {
            f.energy = compute_energy_used(level, combat, agi, speed(KICK_SPEED), 0) as u32;
            let n = abilities.kick;
            (f.mindmg, f.maxdmg) = martial(n, lvl * n / 6 + 7);
        }
        Move::JumpKick => {
            f.energy = compute_energy_used(level, combat, agi, speed(JUMPKICK_SPEED), 0) as u32;
            let n = abilities.jumpkick;
            (f.mindmg, f.maxdmg) = martial(n, lvl * n / 6 + 8);
        }
        _ => match weapon {
            Some(w) => {
                // the weapon's armour class, damage and energy cost.
                ac = i32::from(w.ac);
                f.mindmg = i32::from(w.mindmg);
                f.maxdmg = i32::from(w.maxdmg);
                let wspeed = if slowed {
                    i32::from(w.speed) * 3 / 2
                } else {
                    i32::from(w.speed)
                };
                f.energy =
                    compute_energy_used(level, combat, agi, wspeed, i32::from(w.energy)) as u32;
                // a weapon too heavy for the player's strength costs 1.5% more
                // energy per missing point, and rules out the fast-weapon crit bonus.
                let strreq = i32::from(w.strreq);
                if strreq > str {
                    f.energy = f.energy * ((strreq - str) * 3 + 200) as u32 / 200;
                    tooweak = true;
                }
                // the weapon's accuracy, and its hit and miss verbs.
                accuracy = i32::from(w.accuracy);
                f.hitverbs = w.hitmsg;
                f.missverbs = w.missmsg;
            }
            // fists: the default 1-4 damage.
            None => {
                f.energy = compute_energy_used(level, combat, agi, speed(FISTS_SPEED), 0) as u32;
            }
        },
    }

    // strength: every 10 points above 50 add 1 to the most damage, every 10 above
    // 100 add 2 to the least. The least never exceeds the most, and neither goes below 0.
    f.maxdmg += (str - 50) / 10;
    let strmin = (str - 100) / 10 * 2;
    if strmin > 0 {
        f.mindmg += strmin;
    }
    if f.mindmg > f.maxdmg {
        f.mindmg = f.maxdmg;
    }
    f.mindmg = f.mindmg.max(0);
    f.maxdmg = f.maxdmg.max(0);

    // the worn armour: each piece adds its armour class, damage resistance and
    // accuracy, and its type is noted for the Protective Shield below.
    let (mut leather, mut chain, mut scale, mut plate) = (false, false, false, false);
    for a in armour.iter().filter(|a| a.item_type == ItemType::Armour) {
        ac += i32::from(a.ac);
        f.dr += i32::from(a.dr);
        accuracy += i32::from(a.accuracy);
        match a.armtype {
            ArmourType::SoftLeather
            | ArmourType::SoftStuddedLeather
            | ArmourType::RigidLeather
            | ArmourType::StuddedRigidLeather => leather = true,
            ArmourType::Chainmail => chain = true,
            ArmourType::Scalemail => scale = true,
            ArmourType::Platemail => plate = true,
            _ => {}
        }
    }

    if mv != Move::Backstab {
        // the attack value of a normal blow: accuracy (at least 1) plus the
        // light-load bonus of an unladen, living player (which also sharpens dodge), then
        // skill, training with the whole square root of the level, and the stat terms.
        if accuracy == 0 {
            accuracy = 1;
        }
        if encumpct < 33 && plr.hp > 0 {
            accuracy += 15 - encumpct / 10;
            f.dodge += 10 - encumpct / 10;
        }
        let skill = i64::from((combat - 1) * 2 + level / 2 + accuracy / 2);
        let training = i64::from((combat - 1) * isqrt(level));
        let av = (skill + training) * 2 + i64::from(str - 50) / 3 + i64::from(agi - 50) / 6;
        // The attack value is worked out in 32 bits and kept in 16: the low word.
        f.av = i32::from(av as i16);
    } else {
        // a backstab counts the average of stealth and agility, 5 more with both
        // race and class stealth, 15 less with race stealth alone, plus half the Backstab
        // AV ability, and 10 less when the player has been spotted.
        let mut av = (i32::from(plr.stealth) + agi) / 2;
        if plr.flags.contains(PlayerFlags::RACESTEALTH) {
            if plr.flags.contains(PlayerFlags::CLASSSTEALTH) {
                av += 5;
            } else {
                av -= 15;
            }
        }
        f.av = abilities.backstab_av / 2 + av;
        if plr.status.contains(PlayerStatus::SPOTTED) {
            f.av -= 10;
        }
    }

    // defence value: a tenth of the armour class; against a backstab the average
    // of that and the player's perception.
    f.dv = if mv != Move::Backstab {
        ac / 10
    } else {
        (ac / 10 + i32::from(plr.perception)) / 2
    };

    // the "Alter ..." totals of the player record: attack value, defence value
    // (only a quarter of it against a backstab), most damage and damage resistance (raised
    // by the DR percentage).
    f.av += i32::from(plr.avbonus);
    f.dv += if mv == Move::Backstab {
        i32::from(plr.acbonus) / 4
    } else {
        i32::from(plr.acbonus)
    };
    f.maxdmg += i32::from(plr.dmgbonus);
    f.dr += i32::from(plr.drbonus);
    f.dr = ((i64::from(plr.drpct) + 100) * i64::from(f.dr) / 100) as i32;

    // martial-arts moves: their "Alter ... AV" and "... DMG" abilities.
    let (mav, mdmg) = match mv {
        Move::Punch => (abilities.punch_av, abilities.punch_dmg),
        Move::Kick => (abilities.kick_av, abilities.kick_dmg),
        Move::JumpKick => (abilities.jumpkick_av, abilities.jumpkick_dmg),
        _ => (0, 0),
    };
    f.av += mav;
    f.mindmg += mdmg;
    f.maxdmg += mdmg;

    if blow.atktype == Move::Backstab {
        // a backstab costs the whole round and does twice the damage plus twice
        // the level plus a tenth of the stealth plus the Backstab Min./Max. abilities, cut
        // to 75% for a race-only stealth, then raised by level percent. This reads
        // `atktype`, so a backstab with a weapon not made for it still gets the damage and
        // the cost while its attack and defence values are the ordinary ones.
        f.energy = maxenergy;
        let av = level * 2 + i32::from(plr.stealth) / 10;
        f.mindmg = f.mindmg * 2 + av + abilities.backstab_min;
        f.maxdmg = f.maxdmg * 2 + av + abilities.backstab_max;
        if plr.flags.contains(PlayerFlags::RACESTEALTH)
            && !plr.flags.contains(PlayerFlags::CLASSSTEALTH)
        {
            f.mindmg = f.mindmg * 75 / 100;
            f.maxdmg = f.maxdmg * 75 / 100;
        }
        f.mindmg = f.mindmg * (level + 100) / 100;
        f.maxdmg = f.maxdmg * (level + 100) / 100;
    } else {
        // any other blow has its cost changed by "Alter Energy Usage".
        f.energy = energy_use(f.energy, i32::from(plr.energyuse));
    }

    // group rank (1 front, 2 back): the front rank attacks better (+15 AV) but
    // defends worse (-10 defence against alignment), the back rank the other way round
    // (-10 AV, +15). A jumpkick keeps its AV. Rank 0 changes nothing. (sic) A front-rank
    // player also gets the back-rank change: +5 AV and +5 defence in all. Any other rank
    // value counts as the front rank.
    let not_jumpkick = mv != Move::JumpKick;
    match plr.rank {
        0 => {}
        2 => {
            if not_jumpkick {
                f.av -= 10;
            }
            f.defalign += 15;
        }
        _ => {
            if not_jumpkick {
                f.av += 15;
            }
            f.defalign -= 10;
            if not_jumpkick {
                f.av -= 10;
            }
            f.defalign += 15;
        }
    }

    // blind, or standing in a light level above 900: -10 AV and defence.
    if plr.status.contains(PlayerStatus::BLIND) || blow.light_level > BLIND_LIGHT {
        f.av -= 10;
        f.defalign -= 10;
    }

    // Protective Shield adds to the defence value, less through metal armour: a
    // quarter with platemail, a third with chain or scale, half with leather. Against a
    // backstab only a quarter of what is left counts.
    let mut shield = i32::from(plr.shield);
    if shield != 0 {
        if plate {
            shield /= 4;
        } else if chain || scale {
            shield /= 3;
        } else if leather {
            shield /= 2;
        }
        f.dv += if mv == Move::Backstab {
            shield / 4
        } else {
            shield
        };
    }
    f.defmod = i32::from(plr.defmod);

    // encumbrance: from 75% of the cost unloaded to 125% fully loaded.
    f.energy = f.energy.wrapping_mul((encumpct / 2 + 75) as u32) / 100;

    // a fast blow (under 200 energy, before the minimum of 200 at the end) by a
    // player not too heavily loaded and strong enough for the weapon raises the critical
    // chance by up to 20 (the faster the blow the more; half as much from 33% up).
    if f.energy < 200 && encumpct <= 66 && !tooweak {
        let n = (200 - f.energy as i32 + (agi - 50) / 10).min(20);
        if encumpct < 33 {
            f.crit += n;
        } else {
            f.crit += n / 2;
        }
        f.fast = true;
    }

    // attacking after casting in the same round (`PlayerSettings::CANCAST` cleared) costs 10%
    // more, except for a backstab.
    if mv != Move::Backstab && !plr.settings.contains(PlayerSettings::CANCAST) {
        f.energy = f.energy * 110 / 100;
    }

    // a bash costs double, a smash the whole round.
    match mv {
        Move::Bash => f.energy *= 2,
        Move::Smash => f.energy = maxenergy,
        _ => {}
    }

    // knocked down: AV and DV drop by the smash penalty, not below 0.
    if plr.status2.contains(PlayerStatus2::KNOCKEDDOWN) {
        f.av = (f.av - SMASH_PENALTY).max(0);
        f.dv = (f.dv - SMASH_PENALTY).max(0);
    }

    // every blow costs at least 200 and at most a whole round.
    if f.energy < MIN_ENERGY {
        f.energy = MIN_ENERGY;
    }
    if maxenergy < f.energy {
        f.energy = maxenergy;
    }
    f
}

/// The monster's side of a blow with its attack slot `attack` (an index into
/// `kmon.attacks`, 0..4), its wielded `weapon` (for the hit and miss verbs), its ability
/// totals and the blow it is in (`style` is the opponent's group; a backstab is defended
/// differently).
pub fn monster_fighter(
    mon: &ActiveMonster,
    kmon: &KnownMonster,
    attack: usize,
    weapon: Option<&Item>,
    abilities: MonsterAbilities,
    blow: Blow,
) -> Fighter {
    let atk = &kmon.attacks[attack];
    let ac = i32::from(mon.ac);
    let backstab = blow.atkmove == Move::Backstab;

    let mut f = Fighter {
        // attack value: the accuracy of the chosen attack slot, plus the
        // three "Alter AV" abilities (bless, curse, third).
        av: i32::from(atk.accuracy) + abilities.av_bless + abilities.av_curse + abilities.av_third,
        // defence value: the armour class, or against a backstab the average of
        // half of it and twice the backstab defence; plus "Alter Defence Value".
        dv: if backstab {
            (ac / 2 + i32::from(kmon.bsdefense) * 2) / 2
        } else {
            ac
        } + abilities.defence,
        // damage resistance, kept in tenths; plus "Alter Damage Resistance".
        dr: i32::from(mon.dr) * 10 + abilities.damage_resist,
        // experience for beating it: its experience times its level (32-bit unsigned).
        expval: (mon.exp as u32).wrapping_mul(mon.level),
        // energy cost and damage range of the attack slot; "Alter
        // Damage" moves both ends of the range.
        energy: i32::from(atk.energy) as u32,
        mindmg: i32::from(atk.min) + abilities.damage,
        maxdmg: i32::from(atk.max) + abilities.damage,
        // monsters dodge only through the Dodge ability, and never land a
        // critical hit.
        dodge: abilities.dodge,
        crit: 0,
        // the "Defence Modifier" `resolve` adds to the attacker's hit chance.
        defmod: abilities.defence_mod,
        // the weapon's hit and miss verbs; none without a weapon.
        hitverbs: weapon.map_or(MessageNo(0), |w| w.hitmsg),
        missverbs: weapon.map_or(MessageNo(0), |w| w.missmsg),
        defalign: 0,
        fast: false,
    };

    // defence against the opponent's alignment, when the opponent is in another
    // group. (sic) The good bonus never counts: against a good foe it is replaced by the
    // evil one.
    if super::monster_align_group(kmon.alignment) != blow.style {
        match blow.style {
            AlignGroup::Good => {
                f.defalign = abilities.def_vs_good;
                f.defalign = abilities.def_vs_evil;
            }
            AlignGroup::Evil => f.defalign = abilities.def_vs_evil,
            AlignGroup::Neutral => {}
        }
    }
    // a monster with Shadow Stealth is 10 points harder to hit.
    if abilities.shadow_stealth {
        f.defalign += 10;
    }

    // "Alter Energy Usage" scales the cost by a percentage, never above the
    // energy the monster has per round.
    if (abilities.energyuse as u32) > 0 {
        f.energy = energy_use(f.energy, abilities.energyuse);
        let per_round = u32::from(kmon.energy);
        if per_round < f.energy {
            f.energy = per_round;
        }
    }

    // the attack penalty flag: the smash penalty comes off both its attack and
    // its defence value, never below 0.
    if mon.flags.contains(ActiveMonsterFlags::ATKPEN) {
        f.av = (f.av - SMASH_PENALTY).max(0);
        f.dv = (f.dv - SMASH_PENALTY).max(0);
    }
    f
}
