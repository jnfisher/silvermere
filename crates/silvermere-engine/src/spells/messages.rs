//! A spell's success: the caster's, the target's and the room's lines of a spell that took
//! hold, from the spell's WCCMSG triple (`spell.msgnum`, `msgs[0..3]`) or the built-in
//! texts, with the optional "Starting Message" and "Descriptive Message" records the
//! spell's abilities name.
//!
//! [`SpellMsg`] is the three texts of one cast, built by [`spell_success_texts`] without
//! touching a session; [`display_spell_success`] delivers them with the original's
//! once-per-cast rule for the caster's and the room's lines ([`Told`], fresh at the start
//! of a cast).

use silvermere_rules::abilities::spell_ability_value;
use silvermere_text::Msg;
use silvermere_text::colour::ColRow;
use silvermere_types::enums::{Ability, SpellTarget, SpellType};
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{MessageNo, SessionId};
use silvermere_types::records::Spell;

use crate::movement::exits::{Arg, fmt_msg};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// The built-in texts when the spell has no WCCMSG record.
pub const DEFAULT_TEXTS: [&str; 3] = [
    "You cast %s on %s.",
    "%s cast %s on you.",
    "%s cast %s on %s.",
];

/// The caster's and the room's line of a cast have gone out. A fresh value is the start of
/// a cast.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Told {
    pub caster: bool,
    pub room: bool,
}

/// The three points of view of a spell that took hold, as text (each ends in `\r`, the
/// starting line, when there is one, comes first with its own `\r`). `None` where the
/// original printed nothing: an empty WCCMSG text, or the target's line when the caster
/// is the target (only the descriptive line is left then, and it may be empty too).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpellMsg {
    pub caster: Option<String>,
    pub target: Option<String>,
    pub room: Option<String>,
}

/// The colour row of a spell's lines: `COL_SPELLGEN` for a General spell, `COL_SPELLOTH`
/// for the others.
pub fn spell_colour(spell: &Spell) -> ColRow {
    if spell.spell_type == SpellType::General {
        ColRow::SpellGen
    } else {
        ColRow::SpellOth
    }
}

/// The area kinds whose texts leave the target name out. Note that "Divided Area Incl. You"
/// is not among them, as the original's texts show.
pub fn names_no_target(target: SpellTarget) -> bool {
    matches!(
        target,
        SpellTarget::DivArea
            | SpellTarget::DivAttack
            | SpellTarget::DivParty
            | SpellTarget::FullArea
            | SpellTarget::FullAttack
            | SpellTarget::FullParty
    )
}

fn texts_of(world: &World, n: MessageNo) -> Option<[String; 3]> {
    if n == MessageNo(0) {
        return None;
    }
    world.messages.get(&n).map(|m| m.msgs.clone())
}

