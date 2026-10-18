//! T-076: the start-up room pass on the imported stock world: the
//! specific monsters of the Sealed Tomb, the Large Chamber, the Large Tomb and the Pier
//! (rooms 1309, 2224, 2236, 1813: known monsters 31, 82, 85, 61 in the world export) are
//! alive after a cold start, and the lairs and shops are filled.

mod common;

use common::*;
use silvermere_types::enums::RoomType;
use silvermere_types::ids::{MonsterNo, RoomKey};

#[tokio::test(flavor = "multi_thread")]
async fn a_cold_start_makes_the_specific_monsters_and_fills_the_lairs() {
    let db = stock_db();
    let r = start_as_served(&db.url, 76).await;
    let (specs, total, lairs, filled, arena) = on_world(&r, |w| {
        let specs: Vec<(u32, bool, Vec<MonsterNo>)> = [1309, 2224, 2236, 1813]
            .into_iter()
            .map(|n| {
                let k = RoomKey::new(1, n);
                let st = &w.room_state[&k];
                let here = st
                    .monsters
                    .iter()
                    .filter(|m| m.0 != 0)
                    .filter_map(|m| w.active_monsters.by_number(*m))
                    .map(|m| m.knownnum)
                    .collect();
                (n, st.specalive, here)
            })
            .collect();
        let total = w.active_monsters.values().count();
        let lairs: Vec<RoomKey> = w
            .rooms
            .values()
            .filter(|r| r.room_type == RoomType::Lair)
            .map(|r| r.key)
            .collect();
        let filled = lairs
            .iter()
            .filter(|k| w.room_state[k].nummonsters > 0)
            .count();
        let arena = w.room_state[&RoomKey::new(1, 2150)]
            .monsters
            .iter()
            .filter(|m| m.0 != 0)
            .count();
        (specs, total, lairs.len(), filled, arena)
    })
    .await;
    let want = [(1309, 31), (2224, 82), (2236, 85), (1813, 61)];
    for ((n, alive, here), (wn, mon)) in specs.into_iter().zip(want) {
        assert_eq!(n, wn);
        assert!(alive, "room {n}: RF_SPECALIVE");
        assert!(
            here.contains(&MonsterNo(mon)),
            "room {n} holds known monster {mon}: {here:?}"
        );
    }
    eprintln!(
        "active monsters after start-up: {total}; lairs {lairs}, filled {filled}; arena {arena}"
    );
    // T-040's live runs saw the original hold about 7,400 active monsters after start-up
    // (silvermere before this pass: 7); the stock world makes 7,378 here
    assert!((7_000..7_800).contains(&total), "{total} active monsters");
    // some lairs have an empty pool (no `maxlevel`, or no known monster in range)
    assert!(filled > 2_500, "{filled} of {lairs} lairs filled");
    // the Arena is no Lair or Shop: nothing is made there at start-up
    assert_eq!(arena, 0);
    r.stop().await.unwrap();
}
