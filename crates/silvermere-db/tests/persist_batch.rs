//! T-113: the worker writes a batch kind by kind in multi-row statements. What it stores
//! must be what applying the same writes one at a time with the per-row writers (the path
//! before T-113) stores, on either backend.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use silvermere_db::repo::{MonsterState, RoomState, ShopStock};
use silvermere_db::{Db, Event, Persist, PersistConfig, TestDb, Write};
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, CharacterId, ItemNo, MonsterNo, RoomKey, ShopNo,
};
use silvermere_types::records::{Coins, RoomItem};

const ROOMS: u32 = 20;
const MONSTERS: i32 = 10;
const ITEMS: i32 = 10;
const SHOPS: i32 = 5;

/// A small linear congruential generator: the same writes for both databases.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n)) as u32
    }
}

/// The same content, an account and its character in each database.
async fn seed(db: &Db) -> (AccountId, Vec<CharacterId>) {
    let mut seq = Seq::new(113);
    for n in 1..=ROOMS {
        db.rooms().insert(&room(&mut seq, 1, n)).await.unwrap();
    }
    for n in 1..=MONSTERS {
        db.monsters().insert(&monster(&mut seq, n)).await.unwrap();
    }
    for n in 1..=ITEMS {
        db.items().insert(&item(&mut seq, n)).await.unwrap();
    }
    for n in 1..=SHOPS {
        db.shops().insert(&shop(&mut seq, n)).await.unwrap();
    }
    let acct = new_account(db, &mut seq, "wendy").await;
    let c = character(&mut seq, "wendy", "Wendy");
    let mut ids = vec![db.characters().insert(acct, &c).await.unwrap()];
    // two more players, so a batch saves several characters at once
    for name in ["Xena", "Yuri"] {
        let other = new_account(db, &mut seq, &name.to_lowercase()).await;
        let c = character(&mut seq, &name.to_lowercase(), name);
        ids.push(db.characters().insert(other, &c).await.unwrap());
    }
    (acct, ids)
}

/// A mixed stream of writes, with the orderings that matter: an active monster
/// inserted, saved, removed and its number reused; an ownership record inserted and
/// deleted; evil points and bank books put and deleted; a room written many times.
/// With `refusals` off, no insert meets an existing row, so no batch falls back to one
/// write at a time and the multi-row path writes everything.
fn writes(acct: AccountId, cids: &[CharacterId], refusals: bool) -> Vec<Write> {
    let mut seq = Seq::new(7);
    let mut r = Lcg(113);
    let names = ["Wendy", "Xena", "Yuri"];
    let mut out = Vec::new();
    for i in 0..600u32 {
        let w = match r.below(16) {
            0 | 1 => {
                let mut s = RoomState::empty(RoomKey::new(1, r.below(ROOMS) + 1));
                s.coins = Coins {
                    copper: i,
                    ..Coins::default()
                };
                for slot in 0..r.below(4) as usize {
                    s.items[slot] = RoomItem {
                        item: ItemNo(r.below(50) as i32 + 1),
                        uses: r.below(9) as i16,
                        count: 1,
                    };
                }
                s.monsters[0] = ActiveMonsterId(r.below(30) as i32);
                Write::RoomState(s)
            }
            2 => Write::MonsterState(MonsterState {
                number: MonsterNo(r.below(MONSTERS as u32) as i32 + 1),
                curalive: r.below(5) as u16,
                killed_at: i64::from(i),
            }),
            3 => Write::ItemActive {
                item: ItemNo(r.below(ITEMS as u32) as i32 + 1),
                active: i as u16,
            },
            4 if refusals => {
                Write::ActiveMonsterInsert(active_monster(&mut seq, r.below(30) as i32 + 1))
            }
            // numbers 101.. inserted once each, some later saved or removed
            4 => Write::ActiveMonsterInsert(active_monster(&mut seq, 100 + i as i32)),
            5 | 6 => Write::ActiveMonsterUpdate(active_monster(&mut seq, r.below(30) as i32 + 1)),
            7 if refusals => Write::ActiveMonsterDelete(ActiveMonsterId(r.below(30) as i32 + 1)),
            7 => Write::ActiveMonsterDelete(ActiveMonsterId(match r.below(2) {
                0 => r.below(30) as i32 + 1,
                _ => 100 + r.below(i + 1) as i32,
            })),
            // a save of a character with no row: refused, as one update of it would be
            8 if refusals && r.below(10) == 0 => Write::Character {
                id: CharacterId(9_999),
                character: Box::new(character(&mut Seq::new(1), "nobody", "Nobody")),
            },
            8 => {
                let k = r.below(3) as usize;
                let mut c = character(
                    &mut Seq::new(1 + k as u32),
                    &names[k].to_lowercase(),
                    names[k],
                );
                c.exp = i;
                c.level = r.below(50) as u16;
                Write::Character {
                    id: cids[k],
                    character: Box::new(c),
                }
            }
            9 => {
                let n = r.below(SHOPS as u32) as i32 + 1;
                let mut stock = ShopStock::of(&shop(&mut Seq::new(1), n));
                stock.qty[r.below(4) as usize] = i as u16;
                Write::ShopStock(stock)
            }
            10 if refusals => match r.below(2) {
                0 => Write::ItemOwnershipInsert(item_ownership(&mut seq, r.below(6) as i32 + 1)),
                _ => Write::ItemOwnershipDelete(r.below(6) as i32 + 1),
            },
            10 => match r.below(2) {
                0 => Write::ItemOwnershipInsert(item_ownership(&mut seq, 100 + i as i32)),
                _ => Write::ItemOwnershipDelete(100 + r.below(i + 1) as i32),
            },
            11 => match r.below(3) {
                0 => Write::EvilPointsDelete(acct),
                _ => Write::EvilPoints {
                    account: acct,
                    record: evil_record(&mut seq, "wendy"),
                },
            },
            12 => match r.below(3) {
                0 => Write::BankBookDelete {
                    account: acct,
                    bank: ShopNo(r.below(3) as i32 + 1),
                },
                _ => Write::BankBook {
                    account: acct,
                    book: bank_book(&mut seq, "wendy", r.below(3) as i32 + 1),
                },
            },
            14 => Write::Gang(gang(
                &mut seq,
                ["Hawks", "Doves", "Owls"][r.below(3) as usize],
            )),
            15 => Write::GangDelete(["hawks", "doves", "owls"][r.below(3) as usize].to_string()),
            _ => Write::Event(Event::new(
                at(i64::from(i)),
                "t113",
                serde_json::json!({ "i": i }),
            )),
        };
        out.push(w);
    }
    out.push(Write::Profile {
        account: acct,
        record: permanent_record(&mut seq, "wendy"),
    });
    out
}