/// The texts of [`display_spell_success`] for a caster named `caster`. `target_is_caster`
/// when the caster is the target; `target_is_player` when the target is a player (a monster
/// or no target gets no target line).
pub fn spell_success_texts(
    world: &World,
    spell: &Spell,
    caster: &str,
    targetname: &str,
    amount: i32,
    target_is_player: bool,
    target_is_caster: bool,
) -> SpellMsg {
    let [castermsg, targetmsg, roommsg] =
        texts_of(world, spell.msgnum).unwrap_or_else(|| DEFAULT_TEXTS.map(str::to_owned));
    let area = names_no_target(spell.target);
    let tname = if area { "" } else { targetname };
    let descmsg = texts_of(
        world,
        MessageNo(spell_ability_value(spell, Ability::DescMsg)),
    );
    let startmsg = texts_of(
        world,
        MessageNo(spell_ability_value(spell, Ability::StartMsg)),
    );
    let start = |i: usize, args: &[Arg<'_>]| -> String {
        match &startmsg {
            Some(m) if !m[i].is_empty() => format!("{}\r", fmt_msg(&m[i], args)),
            _ => String::new(),
        }
    };
    let alt = spell.flags.contains(SpellFlags::ALTMSG);
    let amount_d = Arg::D(i64::from(amount));
    // the damage descriptor: the amount through "%d"
    let desc = amount.to_string();

    let caster_line = (!castermsg.is_empty()).then(|| {
        let body = if alt {
            fmt_msg(&castermsg, &[Arg::S(tname), amount_d])
        } else {
            fmt_msg(&castermsg, &[Arg::S(&spell.name), Arg::S(tname), amount_d])
        };
        format!("{}{body}\r", start(0, &[Arg::S(tname)]))
    });

    let target_line = target_is_player.then(|| {
        let mut s = String::new();
        if !target_is_caster {
            s.push_str(&start(1, &[Arg::S(caster)]));
            if !targetmsg.is_empty() {
                let body = if alt {
                    fmt_msg(&targetmsg, &[amount_d])
                } else {
                    fmt_msg(&targetmsg, &[Arg::S(caster), Arg::S(&spell.name), amount_d])
                };
                s.push_str(&body);
                s.push('\r');
            }
        }
        if let Some(d) = &descmsg
            && !d[2].is_empty()
        {
            s.push_str(&d[2]);
            s.push('\r');
        }
        s
    });

    let room_line = (!roommsg.is_empty()).then(|| {
        let body = if alt {
            fmt_msg(&roommsg, &[Arg::S(tname), Arg::S(&desc)])
        } else {
            fmt_msg(
                &roommsg,
                &[
                    Arg::S(caster),
                    Arg::S(&spell.name),
                    Arg::S(tname),
                    Arg::S(&desc),
                ],
            )
        };
        format!("{}{body}\r", start(2, &[Arg::S(caster), Arg::S(tname)]))
    });

    SpellMsg {
        caster: caster_line,
        target: target_line,
        room: room_line,
    }
}

/// The caster's or the target's line as a message, in the spell type's colour.
pub fn coloured_line(spell: &Spell, text: String) -> Msg {
    match spell_colour(spell) {
        ColRow::SpellGen => Msg::SpellLineGen { text },
        _ => Msg::SpellLineOth { text },
    }
}

/// A spell's success shown. `caster` is the casting player (a monster or room caster has
/// none and nothing is shown); `target` the target player, `None` for a monster, an item or
/// nobody. The caster's line goes out once per cast (and only when it is not empty), with
/// his prompt; the target's line every time, with his prompt unless he is the caster; the
/// room's line once per cast, to everyone in the caster's room but the caster and the
/// target, blind players too, each with his prompt, in the colour row of the spell type.
pub fn display_spell_success(
    world: &mut World,
    told: &mut Told,
    caster: SessionId,
    target: Option<SessionId>,
    spell: &Spell,
    targetname: &str,
    amount: i32,
) {
    let Some(plr) = world.session(caster).and_then(|s| s.player()) else {
        return;
    };
    let name = plr.firstname.clone();
    let room = crate::combat::room_of(plr);
    let texts = spell_success_texts(
        world,
        spell,
        &name,
        targetname,
        amount,
        target.is_some(),
        target == Some(caster),
    );
    if !told.caster
        && let Some(text) = texts.caster
    {
        told.caster = true;
        if let Some(s) = world.session_mut(caster) {
            s.output.push(coloured_line(spell, text));
            s.push_prompt();
        }
    }
    if let Some(t) = target
        && let Some(text) = texts.target
        && world.session(t).and_then(|s| s.player()).is_some()
        && let Some(s) = world.session_mut(t)
    {
        s.output.push(coloured_line(spell, text));
        if t != caster {
            s.push_prompt();
        }
    }
    if !told.room
        && let Some(text) = texts.room
    {
        told.room = true;
        tell_room(
            world,
            room,
            [Some(caster), target],
            Noise::Always,
            TellBlind::All,
            true,
            Some(spell_colour(spell) as u8),
            &Msg::SpellLineRoom { text },
        );
    }
}
