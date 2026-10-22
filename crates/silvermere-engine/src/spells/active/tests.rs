//! `spells::active`: the confusion checks of a player and of a monster, with and without a
//! Confuse Message.

use silvermere_text::strip_ansi;
use silvermere_types::enums::{Ability, SpellTarget, SpellType};
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ItemNo, MessageNo, RaceNo, SpellNo};
use silvermere_types::records::{ActiveSpell, Message};

use super::*;
use crate::spells::test_fixture::{self as fx, RAT, ROOM, enter, plr, spawn};

/// The quest boss's death spell as the stock data has it ("dread mystic temp"): the End Cast
/// text spell, Confusion 100, Confuse Message 66, Non-magical.
const TEMP: SpellNo = SpellNo(603);
/// The record the death spells name: all three texts empty.
const BLANK: MessageNo = MessageNo(66);
/// A spell with an ability that does nothing here, to fill the slots ahead of [`TEMP`].
const FILLER: SpellNo = SpellNo(700);

fn temp_spell(msgnum: i16) -> Spell {
    fx::spell(
        TEMP.0,
        "dread mystic temp",
        SpellType::General,
        SpellTarget::from_code(12),
        (0, 0),
        1,
        66,
        &[
            (Ability::EndCast, 604),
            (Ability::Confusion, 100),
            (Ability::ConfuseMsg, msgnum),
            (Ability::NonMagical, 0),
        ],
    )
}

/// The fixture world with the death spell, the filler spell and the blank record.
fn world(msgnum: i16) -> World {
    let mut w = fx::world();
    w.spells.insert(TEMP, temp_spell(msgnum));
    w.spells.insert(
        FILLER,
        fx::spell(
            FILLER.0,
            "filler",
            SpellType::General,
            SpellTarget::from_code(1),
            (0, 0),
            10,
            0,
            &[(Ability::NonMagical, 0)],
        ),
    );
    w.messages.insert(
        BLANK,
        Message {
            number: BLANK,
            msgs: [String::new(), String::new(), String::new()],
        },
    );
    w
}

/// Puts `spell` in the character's spell slot `slot`, the slots ahead of it holding the
/// filler, and marks the character confused.
fn cast_on(w: &mut World, id: SessionId, spell: SpellNo, slot: usize) {
    let c = plr(w, id);
    for i in 0..slot {
        c.spells[i] = FILLER;
    }
    c.spells[slot] = spell;
    c.status |= PlayerStatus::CONFUSED;
}

/// Everything written to `id` since the last call, ANSI off, with the blank lines kept and
/// the prompts dropped.
fn raw(w: &mut World, id: SessionId) -> Vec<String> {
    let prompt = crate::tell::prompt_bytes(w, id);
    let palette = crate::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    let text = String::from_utf8_lossy(&strip_ansi(&b)).into_owned();
    let mut out: Vec<String> = text
        .split("\r\n")
        .map(|l| l.split("[HP=").next().unwrap().trim().to_owned())
        .collect();
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

fn stupid(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("stupidly"))
}

/// Two players in the room, the first under `spell` in slot `slot`; the confusion check of
/// the first, then what each saw and the first one's delay.
fn confuse(w: &mut World, slot: usize) -> (bool, Vec<String>, Vec<String>, u8) {
    let id = enter(w, "Aldric", ROOM);
    let other = enter(w, "Bob", ROOM);
    cast_on(w, id, TEMP, slot);
    raw(w, id);
    raw(w, other);
    let lost = check_confusion(w, id);
    let delay = plr(w, id).delay;
    (lost, raw(w, id), raw(w, other), delay)
}

// The report (public issue #3): a monster's death spell takes the character's last free
// slot (9), where its Confuse Message was not found, so the stock lines were printed. Every
// slot gives the record's (empty) texts.
#[test]
fn a_spells_confuse_message_is_used_in_every_slot() {
    for slot in 0..NSPELLCAST {
        let mut w = world(BLANK.0 as i16);
        let (lost, you, room, delay) = confuse(&mut w, slot);
        assert!(lost, "slot {slot}");
        assert!(!stupid(&you), "slot {slot}: {you:?}");
        assert!(!stupid(&room), "slot {slot}: {room:?}");
        assert!(you.iter().any(String::is_empty), "slot {slot}: {you:?}");
        assert!(room.iter().any(String::is_empty), "slot {slot}: {room:?}");
        assert_eq!(delay, 1, "slot {slot}");
    }
}

