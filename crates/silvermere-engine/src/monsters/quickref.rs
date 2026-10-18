//! The known-monster quick reference: every known monster's number, type and generation
//! index in number order, what `generate_monster`'s pool pick walks.
//!
//! Built from `World::monsters` at load and on every content reload; no 2000-entry
//! cap.

use silvermere_rules::spawn::QuickRef;

use crate::world::World;

/// The quick reference of the known monsters, in number order.
pub fn build(world: &World) -> Vec<QuickRef> {
    let mut v: Vec<QuickRef> = world
        .monsters
        .values()
        .map(|m| QuickRef {
            number: m.number,
            montype: m.montype.code() as i16,
            genindex: m.genindex,
        })
        .collect();
    v.sort_unstable_by_key(|q| q.number);
    v
}

/// Rebuilds `world.monqref`.
pub fn rebuild(world: &mut World) {
    world.monqref = build(world);
}
