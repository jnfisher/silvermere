//! Light sources: the pronoun of the room line when one is lit (lighting runs the item's
//! `use_no_target`). The burn on the medium tick is
//! `cycles::medium::burn_light` (T-037); the room light level is `display::light` (T-025).

/// The possessive pronoun by `player.sex`: "her", "his", else "its".
pub fn possessive(sex: u8) -> &'static str {
    match sex {
        b'F' => "her",
        b'M' => "his",
        _ => "its",
    }
}