#[test]
fn the_records_texts_go_to_the_player_and_the_room() {
    let mut w = world(BLANK.0 as i16);
    w.messages.get_mut(&BLANK).unwrap().msgs =
        ["Your mind reels.".into(), "%s reels.".into(), String::new()];
    let (lost, you, room, _) = confuse(&mut w, 6);
    assert!(lost);
    assert!(you.iter().any(|l| l == "Your mind reels."), "{you:?}");
    assert!(room.iter().any(|l| l == "Aldric reels."), "{room:?}");
    assert!(!stupid(&you) && !stupid(&room));
}

#[test]
fn no_confuse_message_gives_the_stock_lines() {
    let mut w = world(0);
    let s = w.spells.get_mut(&TEMP).unwrap();
    s.abilities[2] = (Ability::None, 0);
    let (lost, you, room, delay) = confuse(&mut w, 6);
    assert!(lost);
    assert!(
        you.iter()
            .any(|l| l == "You look around stupidly and do nothing!"),
        "{you:?}"
    );
    assert!(
        room.iter()
            .any(|l| l == "Aldric looks around stupidly and foams at the mouth!"),
        "{room:?}"
    );
    assert_eq!(delay, 1);
}

#[test]
fn a_missing_record_gives_the_stock_lines() {
    let mut w = world(9999);
    let (lost, you, room, delay) = confuse(&mut w, 6);
    assert!(lost);
    assert!(stupid(&you), "{you:?}");
    assert!(stupid(&room), "{room:?}");
    assert_eq!(delay, 1);
}

// Confusion from the spell, the Confuse Message from the race, an item or a class: the one
// pass over the sources finds it all the same.
#[test]
fn a_confuse_message_from_the_race_or_an_item_is_used() {
    for from in ["race", "item"] {
        let mut w = world(0);
        w.spells.get_mut(&TEMP).unwrap().abilities[2] = (Ability::None, 0);
        let id = enter(&mut w, "Aldric", ROOM);
        let other = enter(&mut w, "Bob", ROOM);
        match from {
            "race" => {
                w.races.get_mut(&RaceNo(1)).unwrap().abilities[0] = (Ability::ConfuseMsg, 66);
            }
            _ => {
                let item = w.items.get_mut(&ItemNo(1)).unwrap();
                item.abilities[0] = (Ability::ConfuseMsg, 66);
                plr(&mut w, id).worn[0] = ItemNo(1);
            }
        }
        cast_on(&mut w, id, TEMP, 7);
        raw(&mut w, id);
        raw(&mut w, other);
        assert!(check_confusion(&mut w, id), "{from}");
        let (you, room) = (raw(&mut w, id), raw(&mut w, other));
        assert!(!stupid(&you) && !stupid(&room), "{from}: {you:?} {room:?}");
    }
}

#[test]
fn without_the_confused_flag_nothing_happens() {
    let mut w = world(BLANK.0 as i16);
    let id = enter(&mut w, "Aldric", ROOM);
    cast_on(&mut w, id, TEMP, 0);
    plr(&mut w, id).status.remove(PlayerStatus::CONFUSED);
    assert!(!check_confusion(&mut w, id));
    assert_eq!(plr(&mut w, id).delay, 0);
}

// The monster check reads the Confusion and the Confuse Message totals of every source on
// the monster (a spell cast on it here, in its last slot).
fn confuse_monster(msgnum: i16) -> Vec<String> {
    let mut w = world(msgnum);
    let watcher = enter(&mut w, "Bob", ROOM);
    let m = spawn(&mut w, RAT, ROOM);
    let mon = w.active_monsters.by_number_mut(m).unwrap();
    mon.spells[NMONSPELLS - 1] = ActiveSpell {
        spell: TEMP,
        value: 0,
        rounds: 1,
    };
    raw(&mut w, watcher);
    assert!(check_monster_confusion(&mut w, m));
    raw(&mut w, watcher)
}

#[test]
fn a_monsters_confuse_message_is_used() {
    let room = confuse_monster(BLANK.0 as i16);
    assert!(!stupid(&room), "{room:?}");
    assert!(room.iter().any(String::is_empty), "{room:?}");
}

#[test]
fn a_monster_with_a_missing_record_gets_the_stock_line() {
    let room = confuse_monster(9999);
    assert!(
        room.iter()
            .any(|l| l.ends_with("looks around stupidly and foams at the mouth!")),
        "{room:?}"
    );
}
