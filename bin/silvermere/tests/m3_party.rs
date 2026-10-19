//! M3 party tests (T-048): two characters on two telnet connections form a party, walk it,
//! change ranks, lose a follower at a level exit and disband. The lines are the ones of
//! `cmd_invite`, `cmd_follow`, `cmd_uninvite`, the party screen, the march and
//! `change_user_rank`.

use crate::common;

use common::m3::{Player, assert_classified, body_lines, classify, edit_live, place, team_db};
use common::stock::start;

/// The Village Entrance, the Weapons Shop north of it, the Armour Shop south of it.
const ENTRANCE: u32 = 2140;
const NARROW_ROAD: u32 = 2146;

fn names(raw: &[u8]) -> Vec<String> {
    classify(raw).into_iter().map(|e| e.name).collect()
}

/// Brynja leads, Isolde follows; both at `room`; returns the running server and the pair.
async fn duo(url: &str, room: u32) -> (silvermere::Running, Player, Player) {
    place(url, "Brynja", room).await;
    place(url, "Isolde", room).await;
    let r = start(url).await;
    let mut b = Player::enter(&r, "Brynja").await;
    let at = b.c.position();
    let i = Player::enter(&r, "Isolde").await;
    // the arrival announcement reaches Brynja after her prompt: wait for it
    b.wait_for(at, "Isolde just entered the Realm\\.").await;
    (r, b, i)
}

/// Brynja invites, Isolde follows.
async fn form(b: &mut Player, i: &mut Player) {
    let at = i.c.position();
    b.cmd("invite isolde").await;
    i.wait_for(at, "has invited you to follow").await;
    let at = b.c.position();
    i.cmd("follow brynja").await;
    b.wait_for(at, "Isolde started to follow you\\.").await;
}

/// `invite`, `follow`, `party` (both screens), a rank change and `uninvite`; every line is
/// the original's and is classified by the registry (`you_invited`, `invited_follow`, `following`,
/// `follower_joined`, `party_header`, `rank_moved`, `follower_removed`, `unfollowed`,
/// `party_none`).
#[tokio::test]
async fn party_formation() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = duo(&url, ENTRANCE).await;

    // INVITE: the inviter's line, the invitee is told with his prompt
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd("invite isolde").await;
    assert_eq!(
        body_lines(&t),
        ["invite isolde", "You have invited Isolde to follow you."]
    );
    let told = i.wait_for(ai, "has invited you to follow").await;
    assert!(
        body_lines(&told)[0].starts_with("Brynja has invited you to follow "),
        "{told}"
    );
    assert!(names(&b.c.raw()[ab..]).contains(&"you_invited".into()));
    assert!(names(&i.c.raw()[ai..]).contains(&"invited_follow".into()));

    // FOLLOW: no period on the follower's line; the leader is told
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = i.cmd("follow brynja").await;
    assert_eq!(
        body_lines(&t),
        ["follow brynja", "You are now following Brynja"]
    );
    let told = b.wait_for(ab, "started to follow you").await;
    assert_eq!(body_lines(&told), ["Isolde started to follow you."]);
    assert!(names(&i.c.raw()[ai..]).contains(&"following".into()));
    assert!(names(&b.c.raw()[ab..]).contains(&"follower_joined".into()));

    // PARTY: the leader's screen, then the follower's with his leader first
    let ab = b.c.position();
    let t = b.cmd("party").await;
    let lines = body_lines(&t);
    assert_eq!(lines[1], "The following people are in your travel party:");
    assert!(
        lines[2].starts_with("  Isolde Stonebrook") && lines[2].ends_with("- Midrank"),
        "{lines:?}"
    );
    assert!(
        lines[3].starts_with("  Brynja Stonebrook") && lines[3].ends_with("- Frontrank"),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_classified("party (leader)", &b.c.raw()[ab..], &["party"]);
    let ai = i.c.position();
    let t = i.cmd("party").await;
    let lines = body_lines(&t);
    assert_eq!(lines[1], "You are following Brynja.");
    assert_eq!(lines[2], "The following people are in your travel party:");
    assert!(lines[3].starts_with("  Brynja Stonebrook"), "{lines:?}");
    assert!(names(&i.c.raw()[ai..]).contains(&"party_header".into()));
    assert_classified("party (follower)", &i.c.raw()[ai..], &["party"]);

    // rank change: the follower to the back, the leader (first of his list) is told
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = i.cmd("backrank").await;
    assert_eq!(
        body_lines(&t),
        [
            "backrank",
            "You have moved to the back ranks of your group."
        ]
    );
    let told = b
        .wait_text(ab, r"just moved to the back rank in your group\.")
        .await;
    assert_eq!(
        body_lines(&told),
        ["Isolde just moved to the back rank in your group."]
    );
    assert!(names(&i.c.raw()[ai..]).contains(&"rank_moved".into()));
    assert!(names(&b.c.raw()[ab..]).contains(&"rank_moved".into()));

    // UNINVITE: both are told; both screens are empty afterwards
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd("uninvite isolde").await;
    assert_eq!(
        body_lines(&t),
        [
            "uninvite isolde",
            "Isolde has been removed from your followers."
        ]
    );
    let told = i.wait_for(ai, "no longer following").await;
    assert_eq!(body_lines(&told), ["You are no longer following Brynja."]);
    assert!(names(&b.c.raw()[ab..]).contains(&"follower_removed".into()));
    assert!(names(&i.c.raw()[ai..]).contains(&"unfollowed".into()));
    // alone again: the "not in a party" line, then the character's own line, as the
    // original always prints it last; rank reset to midrank
    for (p, who) in [(&mut b, "Brynja"), (&mut i, "Isolde")] {
        let t = p.cmd("party").await;
        let lines = body_lines(&t);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(
            lines[..2],
            ["party", "You are not in a party at the present time."]
        );
        assert!(
            lines[2].starts_with(&format!("  {who} Stonebrook")),
            "{lines:?}"
        );
        assert!(lines[2].ends_with("- Midrank"), "{lines:?}");
        assert!(names(p.c.raw()).contains(&"party_none".into()));
    }
    b.leave().await;
    i.leave().await;
    r.stop().await.unwrap();
    // a restart keeps no party (parties are not stored): the screen is the empty one
    let r = start(&url).await;
    let mut b = Player::enter(&r, "Brynja").await;
    let t = b.cmd("party").await;
    assert_eq!(
        body_lines(&t)[..2],
        ["party", "You are not in a party at the present time."]
    );
    b.leave().await;
    r.stop().await.unwrap();
}

