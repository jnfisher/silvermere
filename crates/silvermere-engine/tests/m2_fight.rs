//! Milestone M2 ("fight"), the scenario layer (T-040): fight, death,
//! rebirth, rest and spawn scenarios through the real `Engine` on the testkit's
//! `TestClock` with a seeded `Rng`. Lines go in as `EngineInput::Line`, time as one
//! `EngineInput::Tick` per simulated second; nothing sleeps.
//!
//! The world is `silvermere-testkit`'s `scripts::m2` (Newhaven Arena with the stock giant rat,
//! acid slime and kobold thief records, Kasimir's Kang Witchunter sheet). Where a test needs
//! an outcome the dice would only give at times (a monster that always hits, a set hit
//! point total), it sets the record or the character directly: these in-process tests own
//! the `World`, so no test-only code is built into the engine.
//!
//! Random draws are never compared as sequences, only as masked events or distributions.

// The testkit pieces this test uses, by path: the engine may not name silvermere-testkit, not
// even as a dev-dependency (tests/deps.rs), and no build edge is added here. At the crate root,
// so their `crate::` paths hold; `captures.rs` decodes with `silvermere_net::cp437`, a plain
// table, which this crate provides under that name.
extern crate self as silvermere_net;
#[path = "../../silvermere-testkit/src/captures.rs"]
#[allow(dead_code)]
mod captures;
#[path = "../../silvermere-testkit/src/clock.rs"]
#[allow(dead_code)]
mod clock;
#[path = "../../silvermere-net/src/cp437.rs"]
#[allow(dead_code)]
mod cp437;
#[path = "../../silvermere-testkit/src/scripts/m2.rs"]
#[allow(dead_code)]
mod m2;
#[path = "../../silvermere-testkit/src/normalise.rs"]
#[allow(dead_code)]
mod normalise;
#[path = "../../silvermere-testkit/src/patterns.rs"]
#[allow(dead_code)]
mod patterns;
#[path = "../../silvermere-testkit/src/stats.rs"]
#[allow(dead_code)]
mod stats;

use captures::Capture;
use m2::*;
use normalise::{MaskSpec, normalise};
use patterns::{Event, PatternSet};
use silvermere_engine::SessionId;
use silvermere_engine::combat::autocombat::is_inside_autocombat;
use silvermere_rules::combat::energy_round;
use silvermere_types::enums::RoomType;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MonsterNo, RoomId, RoomKey};
use silvermere_types::records::Coins;
use stats::{binomial_z, chi_square_gof};

const ALPHA: f64 = 0.01;

fn pats() -> PatternSet {
    PatternSet::load_default().expect("patterns.json")
}

/// The random monster adjectives of the stock name lists (`fight-arena` mask hints).
const ADJECTIVES: [&str; 12] = [
    "large ", "small ", "big ", "thin ", "fat ", "nasty ", "angry ", "fierce ", "short ", "tall ",
    "happy ", "slimy ",
];

/// One event as the fight comparison sees it: the name and the fields after the shared
/// masks (`tests/fixtures/masks.json`: damage, experience, coins) plus the verb, which the
/// original picks at random from the weapon's `|` list; an
/// unclassified line keeps its text with the adjectives and numbers masked.
fn fight_key(e: &Event) -> String {
    if e.name == "unknown" {
        let mut line = e.line.clone();
        for a in ADJECTIVES {
            line = line.replace(a, "");
        }
        let line: String = line
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect();
        return format!("unknown: {line}");
    }
    let mut fields = e.fields.clone();
    // without game data the registry cannot split `whap large giant rat` into the verb
    // and the target: join them, drop the adjective, mask the first word (the verb)
    if let (Some(verb), Some(target)) = (fields.remove("verb"), fields.remove("target")) {
        let mut both = format!("{verb} {target}");
        for a in ADJECTIVES {
            both = both.replace(a, "");
        }
        let rest = both.split_once(' ').map_or("", |(_, r)| r).to_string();
        fields.insert("target".into(), rest);
        fields.insert("verb".into(), "*".into());
    }
    let fields: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{} {}", e.name, fields.join(" "))
}

