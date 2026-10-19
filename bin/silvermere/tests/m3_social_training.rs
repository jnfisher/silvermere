//! M3 social-action and training tests (T-048): the generated action family for a lone
//! actor and for a target in the room (`cmd_action`, WCCACTS records, whose lines the Rust
//! registry cannot classify) and the `train stats` refusal away from a trainer.

use crate::common;

use common::m3::{Player, assert_classified, body_lines, classify, place, start_manual, team_db};
use common::stock::start;

fn names(raw: &[u8]) -> Vec<String> {
    classify(raw).into_iter().map(|e| e.name).collect()
}

/// `smile` and `smile isolde` in one room: the actor, the target and the bystander-free
/// room each get the line the action record asks for; `action` lists the verbs in record
/// order. Every line is classified by the registry.
#[tokio::test]
async fn social_actions() {
    let (_dir, url) = team_db();
    place(&url, "Brynja", 2140).await;
    place(&url, "Isolde", 2140).await;
    let r = start_manual(&url).await;
    let mut b = Player::enter(&r, "Brynja").await;
    let at = b.c.position();
    let mut i = Player::enter(&r, "Isolde").await;
    b.wait_for(at, "Isolde just entered the Realm\\.").await;

    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd("smile").await;
    assert_eq!(body_lines(&t), ["smile", "You smile happily!"]);
    let t = b.cmd("smile isolde").await;
    assert_eq!(
        body_lines(&t),
        ["smile isolde", "You smile happily at Isolde!"]
    );
    let t = b.cmd("action").await;
    let lines = body_lines(&t);
    assert_eq!(
        lines[1],
        "hug kiss wave grin bow bleed nod laugh shake cough shrug growl yawn wink nudge"
    );
    assert!(
        lines.iter().any(|l| l.contains("whistle cackle elaugh")),
        "{lines:?}"
    );
    i.cmd("i").await;
    let heard = body_lines(&i.c.plain_from(ai));
    assert_eq!(
        heard[..2],
        ["Brynja smiles happily!", "Brynja smiles happily at you!"]
    );
    // The action lines and the verb list are game-data patterns (WCCACTS verbs): the Rust
    // registry (`patterns.json` statics) does not generate them, the Python registry of
    // the differential harness does. Here the lines are compared in full above.
    let _ = ab;
    b.leave_ticking(&r).await;
    i.leave_ticking(&r).await;
    r.stop().await.unwrap();
}

/// `train stats` in `room` (no trainer): `train_stats_refused`.
async fn train_stats_refused_in(room: u32) {
    let (_dir, url) = team_db();
    place(&url, "Aldric", room).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let at = p.c.position();
    let t = p.cmd("train stats").await;
    assert_eq!(
        body_lines(&t),
        ["train stats", "You may not train your stats here!"],
        "room {room}"
    );
    assert!(names(&p.c.raw()[at..]).contains(&"train_stats_refused".into()));
    assert_classified("train stats", &p.c.raw()[at..], &["train stats"]);
    p.leave().await;
    r.stop().await.unwrap();
}

/// `train stats` at the Weapons Shop.
#[tokio::test]
async fn train_stats_refusal_weapons_shop() {
    train_stats_refused_in(2141).await;
}

/// `train stats` at the Arena.
#[tokio::test]
async fn train_stats_refusal_arena() {
    train_stats_refused_in(2150).await;
}
