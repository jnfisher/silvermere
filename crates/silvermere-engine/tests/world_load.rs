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

/// What play changes in the content records stays as the engine has it across a reload,
/// since the database may be behind (`POST /v1/world-hold`): a limited item's copies in
/// play, a monster's alive count and last kill, and a gang shop's slots and markup. The
/// rest of each record comes from the database.
#[test]
fn reload_content_keeps_what_play_changed_in_the_content_records() {
    use silvermere_types::enums::ShopType;
    use silvermere_types::ids::{ItemNo, ShopNo};
    let mut s = common::tiny_snapshot();
    s.shops.push(silvermere_types::records::Shop {
        number: ShopNo(7),
        name: "Gang shop".into(),
        desc: Default::default(),
        shop_type: ShopType::GangShop,
        minlevel: 0,
        maxlevel: 0,
        markup: 0,
        ownable: 0,
        expandable: 0,
        trainclass: silvermere_types::ids::ClassNo(0),
        stock: std::array::from_fn(|_| silvermere_types::records::ShopSlot {
            item: ItemNo(0),
            maxqty: 0,
            qty: 0,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        }),
    });
    let mut world = World::load(GameConfig::default(), 4, s.clone()).unwrap();
    let item = *world.items.keys().next().unwrap();
    let monster = *world.monsters.keys().next().unwrap();
    world.items.get_mut(&item).unwrap().active = 3;
    let m = world.monsters.get_mut(&monster).unwrap();
    m.curalive = 2;
    m.killed_at = 1_700_000_000;
    let g = world.shops.get_mut(&ShopNo(7)).unwrap();
    g.markup = 150;
    g.stock[0].item = ItemNo(2);
    g.stock[0].maxqty = 42;

    let mut edited = s;
    let i = edited.items.iter_mut().find(|i| i.number == item).unwrap();
    i.name = "edited item".into();
    i.active = 0;
    let m = edited
        .monsters
        .iter_mut()
        .find(|m| m.number == monster)
        .unwrap();
    m.name = "edited monster".into();
    m.curalive = 0;
    edited.shops.last_mut().unwrap().name = "Edited gang shop".into();
    world.reload_content(&edited).unwrap();

    assert_eq!(world.items[&item].name, "edited item");
    assert_eq!(world.items[&item].active, 3);
    assert_eq!(world.monsters[&monster].name, "edited monster");
    assert_eq!(
        (
            world.monsters[&monster].curalive,
            world.monsters[&monster].killed_at
        ),
        (2, 1_700_000_000)
    );
    let g = &world.shops[&ShopNo(7)];
    assert_eq!(g.name, "Edited gang shop");
    assert_eq!(
        (g.markup, g.stock[0].item, g.stock[0].maxqty),
        (150, ItemNo(2), 42)
    );
}