/// The fight in `lines`: the events from the first `*Combat Engaged*` to the last
/// `*Combat Off*`, prompts and command echoes left out, masked.
fn fight_events(lines: &[String], sent: &[&str]) -> Vec<String> {
    let masks = MaskSpec::load_default().expect("masks.json");
    let events = normalise(&pats().classify_lines(lines), &masks);
    let first = events.iter().position(|e| e.name == "combat_engaged");
    let last = events.iter().rposition(|e| e.name == "combat_off");
    let (Some(first), Some(last)) = (first, last) else {
        return Vec::new();
    };
    events[first..=last]
        .iter()
        .filter(|e| e.cat != "prompt")
        .filter(|e| !(e.name == "unknown" && sent.contains(&e.line.trim())))
        .map(fight_key)
        .collect()
}

/// The player's blows in `text` on a monster named `mon`.
fn blows(text: &[String], mon: &str) -> usize {
    text.iter()
        .filter(|l| l.starts_with("You ") && l.contains(mon) && !l.starts_with("You gain"))
        .count()
}

/// The `fight-arena` capture (T-005): Aldric attacks the arena's giant rat with `a rat`,
/// sends `a rat` again while engaged (`*Combat Off*`, `*Combat Engaged*`, the restart),
/// takes two misses ("lunges at you!") and lands two blows; the rat dies (its WCCMSG 27
/// death line), `You gain 9 experience.`, `*Combat Off*`. The same script on the new
/// engine gives the same event sequence once damage, the random hit verb, the random
/// adjective and experience are masked. The seed is the one whose draws give the capture's
/// outcomes (two misses for the rat, two hits for the player, the rat's blow first in the
/// 60/40 order); the dice themselves are never compared. Covers `cmd_any_attack`, the
/// round, `check_kill_monster` and `distribute_experience`.
#[test]
fn fight_rat_matches_capture_after_masking() {
    let cap =
        Capture::load(captures::captures_dir().join("fight-arena")).expect("fight-arena capture");
    let sent = ["a rat", "get copper"];
    let want = fight_events(&cap.raw_lines(), &sent);
    assert!(want.len() >= 10, "capture fight: {want:#?}");

    let mut h = M2::new(false, 1);
    let k = h.kang("Kasimir", ARENA);
    let rat = h.monster(RAT, ARENA);
    h.lines(k);
    h.line(k, "a rat");
    h.line(k, "a rat");
    for _ in 0..60 {
        h.ticks(1);
        if !h.alive(rat) {
            break;
        }
    }
    h.ticks(1);
    let got = fight_events(&h.lines(k), &sent);
    assert_eq!(got, want, "new engine vs fight-arena");
}

/// A re-sent attack restarts auto-combat: a second `bash`
/// while engaged prints `*Combat Off*` then `*Combat Engaged*` (the entry is killed and
/// restarted), the carried-over energy is gone, and the next round makes fewer blows than
/// the same fight without the re-send. The weapon is a fast staff (speed 600, so a bash
/// costs about 550 and a round alternates one and two blows); the stock quarterstaff's
/// bash costs the full 1000 and always makes one.
#[test]
fn bash_keeps_bashing_and_restart_resets_energy() {
    let run = |resend: bool| -> (Vec<usize>, Vec<String>, i32, i32) {
        let mut h = M2::new(false, 3);
        h.w().items.get_mut(&QUARTERSTAFF).unwrap().speed = 600;
        let k = h.kang("Kasimir", ARENA);
        h.sturdy(k);
        let rat = h.monster(RAT, ARENA);
        h.mon(rat).hp = 30_000;
        h.line(k, "bash rat");
        assert!(h.text(k).contains(&"*Combat Engaged*".to_owned()));
        let mut counts = Vec::new();
        let mut carry = 0;
        let mut cost = 0;
        let mut found = false;
        // run rounds until the carried energy would give two blows next round
        for _ in 0..20 {
            h.next_round();
            counts.push(blows(&h.text(k), "giant rat"));
            carry = i32::from(h.plr(k).energy);
            if counts.len() == 1 {
                // the first round starts from 1000 and makes one bash
                cost = 1000 - carry;
            }
            if carry > 0 && counts.len() >= 2 && energy_round(carry, cost).blows > 1 {
                found = true;
                break;
            }
        }
        assert!(found, "no carried energy: {counts:?}");
        let mut restart = Vec::new();
        if resend {
            h.line(k, "bash rat");
            restart = h.text(k);
            assert_eq!(h.plr(k).energy, 0, "the restart empties the pool");
        }
        h.next_round();
        counts.push(blows(&h.text(k), "giant rat"));
        assert!(is_inside_autocombat(h.w(), k));
        (counts, restart, carry, cost)
    };
    let (plain, _, carry, cost) = run(false);
    let (resent, lines, carry2, _) = run(true);
    assert_eq!(carry, carry2, "same fight up to the re-send");
    let off = lines.iter().position(|l| l == "*Combat Off*");
    let on = lines.iter().position(|l| l == "*Combat Engaged*");
    assert!(
        off.is_some() && on.is_some() && off < on,
        "Off then Engaged: {lines:?}"
    );
    let n = plain.len();
    assert_eq!(plain[..n - 1], resent[..n - 1]);
    assert!(
        resent[n - 1] < plain[n - 1],
        "fewer blows after the restart: {resent:?} vs {plain:?}"
    );
    // the plain run carries `carry` into the
    // round, the restarted one starts from nothing
    assert_eq!(plain[n - 1] as u32, energy_round(carry, cost).blows);
    assert_eq!(resent[n - 1] as u32, energy_round(0, cost).blows);
}