/// The leader walks north, south, south; each time the follower prints
/// ` -- Following your Party leader <dir> --` and the room that follows, and the leader sees
/// the follower walk in after him.
#[tokio::test]
async fn party_moves() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = duo(&url, ENTRANCE).await;
    form(&mut b, &mut i).await;
    for (dir, word, back, title) in [
        ("n", "north", "south", "Newhaven, Weapons Shop"),
        ("s", "south", "north", "Newhaven, Village Entrance"),
        ("s", "south", "north", "Newhaven, Armour Shop"),
    ] {
        let (ab, ai) = (b.c.position(), i.c.position());
        b.say(dir).await;
        let walked = i
            .wait_for(ai, &format!(r"-- Following your Party leader {word} --"))
            .await;
        assert!(walked.contains(title), "{walked}");
        assert!(
            names(&i.c.raw()[ai..]).contains(&"follow_leader".into()),
            "{walked}"
        );
        let led = b
            .wait_for(
                ab,
                &format!(r"Isolde walks into the room from the {back}\."),
            )
            .await;
        assert!(led.contains(title), "{led}");
    }
    b.leave().await;
    i.leave().await;
    r.stop().await.unwrap();
}

/// The leader goes west from the Narrow Road through the Healer's Level exit (levels 1 to
/// 3); the follower, at level 5, is turned back. In the original (not the card) the follower is
/// dropped from `leader` silently: no `You are no longer following` line, only the exit's
/// own refusal for him and nothing for the leader.
/// He stays in the leader's list, so `follow` takes him back without a new invitation.
#[tokio::test]
async fn party_drop_at_a_level_exit() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = duo(&url, NARROW_ROAD).await;
    edit_live(&r, "Isolde", |c| c.level = 5).await;
    form(&mut b, &mut i).await;
    let (ab, ai) = (b.c.position(), i.c.position());
    b.say("w").await;
    let t = b.wait_for(ab, "Newhaven, Healer").await;
    assert!(t.contains("Newhaven, Healer"), "{t}");
    let refused = i
        .wait_for(ai, "You have progressed too far to go through this exit!")
        .await;
    assert!(!refused.contains("no longer following"), "{refused}");
    assert!(
        !b.c.plain_from(ab).contains("Isolde"),
        "{}",
        b.c.plain_from(ab)
    );
    // back together: the leader comes back east; FOLLOW works without INVITE
    let (ab, ai) = (b.c.position(), i.c.position());
    b.say("e").await;
    b.wait_for(ab, "Newhaven, Narrow Road").await;
    i.wait_for(ai, r"Brynja walks into the room from the west\.")
        .await;
    let t = i.cmd("follow brynja").await;
    assert_eq!(
        body_lines(&t),
        ["follow brynja", "You are now following Brynja"]
    );
    b.leave().await;
    i.leave().await;
    r.stop().await.unwrap();
}
