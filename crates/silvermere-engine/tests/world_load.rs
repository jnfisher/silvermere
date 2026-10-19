//! `World::load` over the tiny fixture world and `reload_content`.

use crate::common;

use silvermere_engine::{World, WorldError, WorldSnapshot};
use silvermere_types::config::GameConfig;
use silvermere_types::ids::RoomKey;

#[test]
fn load_populates_the_maps_with_the_fixture_counts() {
    let world = common::tiny_world(4);
    assert_eq!(world.rooms.len(), 4);
    assert_eq!(world.room_state.len(), 4);
    assert_eq!(world.items.len(), 2);
    assert_eq!(world.monsters.len(), 2);
    assert_eq!(world.classes.len(), 1);
    assert_eq!(world.races.len(), 1);
    assert!(world.spells.is_empty());
    assert!(world.sessions.is_empty());
    assert_eq!(world.ids.max(), 4);
    assert_eq!(
        world.scheduler.pending().count(),
        6,
        "the six periodic routines are armed"
    );
    assert_eq!(world.room(RoomKey::new(1, 2140)).unwrap().name, "Room 1");
}

#[test]
fn load_refuses_without_the_special_rooms() {
    let mut s = common::tiny_snapshot();
    s.rooms.retain(|r| r.key != RoomKey::new(1, 376));
    let err = World::load(GameConfig::default(), 4, s).err().unwrap();
    assert_eq!(
        err,
        WorldError::MissingSpecialRoom {
            name: "jail_room",
            room: 376
        }
    );
    assert_eq!(
        World::load(GameConfig::default(), 4, WorldSnapshot::default()).err(),
        Some(WorldError::NoRooms)
    );
}

#[test]
fn reload_content_changes_a_room_name_and_leaves_room_state_alone() {
    let mut world = common::tiny_world(4);
    let key = RoomKey::new(1, 2140);
    world.room_state.get_mut(&key).unwrap().coins.copper = 55;
    let before = world.room_state.clone();

    let mut s = common::tiny_snapshot();
    s.rooms.iter_mut().find(|r| r.key == key).unwrap().name = "Renamed".into();
    s.rooms[0].coins.copper = 999; // content coins are not live state once loaded
    s.items.push(silvermere_engine::world::blank::clear_item(
        silvermere_types::ids::ItemNo(3),
    ));
    world.reload_content(&s).unwrap();

    assert_eq!(world.room(key).unwrap().name, "Renamed");
    assert_eq!(world.room_state, before);
    assert_eq!(world.items.len(), 3);

    // A reload without a special room is refused and changes nothing.
    let mut bad = common::tiny_snapshot();
    bad.rooms.remove(0);
    assert!(world.reload_content(&bad).is_err());
    assert_eq!(world.room(key).unwrap().name, "Renamed");
    assert_eq!(world.rooms.len(), 4);
}

/// At a cold start each placed item that is not lying in its room gets
/// a new copy: in view when it can be picked up, hidden when not; one already there is
/// not doubled.
#[test]
fn load_puts_the_placed_items_back() {
    use silvermere_types::ids::ItemNo;
    let mut s = common::tiny_snapshot();
    s.items[0].gettable = 1;
    s.items[0].charges = 3;
    s.items[1].gettable = 0;
    let room = RoomKey::new(1, 2140);
    let r = s.rooms.iter_mut().find(|r| r.key == room).unwrap();
    r.placed[0] = ItemNo(1);
    r.placed[1] = ItemNo(2);
    let other = s.rooms.iter_mut().find(|r| r.key != room).unwrap();
    let other_key = other.key;
    other.placed[0] = ItemNo(1);
    other.items[0].item = ItemNo(1);
    other.items[0].count = 1;
    let top = common::special_rooms().into_iter().max().unwrap();
    common::dense_map1(&mut s, top);
    let world = World::load(GameConfig::default(), 4, s).unwrap();
    let st = &world.room_state[&room];
    let visible: Vec<_> = st.items.iter().filter(|i| i.item.0 != 0).collect();
    let hidden: Vec<_> = st.hiditems.iter().filter(|i| i.item.0 != 0).collect();
    assert_eq!(visible.len(), 1);
    assert_eq!((visible[0].item, visible[0].uses), (ItemNo(1), 3));
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].item, ItemNo(2));
    let there = world.room_state[&other_key]
        .items
        .iter()
        .filter(|i| i.item == ItemNo(1))
        .count();
    assert_eq!(there, 1, "an item already lying there is not doubled");
}