/// `check_kill_user`, the wounded gate, `drops to the ground!` and the bell, and
/// `deathhp` (stock -10, John's server -200): a giant rat that
/// always bites for 17 drives Kasimir from 48 through 0. Down, he may look, `st` (his hit
/// points negative) and `exp`; `rest` gets the mortally-wounded line. At `deathhp` he dies:
/// `You have been killed!`, the miracle, `You have 8 lives left.`, the reincarnation room,
/// full hit points, his staff and gold on the arena floor, and Wendy, waiting in the
/// reincarnation room, sees him appear.
#[test]
fn death_and_rebirth_sequence() {
    for deathhp in [-10, -200] {
        let mut h = M2::new(false, 5);
        h.w().config.death_hp = deathhp;
        {
            let k = h.w().monsters.get_mut(&MonsterNo(RAT)).unwrap();
            k.attacks[0].accuracy = 1000;
            (k.attacks[0].min, k.attacks[0].max) = (17, 17);
        }
        let reinc = reinc_room();
        let k = h.kang("Kasimir", ARENA);
        let wendy = h.kang("Wendy", reinc);
        h.plr(k).coins = Coins {
            gold: 3,
            ..Coins::default()
        };
        h.monster(RAT, ARENA);
        h.take(k);
        h.take(wendy);
        // tick until he is down
        let mut raw = Vec::new();
        for _ in 0..60 {
            h.ticks(1);
            raw.extend(h.take(k));
            if h.plr(k).hp <= 0 {
                break;
            }
        }
        let hp = h.plr(k).hp;
        assert_eq!(hp, -3, "48 - 3*17");
        let text = String::from_utf8_lossy(&raw).into_owned();
        assert!(
            text.contains("Kasimir drops to the ground!\x07"),
            "deathhp {deathhp}: the drop and the bell: {text:?}"
        );
        // the unconscious window
        if deathhp < i32::from(hp) {
            h.line(k, "look");
            let look = h.text(k);
            assert!(
                look.iter().any(|l| l.contains("Newhaven, Arena")),
                "{look:?}"
            );
            h.line(k, "st");
            let st = h.text(k);
            assert!(
                st.iter()
                    .any(|l| l.starts_with("Hits:") && l.contains("-3/48")),
                "st shows the negative hit points: {st:?}"
            );
            h.line(k, "exp");
            assert!(h.text(k).iter().any(|l| l.starts_with("Exp:")));
            h.line(k, "rest");
            assert_eq!(
                h.text(k),
                ["You may not do that while you are mortally wounded!"],
                "rest is refused"
            );
            assert!(!h.plr(k).status.contains(PlayerStatus::RESTING));
        }
        // the rat goes on biting until deathhp
        let mut seen = Vec::new();
        for _ in 0..400 {
            h.ticks(1);
            seen.extend(h.text(k));
            if h.plr(k).room == RoomId(reinc) {
                break;
            }
        }
        let killed = seen.iter().position(|l| l == "You have been killed!");
        assert!(killed.is_some(), "deathhp {deathhp}: {seen:?}");
        let i = killed.unwrap();
        assert_eq!(
            seen[i..i + 3],
            [
                "You have been killed!",
                "But, due to a miracle, you have been saved.",
                "You have 8 lives left."
            ]
        );
        let c = h.plr(k).clone();
        assert_eq!((c.lives, c.hp, c.maxhp, c.room), (8, 48, 48, RoomId(reinc)));
        assert_eq!(c.weapon, ItemNo(0));
        let arena = h.w().room_state[&RoomKey::new(1, ARENA)].clone();
        assert!(
            arena.items.iter().any(|i| i.item == QUARTERSTAFF),
            "the pile"
        );
        assert_eq!(arena.coins.gold, 3);
        assert!(
            h.text(wendy)
                .contains(&"Kasimir appeared on the floor in the middle of the room.".to_owned()),
            "Wendy sees him appear"
        );
    }
}

