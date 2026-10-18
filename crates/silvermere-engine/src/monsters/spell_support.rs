//! What the monster casting engines share: the lines of a monster's successful cast
//! ([`monster_display_spell_success`]) and the lasting add on a player that shows them
//! ([`monster_add_cast_spell_to_user`]). Used by the area cast (T-051's
//! [`super::area_cast`]) and the cast at a player (T-052's [`super::cast`]).

use silvermere_rules::abilities::spell_ability_value;
use silvermere_rules::spells::{CharacterSlots, SlotOutcome, monster_add_spell_to_character};
use silvermere_text::Msg;
use silvermere_types::enums::Ability;
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{ActiveMonsterId, MessageNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::Spell;

use crate::combat;
use crate::movement::exits::{Arg, fmt_msg};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// The built-in texts when the spell has no WCCMSG record: the target's and the room's.
pub const DEFAULT_MONSTER_TEXTS: [&str; 2] = ["%s cast %s on you.", "%s cast %s on %s."];

/// The target's and the room's text of one monster cast, each line ending in `\r`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonsterSpellMsg {
    /// `None` when no player was hit.
    pub target: Option<String>,
    pub room: String,
}

fn texts_of(world: &World, n: i32) -> Option<[String; 3]> {
    if n == 0 {
        return None;
    }
    world.messages.get(&MessageNo(n)).map(|m| m.msgs.clone())
}

fn upper_first(mut s: String) -> String {
    if let Some(c) = s.get(..1) {
        let up = c.to_ascii_uppercase();
        s.replace_range(..1, &up);
    }
    s
}

/// The texts of [`monster_display_spell_success`] for a monster named `monster`.
///
/// The target's: the "Starting Message" record's second line with the monster's name, then
/// the WCCMSG target line (for an `SPF_ALTMSG` spell given the damage only, its format's
/// first letter raised; else given the monster's name, the spell's name and the damage, the
/// result's first letter raised), then the "Descriptive Message" record's third line. The
/// room's: the starting record's third line with the monster's and the target's names, then
/// the WCCMSG room line (alternate: the target's name, the damage word and two `*`; else
/// the monster's name, the spell's name, the target's name and the damage word), its first
/// letter raised. The damage word is the one describing the amount, the amount through
/// `%d`.
pub fn monster_spell_texts(
    world: &World,
    spell: &Spell,
    monster: &str,
    tgtname: &str,
    damage: i32,
    target_is_player: bool,
) -> MonsterSpellMsg {
    let (tgtmsg, roommsg) = match texts_of(world, spell.msgnum.0) {
        Some([_, t, r]) => (t, r),
        None => (
            DEFAULT_MONSTER_TEXTS[0].to_owned(),
            DEFAULT_MONSTER_TEXTS[1].to_owned(),
        ),
    };
    let desc = texts_of(world, spell_ability_value(spell, Ability::DescMsg));
    let start = texts_of(world, spell_ability_value(spell, Ability::StartMsg));
    let alt = spell.flags.contains(SpellFlags::ALTMSG);
    let word = damage.to_string();
    let target = target_is_player.then(|| {
        let mut s = String::new();
        if let Some(m) = &start
            && !m[1].is_empty()
        {
            s.push_str(&fmt_msg(&m[1], &[Arg::S(monster)]));
            s.push('\r');
        }
        if !tgtmsg.is_empty() {
            let buf = format!("{tgtmsg}\r");
            if alt {
                s.push_str(&fmt_msg(&upper_first(buf), &[Arg::D(i64::from(damage))]));
            } else {
                s.push_str(&upper_first(fmt_msg(
                    &buf,
                    &[
                        Arg::S(monster),
                        Arg::S(&spell.name),
                        Arg::D(i64::from(damage)),
                    ],
                )));
            }
        }
        if let Some(d) = &desc
            && !d[2].is_empty()
        {
            s.push_str(&d[2]);
            s.push('\r');
        }
        s
    });
    let mut room = String::new();
    if let Some(m) = &start
        && !m[2].is_empty()
    {
        room.push_str(&fmt_msg(&m[2], &[Arg::S(monster), Arg::S(tgtname)]));
        room.push('\r');
    }
    if !roommsg.is_empty() {
        let buf = format!("{roommsg}\r");
        let line = if alt {
            fmt_msg(
                &buf,
                &[Arg::S(tgtname), Arg::S(&word), Arg::S("*"), Arg::S("*")],
            )
        } else {
            fmt_msg(
                &buf,
                &[
                    Arg::S(monster),
                    Arg::S(&spell.name),
                    Arg::S(tgtname),
                    Arg::S(&word),
                ],
            )
        };
        room.push_str(&upper_first(line));
    }
    MonsterSpellMsg { target, room }
}

/// The lines of a monster's successful cast: the target's lines in light red with his
/// prompt (when a player was hit), then the room's lines in light red to everyone in the
/// monster's room but the target, blind players too, each with his prompt and no colour
/// row.
pub fn monster_display_spell_success(
    world: &mut World,
    target: Option<SessionId>,
    spell: &Spell,
    mon: ActiveMonsterId,
    tgtname: &str,
    damage: i32,
) {
    let Some(m) = combat::monster(world, mon) else {
        return;
    };
    let (name, room) = (
        m.name.clone(),
        RoomKey {
            map: m.map,
            room: m.room,
        },
    );
    let texts = monster_spell_texts(world, spell, &name, tgtname, damage, target.is_some());
    if let (Some(t), Some(text)) = (target, texts.target)
        && let Some(s) = world.session_mut(t)
    {
        s.output.push(Msg::MonsterSpellLine { text });
        s.push_prompt();
    }
    tell_room(
        world,
        room,
        [None, target],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::MonsterSpellLine { text: texts.room },
    );
}

/// A monster's lasting spell on a player: the spell goes into the player's slots (renewed
/// only when `extend` and the new value is larger, else refused), and a success shows the
/// cast with the player as the target and `effect` as the amount.
pub fn monster_add_cast_spell_to_user(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    duration: i32,
    effect: i32,
    mon: ActiveMonsterId,
    extend: bool,
) -> SlotOutcome {
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return SlotOutcome::Full;
    };
    let Some(c) = combat::player_mut(world, id) else {
        return SlotOutcome::Full;
    };
    let out = monster_add_spell_to_character(
        CharacterSlots {
            spells: &mut c.spells,
            spellvals: &mut c.spellvals,
            spellrnds: &mut c.spellrnds,
        },
        spl.number,
        duration,
        effect,
        extend,
    );
    if out.is_success() {
        let name = c.firstname.clone();
        monster_display_spell_success(world, Some(id), &spl, mon, &name, effect);
    }
    out
}