/// Everything the writes can change, read back.
async fn stored(db: &Db, acct: AccountId) -> String {
    let rooms = db.rooms().all().await.unwrap();
    let mut monsters = Vec::new();
    for n in 1..=MONSTERS {
        monsters.push(db.monster_state().get(MonsterNo(n)).await.unwrap());
    }
    let mut shops = Vec::new();
    for n in 1..=SHOPS {
        shops.push(db.shop_stock().get(ShopNo(n)).await.unwrap());
    }
    let items: Vec<_> = db
        .items()
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|i| (i.number, i.active))
        .collect();
    let events: Vec<_> = db
        .events()
        .recent(10_000)
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.event.kind, e.event.payload))
        .collect();
    format!(
        "{rooms:?}\n{monsters:?}\n{shops:?}\n{items:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{events:?}",
        db.active_monsters().all().await.unwrap(),
        db.characters().all().await.unwrap(),
        db.item_ownership().all().await.unwrap(),
        db.gangs().all().await.unwrap(),
        db.evil_points().get(acct).await.unwrap(),
        db.bank_books().for_account(acct).await.unwrap(),
        db.profiles().get(acct).await.unwrap(),
    )
}

async fn run(test: &TestDb, config: PersistConfig, refusals: bool) -> (String, u64) {
    let (acct, cids) = seed(test).await;
    let (persist, worker) = Persist::spawn(Arc::clone(test.db()), config);
    for w in writes(acct, &cids, refusals) {
        persist.send(w).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(120), persist.flush())
        .await
        .expect("flush within the timeout")
        .unwrap();
    let dropped = persist.dropped();
    drop(persist);
    worker.await.unwrap();
    (stored(test, acct).await, dropped)
}

/// One write per transaction with the per-row writers: the path before T-113.
fn single() -> PersistConfig {
    PersistConfig {
        batch_max: 1,
        per_row_reference: true,
        ..PersistConfig::default()
    }
}

/// Everything in one batch: the most grouping, and the most writes to one row.
fn whole() -> PersistConfig {
    PersistConfig {
        batch_max: 10_000,
        batch_window: Duration::from_millis(500),
        ..PersistConfig::default()
    }
}

fn assert_same(got: &str, want: &str) {
    if got != want {
        let line = got
            .lines()
            .zip(want.lines())
            .position(|(g, w)| g != w)
            .unwrap_or(0);
        panic!("stored state differs at line {line}");
    }
}

#[tokio::test]
async fn a_batch_grouped_by_kind_stores_what_one_write_at_a_time_stores() {
    let one_by_one = db().await;
    let batched = db().await;
    let (want, want_dropped) = run(&one_by_one, single(), false).await;
    let (got, got_dropped) = run(&batched, whole(), false).await;
    assert_eq!((want_dropped, got_dropped), (0, 0));
    assert_same(&got, &want);
    one_by_one.close().await;
    batched.close().await;
}

#[tokio::test]
async fn a_refused_write_in_a_grouped_batch_stores_what_one_at_a_time_stores() {
    let one_by_one = db().await;
    let batched = db().await;
    let (want, want_dropped) = run(&one_by_one, single(), true).await;
    assert!(
        want_dropped > 0,
        "the stream has refusals (duplicate inserts)"
    );
    // Refusals may be fewer: coalescing drops an earlier save of a row, and with it a
    // duplicate insert it made (save, insert, save: the insert meets no row). What is
    // stored is the same.
    let (got, got_dropped) = run(&batched, whole(), true).await;
    assert!(got_dropped <= want_dropped);
    assert_same(&got, &want);
    one_by_one.close().await;
    batched.close().await;
}