/// The slow tick (`max((level+20)*hea/750, 1)`), the rest tick (every 21st fast
/// tick of resting heals that tripled) and `rest`: Kasimir rests from
/// 20/48 for 120 s of `TestClock`, and the hit points of every prompt are the schedule the
/// formulas give. Then a giant rat arrives: its arrival alone leaves him resting
/// (`generate_monster` touches no player), and its first bite ends the rest
/// (`attack_monster_user`).
#[test]
fn rest_timing_matches_formula() {
    let mut h = M2::new(false, 7);
    let k = h.kang("Kasimir", ARENA);
    h.plr(k).hp = 20;
    h.take(k);
    let (level, hea) = (1i32, 74i32);
    let base = ((level + 20) * hea / 750).max(1);
    let start = h.tick_count();
    h.line(k, "rest");
    assert_eq!(h.text(k), ["You are now resting."]);
    // the schedule: slow ticks on the scheduler's 30 s period, rest ticks every 21st
    let (mut hp, mut resttick) = (20, 0);
    let mut want = Vec::new();
    for t in 1..=120u64 {
        let mut changed = false;
        if (start + t).is_multiple_of(30) && hp < 48 {
            hp = (hp + base).min(48);
            changed = true;
        }
        resttick += 1;
        if resttick == 21 {
            resttick = 0;
            hp = (hp + base * 3).min(48);
            changed = true;
        }
        if changed {
            want.push(hp);
        }
    }
    let mut got = Vec::new();
    for _ in 0..120 {
        h.ticks(1);
        got.extend(prompt_hps(&h.lines(k)));
    }
    assert_eq!(got, want, "prompts while resting");
    assert_eq!(i32::from(h.plr(k).hp), hp);
    assert!(h.plr(k).status.contains(PlayerStatus::RESTING));

    // a monster arrives: still resting; its bite ends it
    h.plr(k).hp = 20;
    {
        let rat = h.w().monsters.get_mut(&MonsterNo(RAT)).unwrap();
        rat.attacks[0].accuracy = 1000;
    }
    h.monster(RAT, ARENA);
    let arrival = h.text(k);
    assert!(
        arrival
            .iter()
            .any(|l| l.contains("giant rat") && l.contains("into the room")),
        "{arrival:?}"
    );
    assert!(
        h.plr(k).status.contains(PlayerStatus::RESTING),
        "arrival alone"
    );
    for _ in 0..5 {
        h.ticks(1);
        if h.plr(k).hp < 20 {
            break;
        }
    }
    assert!(h.plr(k).hp < 20, "bitten");
    assert!(
        !h.plr(k).status.contains(PlayerStatus::RESTING),
        "the bite ends the rest"
    );
}

/// The spawn pass: a room spawns when its players outnumber its
/// monsters and a roll of 1..=99 is under 90 for an Arena, 5 for a Normal room, so
/// 89/99 and 4/99 of the eligible ticks; each player in the room rolls with the counts of
/// the first, so two players in the empty arena get 0, 1 or 2 monsters as two rolls of
/// 89/99 (spawns scale with the player count; the two-player spawn-arena scenario of T-035).
/// 2,000 ticks each, z- and chi-square tests at p > 0.01.
#[test]
fn spawn_rate_arena_vs_normal() {
    const N: u64 = 2_000;
    let arena_p = 89.0 / 99.0;
    let normal_p = 4.0 / 99.0;
    let clear = |h: &mut M2, room: u32| {
        for m in h.here(room) {
            silvermere_engine::monsters::active::remove_active_monster(h.w(), m, false);
        }
    };

    // one player in the arena
    let mut h = M2::new(true, 21);
    h.kang("Kasimir", ARENA);
    let mut hits = 0;
    for _ in 0..N {
        h.w().spawn_cycle();
        if !h.here(ARENA).is_empty() {
            hits += 1;
            clear(&mut h, ARENA);
        }
    }
    let t = binomial_z(hits, N, arena_p);
    assert!(t.passes(ALPHA), "arena {hits}/{N}: {t:?}");

    // one player in a Normal room of the arena's monster group, no exits
    let mut s = snapshot(true);
    let road = s.rooms.iter_mut().find(|r| r.key.room.0 == ROAD).unwrap();
    road.mongroup = ARENA_GROUP;
    road.maxmonsters = 15;
    road.room_type = RoomType::Normal;
    for e in &mut road.exits {
        e.target = RoomId(0);
    }
    let mut h = M2::with(s, config(), 22);
    h.kang("Kasimir", ROAD);
    let mut hits = 0;
    for _ in 0..N {
        h.w().spawn_cycle();
        if !h.here(ROAD).is_empty() {
            hits += 1;
            clear(&mut h, ROAD);
        }
    }
    let t = binomial_z(hits, N, normal_p);
    assert!(t.passes(ALPHA), "normal room {hits}/{N}: {t:?}");

    // two players in the arena (maxmonsters 2): up to two a tick, two rolls of 89/99
    let mut h = M2::new(true, 23);
    h.kang("Kasimir", ARENA);
    h.kang("Brynja", ARENA);
    let mut hist = [0u64; 3];
    for _ in 0..N {
        h.w().spawn_cycle();
        let n = h.here(ARENA).len();
        assert!(n <= 2, "{n} monsters for two players");
        hist[n] += 1;
        clear(&mut h, ARENA);
    }
    let q = 1.0 - arena_p;
    let t = chi_square_gof(&hist, &[q * q, 2.0 * arena_p * q, arena_p * arena_p]);
    assert!(t.passes(ALPHA), "two players {hist:?}: {t:?}");
}

