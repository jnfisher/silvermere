//! `SessionId` allocation: dense, reused lowest first, capped at `server.max_sessions`.

use silvermere_engine::{ServerFull, SessionId, SessionIds};

#[test]
fn allocate_256_free_3_reuse_lowest_first_then_full() {
    let mut ids = SessionIds::new(256);
    let all: Vec<SessionId> = (0..256).map(|_| ids.alloc().unwrap()).collect();
    assert_eq!(all.first(), Some(&SessionId(0)));
    assert_eq!(all.last(), Some(&SessionId(255)));
    assert_eq!(ids.alloc(), Err(ServerFull { max: 256 }));

    ids.free(SessionId(200));
    ids.free(SessionId(7));
    ids.free(SessionId(42));
    assert_eq!(ids.in_use(), 253);
    let back: Vec<SessionId> = (0..3).map(|_| ids.alloc().unwrap()).collect();
    assert_eq!(back, [SessionId(7), SessionId(42), SessionId(200)]);
    assert_eq!(ids.alloc(), Err(ServerFull { max: 256 }));
    assert!(ids.is_allocated(SessionId(7)));
}

#[test]
fn freeing_twice_or_out_of_range_changes_nothing() {
    let mut ids = SessionIds::new(2);
    let a = ids.alloc().unwrap();
    ids.free(a);
    ids.free(a);
    ids.free(SessionId(9));
    assert_eq!(ids.in_use(), 0);
    assert_eq!(ids.alloc().unwrap(), a);
    assert_eq!(ids.alloc().unwrap(), SessionId(1));
}
