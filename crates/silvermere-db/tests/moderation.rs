//! Moderation: account sanctions, IP bans and the closed realm, written and read back.

use crate::common;

use common::*;
use silvermere_db::repo::{RealmClosed, Sanction, SanctionKind};

fn sanction(
    account: silvermere_types::ids::AccountId,
    kind: SanctionKind,
    until: Option<i64>,
) -> Sanction {
    Sanction {
        account,
        kind,
        until: until.map(at),
        reason: format!("{} for spam", kind.as_str()),
        by_whom: "Sysop".into(),
        created: at(0),
    }
}

#[tokio::test]
async fn sanctions_are_set_replaced_and_lifted() {
    let db = db().await;
    let mut seq = Seq::new(1);
    let lars = new_account(&db, &mut seq, "Lars").await;
    let mia = new_account(&db, &mut seq, "Mia").await;
    let m = db.moderation();
    m.set(&sanction(lars, SanctionKind::Jail, Some(600)))
        .await
        .unwrap();
    m.set(&sanction(lars, SanctionKind::Mute, None))
        .await
        .unwrap();
    m.set(&sanction(mia, SanctionKind::Ban, Some(60)))
        .await
        .unwrap();
    // a second jail replaces the first
    m.set(&sanction(lars, SanctionKind::Jail, Some(1200)))
        .await
        .unwrap();
    let of = m.of(lars).await.unwrap();
    assert_eq!(
        of,
        vec![
            sanction(lars, SanctionKind::Jail, Some(1200)),
            sanction(lars, SanctionKind::Mute, None),
        ]
    );
    // at 100 s Mia's ban is over; Lars's jail and mute are not
    let active = m.active(at(100)).await.unwrap();
    assert_eq!(
        active
            .iter()
            .map(|s| (s.account, s.kind))
            .collect::<Vec<_>>(),
        vec![(lars, SanctionKind::Jail), (lars, SanctionKind::Mute)]
    );
    assert!(m.lift(lars, SanctionKind::Mute).await.unwrap());
    assert!(!m.lift(lars, SanctionKind::Mute).await.unwrap());
    assert_eq!(m.of(lars).await.unwrap().len(), 1);
    // deleting the account takes its sanctions with it
    db.accounts().delete(lars).await.unwrap();
    assert!(m.of(lars).await.unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn ip_bans_are_kept_by_network() {
    let db = db().await;
    let m = db.moderation();
    let a = m
        .ban_ip("203.0.113.0/24", None, "bots", "Sysop", at(0))
        .await
        .unwrap();
    let b = m
        .ban_ip("2001:db8::/32", Some(at(60)), "", "ops", at(1))
        .await
        .unwrap();
    assert_ne!(a, b);
    // the same network again replaces the ban
    m.ban_ip("203.0.113.0/24", Some(at(90)), "bots again", "Sysop", at(2))
        .await
        .unwrap();
    let bans = m.ip_bans().await.unwrap();
    assert_eq!(bans.len(), 2);
    assert_eq!(bans[0].network, "203.0.113.0/24");
    assert_eq!(bans[0].until, Some(at(90)));
    assert_eq!(bans[0].reason, "bots again");
    assert!(bans[1].active_at(at(59)) && !bans[1].active_at(at(60)));
    assert!(m.unban_ip("2001:db8::/32").await.unwrap());
    assert!(!m.unban_ip("2001:db8::/32").await.unwrap());
    assert_eq!(m.ip_bans().await.unwrap().len(), 1);
    db.close().await;
}

#[tokio::test]
async fn the_realm_closes_and_opens() {
    let db = db().await;
    let m = db.moderation();
    assert_eq!(m.realm_closed().await.unwrap(), None);
    let closed = RealmClosed {
        reason: "upgrading".into(),
        by_whom: "Sysop".into(),
        created: at(5),
    };
    m.close_realm(&closed).await.unwrap();
    assert_eq!(m.realm_closed().await.unwrap(), Some(closed.clone()));
    let again = RealmClosed {
        reason: "still upgrading".into(),
        ..closed
    };
    m.close_realm(&again).await.unwrap();
    assert_eq!(m.realm_closed().await.unwrap(), Some(again));
    assert!(m.open_realm().await.unwrap());
    assert!(!m.open_realm().await.unwrap());
    assert_eq!(m.realm_closed().await.unwrap(), None);
    db.close().await;
}