/// `distribute_experience` and its split: Kasimir and Brynja
/// both fight one giant rat; the kill's `expval` (exp 9 x level 1) is divided by the two
/// attackers, `exp/=cnt` with at least 1, and each gets the share.
#[test]
fn exp_split_two_attackers() {
    let mut h = M2::new(false, 9);
    let a = h.kang("Kasimir", ARENA);
    let b = h.kang("Brynja", ARENA);
    h.sturdy(a);
    h.sturdy(b);
    let rat = h.monster(RAT, ARENA);
    h.mon(rat).hp = 30_000;
    h.line(a, "a rat");
    h.line(b, "a rat");
    h.next_round();
    assert!(is_inside_autocombat(h.w(), a) && is_inside_autocombat(h.w(), b));
    h.mon(rat).hp = 1;
    h.text(a);
    h.text(b);
    for _ in 0..10 {
        h.next_round();
        if !h.alive(rat) {
            break;
        }
    }
    assert!(!h.alive(rat));
    // expval = exp 9 x level 1; divided by the attackers, at least 1
    let (expval, attackers) = (9u32, 2u32);
    let share = (expval / attackers).max(1);
    for id in [a, b] {
        let t = h.text(id);
        assert!(
            t.contains(&format!("You gain {share} experience.")),
            "{id:?}: {t:?}"
        );
        assert_eq!(h.plr(id).exp, share);
    }
}

/// Telepath (`cmd_whisper`)
/// is one of the commands a mortally wounded player keeps. Kasimir
/// lies at -5 (deathhp -200): he telepaths Brynja and gets her answer; `rest` is refused.
#[test]
fn mortally_wounded_telepath_works() {
    let mut h = M2::new(false, 11);
    h.w().config.death_hp = -200;
    let k = h.kang("Kasimir", ARENA);
    let b = h.kang("Brynja", ROAD);
    h.plr(k).hp = -5;
    h.take(k);
    h.take(b);
    h.line(k, "/Brynja @down");
    assert!(
        h.text(b).contains(&"Kasimir telepaths: @down".to_owned()),
        "Brynja hears him"
    );
    h.text(k);
    h.line(b, "/Kasimir hold on");
    assert!(
        h.text(k).contains(&"Brynja telepaths: hold on".to_owned()),
        "he hears her"
    );
    h.line(k, "rest");
    assert_eq!(
        h.text(k),
        ["You may not do that while you are mortally wounded!"]
    );
}

