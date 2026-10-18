//! Light sources: what lighting one does to the character (the light part of
//! `use_no_target`) and the pronoun of the room line. The burn on the medium tick is
//! `cycles::medium::burn_light` (T-037); the room light level is `display::light` (T-025).

use silvermere_types::enums::Ability;
use silvermere_types::ids::SessionId;
use silvermere_types::records::Item;

use crate::world::World;

/// The possessive pronoun by `player.sex`: "her", "his", else "its".
pub fn possessive(sex: u8) -> &'static str {
    match sex {
        b'F' => "her",
        b'M' => "his",
        _ => "its",
    }
}

/// The light abilities of `use_no_target`, slot by slot: "Alter General
/// Light" adds to the player's light, its Duration to `lighttime`, and "Auto Use
/// Abilities" ends the list for an item used by hand. Returns whether anything happened.
///
/// TODO(T-053): the rest of `use_no_target` (heal, mana, scrolls, one-time casts, keys)
/// runs here once item use lands; a light source in the stock data has none of them.
pub fn use_light(world: &mut World, id: SessionId, itm: &Item) -> bool {
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return false;
    };
    let mut rc = false;
    for &(ability, value) in &itm.abilities {
        match ability {
            Ability::AutoUse => break,
            Ability::GenLight => {
                plr.light = plr.light.wrapping_add(value);
                rc = true;
            }
            Ability::GenLightTime => {
                plr.lighttime = plr.lighttime.wrapping_add(value);
                rc = true;
            }
            _ => {}
        }
    }
    rc
}
