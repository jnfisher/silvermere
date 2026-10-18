//! The guards of the `use_*_target` functions: a missing user, target or item does nothing.
//! The behaviour with a player in the game is tested through the engine in
//! `silvermere-testkit/tests/stock_item_use.rs`.

use silvermere_types::config::GameConfig;
use silvermere_types::enums::{Direction, ItemType};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, RoomKey, SessionId, SpellNo};

use super::*;
use crate::world::{World, WorldSnapshot, blank};

const ELIXIR: ItemNo = ItemNo(10);

fn world() -> World {
    let mut s = WorldSnapshot::default();
    let g = GameConfig::default();
    for n in [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room] {
        s.rooms.push(blank::clear_room(RoomKey::new(1, n)));
    }
    let mut elixir = blank::clear_item(ELIXIR);
    elixir.name = "elixir".into();
    elixir.item_type = ItemType::Drink;
    elixir.abilities[0] = (Ability::Heal, 10);
    s.items.push(elixir);
    World::load(GameConfig::default(), 4, s).expect("world loads")
}

#[test]
fn nothing_without_a_user_or_an_item() {
    let mut w = world();
    let nobody = SessionId(3);
    let itm = w.items.get(&ELIXIR).unwrap().clone();
    assert!(!use_no_target(&mut w, nobody, &itm, None));
    assert!(!use_no_target(&mut w, nobody, &itm, Some(Direction::North)));
    assert_eq!(use_item(&mut w, nobody, ItemNo(999), UseTarget::None), 0);
    assert_eq!(
        use_player_target(&mut w, nobody, &itm, nobody, Via::Hand),
        0
    );
    assert_eq!(
        use_monster_target(&mut w, nobody, &itm, ActiveMonsterId(1), Via::AutoUse),
        0
    );
    assert_eq!(use_item_target(&mut w, nobody, &itm, ItemNo(999)), 0);
    assert_eq!(use_spell_target(&mut w, nobody, &itm, SpellNo(1)), 0);
    assert_eq!(
        auto_use_on_hit(
            &mut w,
            nobody,
            ItemNo(999),
            Combatant::Monster(ActiveMonsterId(1))
        ),
        0
    );
}