/// `give_monsters_a_free_attack`, `handle_following_monsters` (the arrival record's third
/// text) and `move_monster`'s group rule: Kasimir climbs the stairs out of the arena with a
/// giant rat (`followpct` 100) after him. The rat takes its free swing as he leaves. In the stock
/// data the Narrow Road is monster group 0 (`WCCMP001` 2146), so the arena's group-6 rat
/// stays below; on a road of the arena's group it follows, and its arrival line comes
/// after the room. The card names the `creeps in after you!` family (WCCMSG 26's third
/// text, the registry's `monster_follows`), but 1.11p never prints that text: the follower
/// moves with a plain `move_monster`, so the room,
/// Kasimir included, sees `giant rat moves into the room from below.`.
#[test]
fn monster_free_attack_and_follow_on_leave() {
    for road_group in [0, ARENA_GROUP] {
        let mut h = M2::new(false, 13);
        h.w()
            .rooms
            .get_mut(&RoomKey::new(1, ROAD))
            .unwrap()
            .mongroup = road_group;
        {
            let rat = h.w().monsters.get_mut(&MonsterNo(RAT)).unwrap();
            rat.followpct = 100;
            rat.attacks[0].accuracy = 1000;
        }
        let k = h.kang("Kasimir", ARENA);
        let rat = h.monster(RAT, ARENA);
        h.mon(rat).followpct = 100;
        h.mon(rat).following = "Kasimir".into();
        h.text(k);
        // the move is queued and made on the next fast tick
        h.line(k, "u");
        h.ticks(2);
        let t = h.text(k);
        let swing = t
            .iter()
            .position(|l| l.starts_with("The giant rat bites you for"));
        let road = t.iter().position(|l| l == "Newhaven, Narrow Road");
        let follow = t
            .iter()
            .position(|l| l == "giant rat moves into the room from below.");
        assert!(!t.iter().any(|l| l.contains("in after you")), "{t:?}");
        assert!(
            matches!((swing, road), (Some(a), Some(b)) if a < b),
            "group {road_group}: the swing, then the room: {t:?}"
        );
        assert!(h.plr(k).hp < 48);
        if road_group == ARENA_GROUP {
            assert!(
                matches!((road, follow), (Some(b), Some(c)) if b < c),
                "the follow line after the room: {t:?}"
            );
            assert_eq!(h.here(ROAD), [rat]);
        } else {
            assert_eq!(follow, None, "{t:?}");
            assert_eq!(h.here(ARENA), [rat], "the rat stays in its group's rooms");
        }
    }
}

/// The failure model: a handler that panics while two fights are on
/// interrupts its own session's command (`Command interrupted.`) and nothing else. The
/// other session's next round lands as before, and the world goes on: both fights finish.
/// (The engine has no seam to throw inside a blow; the panic is a `Custom` input of
/// Kasimir's between two energy rounds.)
#[test]
fn handler_panic_during_combat_isolated() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let expected = info
            .payload()
            .downcast_ref::<&str>()
            .is_some_and(|m| *m == "probe: mid-fight");
        if !expected {
            default_hook(info);
        }
    }));
    let mut h = M2::new(false, 17);
    let a = h.kang("Kasimir", ARENA);
    let b = h.kang("Brynja", ROAD);
    let ra = h.monster(RAT, ARENA);
    let rb = h.monster(RAT, ROAD);
    for (id, m) in [(a, ra), (b, rb)] {
        h.sturdy(id);
        h.mon(m).hp = 30_000;
        h.line(id, "a rat");
    }
    let target = |h: &mut M2, id: SessionId| -> Option<ActiveMonsterId> {
        match h.w().autocombat.entry(id).target? {
            silvermere_engine::combat::AutoTarget::Monster(m) => Some(m),
            _ => None,
        }
    };
    let (ta, tb) = (target(&mut h, a), target(&mut h, b));
    assert!(ta.is_some() && tb.is_some(), "both engaged: {ta:?} {tb:?}");
    h.next_round();
    h.text(a);
    h.text(b);
    let hp_b = h.mon(tb.unwrap()).hp;
    h.engine.handle(silvermere_engine::EngineInput::Custom {
        session: Some(a),
        run: Box::new(|_| panic!("probe: mid-fight")),
    });
    assert_eq!(h.text(a), ["Command interrupted."]);
    assert!(h.text(b).is_empty(), "nothing reaches Brynja");
    h.next_round();
    assert!(blows(&h.text(b), "giant rat") >= 1, "Brynja's round");
    assert!(h.mon(tb.unwrap()).hp <= hp_b);
    assert!(blows(&h.text(a), "giant rat") >= 1, "Kasimir's round too");
    // both fights finish
    h.mon(ta.unwrap()).hp = 1;
    h.mon(tb.unwrap()).hp = 1;
    for _ in 0..10 {
        h.next_round();
        if !h.alive(ta.unwrap()) && !h.alive(tb.unwrap()) {
            break;
        }
    }
    assert!(!h.alive(ta.unwrap()) && !h.alive(tb.unwrap()));
    assert_eq!(h.w().sessions.len(), 2);
}
