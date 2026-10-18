//! The database update's known-monster merge.

use crate::decode::MonsterRecord;

/// Merges a known-monster update. The ability, item, attack and spell arrays, the name, the
/// four description lines and every other design member are compared; when one differs all
/// of them are copied. Kept: `number` (the key), `curalive`, `killdate`, `killtime`, the
/// `unk_0x004` span and `dirty`. Returns whether the monster was copied.
pub fn merge_monster(old: &mut MonsterRecord, new: &MonsterRecord) -> bool {
    let changed = old.abilities != new.abilities
        || old.abilvals != new.abilvals
        || old.items != new.items
        || old.itemuses != new.itemuses
        || old.itempcts != new.itempcts
        || old.atktypes != new.atktypes
        || old.atkaccs != new.atkaccs
        || old.atkpcts != new.atkpcts
        || old.atkmins != new.atkmins
        || old.atkmaxs != new.atkmaxs
        || old.hitmsgs != new.hitmsgs
        || old.defmsgs != new.defmsgs
        || old.missmsgs != new.missmsgs
        || old.hitspells != new.hitspells
        || old.atkenergies != new.atkenergies
        || old.spells != new.spells
        || old.spellpcts != new.spellpcts
        || old.spelllvls != new.spelllvls
        || old.name != new.name
        || old.desc != new.desc
        || old.montype != new.montype
        || old.level != new.level
        || old.genindex != new.genindex
        || old.wornitem != new.wornitem
        || old.weapon != new.weapon
        || old.dr != new.dr
        || old.ac != new.ac
        || old.group != new.group
        || old.followpct != new.followpct
        || old.magicres != new.magicres
        || old.bsdefense != new.bsdefense
        || old.exp != new.exp
        || old.maxhp != new.maxhp
        || old.energy != new.energy
        || old.hpregen != new.hpregen
        || old.maxalive != new.maxalive
        || old.grouprole != new.grouprole
        || old.maxfollow != new.maxfollow
        || old.undead != new.undead
        || old.alignment != new.alignment
        || old.unk_0x0AB != new.unk_0x0AB
        || old.regenhrs != new.regenhrs
        || old.arrivemsg != new.arrivemsg
        || old.deathmsg != new.deathmsg
        || old.cash != new.cash
        || old.gender != new.gender
        || old.talktext != new.talktext
        || old.charmlvl != new.charmlvl
        || old.namelist != new.namelist
        || old.charmres != new.charmres
        || old.unk_0x192 != new.unk_0x192
        || old.deathspell != new.deathspell
        || old.unk_0x19F != new.unk_0x19F
        || old.unk_0x1A1 != new.unk_0x1A1
        || old.unk_0x1A3 != new.unk_0x1A3
        || old.unk_0x1A4 != new.unk_0x1A4
        || old.createspell != new.createspell
        || old.randtext != new.randtext;
    if !changed {
        return false;
    }
    let keep = (
        old.number,
        old.unk_0x004.clone(),
        old.curalive,
        old.killdate,
        old.killtime,
        old.dirty,
    );
    *old = new.clone();
    (
        old.number,
        old.unk_0x004,
        old.curalive,
        old.killdate,
        old.killtime,
        old.dirty,
    ) = keep;
    true
}
