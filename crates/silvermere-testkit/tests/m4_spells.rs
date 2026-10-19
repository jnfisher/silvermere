//! M4 spells (T-057), layer 3: casting over telnet against the stock world in an
//! in-process server, one fresh copy of the world per test. The fast tick is fed by the
//! test ([`common::ticker`]), so a round takes milliseconds; the server seed fixes the dice.
//!
//! Rules pinned: the cast succeeds when Spellcasting plus the spell's difficulty beats a
//! roll of 0-100, the amount is rolled evenly between the spell's minimum and maximum, the
//! medium cycle's room spell lands on every other pass, monster casts (T-052).
//! Every scenario also checks that each line it saw is classified (static registry plus the
//! stock WCCMSG texts, [`common::Classifier`]) and that it ends at a prompt.

use crate::common;

use std::time::Duration;

use common::*;
use silvermere_auth::Role;
use silvermere_engine::combat::AutoTarget;
use silvermere_engine::spells::autocombat_cast;
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::SpellNo;

const DRUID: u16 = 13;
const MAGE: u16 = 12;
const VINE: SpellNo = SpellNo(25);
const MEND: SpellNo = SpellNo(27);
const FBAL: SpellNo = SpellNo(120);
const ARENA: (u32, u32) = (1, 2150);
const KOBOLD_THIEF: u16 = 7;
const GIANT_RAT: u16 = 1;
const DARK_GOBLIN: u16 = 53;
const ORC_SHAMAN: u16 = 481;
const TORCHLIT_CAVE: (u32, u32) = (9, 439);
const LAVA_POOL: (u32, u32) = (7, 991);
const TICK: Duration = Duration::from_millis(15);

/// A level-2 druid (Thornwyn) with vine strike and mend, hit points to survive a long fight.
async fn druid(r: &silvermere::Running, url: &str) -> silvermere_testkit::TelnetClient {
    let c = new_character(r, url, "Thornwyn", DRUID, &[]).await;
    edit_player(r, "Thornwyn", |p| {
        p.spellbook = vec![VINE, MEND];
        p.level = 2;
        p.maxhp = 400;
        p.hp = 400;
    })
    .await;
    c
}

fn assert_clean(cl: &Classifier, out: &str, sent: &[&str]) {
    let unknown = cl.unknown(out, sent);
    assert!(
        unknown.is_empty(),
        "unclassified lines: {unknown:#?}\n{out}"
    );
    assert!(
        regex::Regex::new(&format!("{PROMPT}\\s*$"))
            .unwrap()
            .is_match(out.trim_end()),
        "does not end at a prompt:\n{out}"
    );
}

/// spell_engage: `vine kobold` in the Arena engages (`*Combat Engaged*`), vine strike is
/// cast again each round without another command (the auto-combat recast,
/// `RoundHooks::cast`), and the kill gives experience.
#[tokio::test(flavor = "multi_thread")]
async fn vine_engages_recasts_each_round_and_kills() {
    let db = stock_db();
    let r = start(&db.url, 11).await;
    let mut c = druid(&r, &db.url).await;
    place(&r, "Thornwyn", ARENA.0, ARENA.1).await;
    spawn(&r, ARENA.0, ARENA.1, KOBOLD_THIEF).await;
    let cl = Classifier::new(&r).await;
    cmd(&mut c, "").await;
    let t = ticker(&r, TICK);
    let at = c.position();
    c.send_line("vine kobold").await.unwrap();
    c.until(
        r"You gain \d+ experience\.(?s:.*)\[HP=",
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    t.abort();
    let out = c.plain_from(at);
    assert!(out.contains("*Combat Engaged*"), "{out}");
    let attempts = out.matches("You cast vine strike at").count()
        + out.matches("You attempt to cast vine strike").count();
    assert!(attempts >= 2, "one command, {attempts} casts:\n{out}");
    let names = cl.names(&out);
    for want in ["combat_engaged", "exp_gain"] {
        assert!(names.iter().any(|n| n == want), "{want} missing:\n{out}");
    }
    let tail = cmd(&mut c, "").await;
    assert_clean(&cl, &format!("{out}{tail}"), &["vine kobold"]);
    r.stop().await.unwrap();
}

/// spell_rest: `rest` then `cast mend` (`cmd_cast`): casting ends resting, so the
/// prompt after it no longer says `(Resting)`.
#[tokio::test(flavor = "multi_thread")]
async fn casting_mend_ends_resting() {
    let db = stock_db();
    let r = start(&db.url, 12).await;
    let mut c = druid(&r, &db.url).await;
    // A cast needs the once-per-round permission (PS_CANCAST), which the combat round gives
    // every player. The test used to wait for the server's own
    // clock to run a round, which a loaded machine delays ("You have already cast a spell this
    // round!"): run the rounds by hand (60 fast ticks, as `cast_no_mana` does), before HP is set
    // so that regeneration cannot raise it.
    tick(&r, 60).await;
    edit_player(&r, "Thornwyn", |p| {
        p.hp = 100;
        p.spellcast = 200;
    })
    .await;
    let cl = Classifier::new(&r).await;
    let rest = cmd(&mut c, "rest").await;
    assert!(rest.contains("You are now resting."), "{rest}");
    assert!(rest.contains("(Resting)"), "{rest}");
    let mend = cmd(&mut c, "cast mend").await;
    let prompt = mend.trim_end().rsplit('\n').next().unwrap();
    assert!(!prompt.contains("(Resting)"), "{mend}");
    // the cast itself, not the echo: mend healed, or the 2% failure the cap leaves
    // (Spellcasting plus difficulty, capped at 98)
    let hp = on_world(&r, |w| {
        let id = session_of(w, "Thornwyn");
        w.session(id).unwrap().character.as_ref().unwrap().hp
    })
    .await;
    assert!(
        hp > 100 || mend.contains("You attempt to cast mend, but fail."),
        "mend neither healed nor failed (hp {hp}):\n{mend}"
    );
    assert_clean(&cl, &format!("{rest}{mend}"), &["rest", "cast mend"]);
    r.stop().await.unwrap();
}

/// spell_no_mana: by hand, a cast without the mana says `You do not have enough mana to
/// cast that spell.` (`short_lines`); in auto-combat the recast that finds the mana short
/// is silent and only spends the energy (`autocombat_short`), so the fight stays on and
/// the casts come back with the regenerated mana.
#[tokio::test(flavor = "multi_thread")]
async fn out_of_mana_by_hand_and_in_combat() {
    let db = stock_db();
    let r = start(&db.url, 13).await;
    let mut c = druid(&r, &db.url).await;
    place(&r, "Thornwyn", ARENA.0, ARENA.1).await;
    let m = spawn(&r, ARENA.0, ARENA.1, GIANT_RAT).await;
    on_world(&r, move |w| {
        if let Some(a) = w.active_monsters.values_mut().find(|a| a.number == m) {
            a.hp = 3000;
            a.maxhp = 3000;
        }
    })
    .await;
    edit_player(&r, "Thornwyn", |p| p.mana = 0).await;
    let cl = Classifier::new(&r).await;
    // (an attack spell by hand engages first and checks the mana with the round)
    let short = cmd(&mut c, "cast mend").await;
    assert!(
        cl.names(&short).iter().any(|n| n == "no_mana"),
        "no no_mana line:\n{short}"
    );
    assert!(!short.contains("Combat Engaged"), "{short}");
    edit_player(&r, "Thornwyn", |p| p.mana = 2).await;
    let t = ticker(&r, TICK);
    let at = c.position();
    c.send_line("vine rat").await.unwrap();
    // spent, then a round or more at MA=0, then a cast again on the regenerated mana
    c.until(
        r"(?s)MA=0\]:.*MA=0\]:.*You (?:cast|attempt to cast) vine strike",
        WAIT,
    )
    .await
    .unwrap();
    t.abort();
    let tail = cmd(&mut c, "").await;
    let out = c.plain_from(at);
    assert!(
        !out.contains("Combat Off"),
        "the fight stopped with the mana:\n{out}"
    );
    assert!(
        !out.contains("You do not have enough mana"),
        "the auto-combat recast is silent:\n{out}"
    );
    assert_clean(
        &cl,
        &format!("{short}{out}{tail}"),
        &["cast mend", "vine rat"],
    );
    r.stop().await.unwrap();
}

/// What `n` casts of `spell` at a fresh copy of `monster` show: the fail count and the
/// damage of every cast that landed. Run on the world thread with the server's seeded RNG,
/// each call with full energy and mana (the fixed test sheets: the level
/// and Spellcasting given). One call can cast more than once: the monster-target engine
/// loops while the energy lasts (`MAXTRIES`).
#[allow(clippy::too_many_arguments)]
async fn cast_many(
    r: &silvermere::Running,
    caster: &'static str,
    spell: SpellNo,
    monster: u16,
    room: (u32, u32),
    spellcast: i16,
    area: bool,
    n: usize,
) -> Casts {
    let m = spawn(r, room.0, room.1, monster).await;
    on_world(r, move |w| {
        let id = session_of(w, caster);
        let palette = silvermere_engine::tell::palette_of(w, id);
        let mut out = Casts::default();
        let re = regex::Regex::new(
            r"^(?:You cast .+ (?:at|on) .+ for|The .+ explodes, causing) (\d+) damage!$",
        )
        .unwrap();
        // the engines' resistance: the Magic Resistance ability plus the known monster's
        // `magicres`; an area cast shows one line
        // ("The fireball explodes, causing %d damage!") with the same multiplier
        let a = w
            .active_monsters
            .values()
            .find(|a| a.number == m)
            .unwrap()
            .clone();
        out.mr = silvermere_rules::spells::resist::monster_magic_resistance(
            silvermere_engine::combat::monster_ability(
                w,
                &a,
                silvermere_types::enums::Ability::MagicResist,
            ),
            w.monsters[&a.knownnum].magicres,
            None,
        );
        for _ in 0..n {
            if let Some(a) = w.active_monsters.values_mut().find(|a| a.number == m) {
                a.maxhp = 30000;
                a.hp = 30000;
            }
            {
                let p = w.session_mut(id).unwrap().character.as_mut().unwrap();
                p.energy = 1000;
                p.mana = p.maxmana as i16;
                p.spellcast = spellcast;
                p.settings.insert(PlayerSettings::CANCAST);
                p.hp = p.maxhp;
            }
            let target = (!area).then_some(AutoTarget::Monster(m));
            let _ = autocombat_cast(w, id, spell, target);
            let s = w.session_mut(id).unwrap();
            let raw = s.output.render(palette, b"\n");
            s.output.clear();
            let text = String::from_utf8_lossy(&silvermere_text::strip_ansi(&raw)).into_owned();
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if line.starts_with("You attempt to cast") {
                    out.fails += 1;
                } else if let Some(k) = re.captures(line) {
                    out.damage.push(k[1].parse().unwrap());
                } else {
                    out.other.push(line.to_owned());
                }
            }
        }
        // the fight the casts started is not the test's, nor is the monster (an area
        // spell would show the next run the line of whichever target comes first)
        if let Some(p) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.energy = 0;
        }
        silvermere_engine::monsters::active::remove_active_monster(w, m, false);
        out
    })
    .await
}

#[derive(Default)]
struct Casts {
    fails: usize,
    damage: Vec<i32>,
    other: Vec<String>,
    /// The monster's magic resistance (`magicres`; the stock targets have no MagicResist
    /// ability).
    mr: i32,
}

/// The 0.999 quantile of chi-square with `k` degrees of freedom (Wilson-Hilferty).
fn chi2_999(k: usize) -> f64 {
    let k = k as f64;
    let z = 3.09;
    k * (1.0 - 2.0 / (9.0 * k) + z * (2.0 / (9.0 * k)).sqrt()).powi(3)
}

/// Pearson's chi-square of the observed damage against the shown damage the rules predict:
/// the roll uniform on `[lo, hi]`, then `monster_mr_damage` with the target's resistance. Also
/// asserts that no value falls outside the prediction. Returns `(chi-square, degrees of freedom)`.
fn damage_fit(damage: &[i32], lo: i32, hi: i32, mr: i32) -> (f64, usize) {
    use std::collections::BTreeMap;

    let mut want: BTreeMap<i32, f64> = BTreeMap::new();
    for a in lo..=hi {
        let shown = silvermere_rules::spells::resist::monster_mr_damage(a, mr, false);
        *want.entry(shown).or_default() += 1.0 / f64::from(hi - lo + 1);
    }
    let mut got: BTreeMap<i32, f64> = BTreeMap::new();
    for d in damage {
        assert!(
            want.contains_key(d),
            "damage {d} cannot come from a roll in [{lo}, {hi}] at MR {mr}: {want:?}"
        );
        *got.entry(*d).or_default() += 1.0;
    }
    let n = damage.len() as f64;
    let x2 = want
        .iter()
        .map(|(v, p)| {
            let e = n * p;
            let o = got.get(v).copied().unwrap_or(0.0);
            (o - e) * (o - e) / e
        })
        .sum();
    (x2, want.len() - 1)
}

/// spell_fail: 200 vine strikes with Spellcasting 40 against vine strike's `diff` 15: the
/// failure rate is `1 - (40+15)/100` within four standard deviations.
#[tokio::test(flavor = "multi_thread")]
async fn cast_fail_rate_follows_spellcast_plus_diff() {
    let db = stock_db();
    let r = start(&db.url, 14).await;
    let _c = druid(&r, &db.url).await;
    place(&r, "Thornwyn", ARENA.0, ARENA.1).await;
    let c = cast_many(&r, "Thornwyn", VINE, GIANT_RAT, ARENA, 40, false, 200).await;
    let total = c.fails + c.damage.len();
    let p: f64 = 1.0 - 55.0 / 100.0;
    let sd = (total as f64 * p * (1.0 - p)).sqrt();
    let want = total as f64 * p;
    assert!(
        total >= 200 && (c.fails as f64 - want).abs() <= 4.0 * sd,
        "{} fails of {total}, want {want:.0} +/- {:.0}; other: {:?}",
        c.fails,
        4.0 * sd,
        &c.other[..c.other.len().min(5)]
    );
    r.stop().await.unwrap();
}

/// Statistical (07 layer 4): 5,000 vine strikes on the fixed level-2 druid sheet
/// (Spellcasting 50) at the giant rat and at the kobold thief. The damage histogram fits the
/// rules' prediction (chi-square under the 0.999 quantile) and the fail rate is within four
/// standard deviations of `1 - (spellcast+diff)/100`.
#[tokio::test(flavor = "multi_thread")]
async fn vine_strike_5000_casts_rat_and_kobold() {
    let db = stock_db();
    let r = start(&db.url, 15).await;
    let _c = druid(&r, &db.url).await;
    place(&r, "Thornwyn", ARENA.0, ARENA.1).await;
    let (lo, hi) = on_world(&r, |w| {
        let a = silvermere_rules::spells::amount_range(&w.spells[&VINE], 2);
        (a.min, a.max)
    })
    .await;
    for monster in [GIANT_RAT, KOBOLD_THIEF] {
        let c = cast_many(&r, "Thornwyn", VINE, monster, ARENA, 50, false, 5000).await;
        let total = c.fails + c.damage.len();
        let p: f64 = 1.0 - 65.0 / 100.0;
        let sd = (total as f64 * p * (1.0 - p)).sqrt();
        assert!(
            total >= 5000 && (c.fails as f64 - total as f64 * p).abs() <= 4.0 * sd,
            "monster {monster}: {} fails of {total}",
            c.fails
        );
        let (x2, df) = damage_fit(&c.damage, lo, hi, c.mr);
        assert!(
            x2 < chi2_999(df),
            "monster {monster}: chi-square {x2:.2} with {df} df"
        );
    }
    r.stop().await.unwrap();
}

/// A mage made level 15 the way the scenario says (`sysop tweak level 15`, T-055; the
/// DEBUG grant stands in for `CODEC_OF_TWINKS`), fireball in the book.
async fn mage(r: &silvermere::Running, url: &str) -> silvermere_testkit::TelnetClient {
    let mut c = new_character(r, url, "Merla", MAGE, &[Role::Sysop]).await;
    on_world(r, |w| {
        let id = session_of(w, "Merla");
        w.session_mut(id)
            .unwrap()
            .sysop_keys
            .insert(silvermere_engine::commands::sysop::Key::Debug);
    })
    .await;
    let out = cmd(&mut c, "sysop tweak level 15").await;
    assert!(!out.contains("not recognized"), "{out}");
    edit_player(r, "Merla", |p| {
        assert_eq!(p.level, 15, "sysop tweak level");
        p.spellbook = vec![FBAL];
        p.maxmana = 200;
        p.mana = 200;
        p.maxhp = 2000;
        p.hp = 2000;
        p.spellcast = 200;
    })
    .await;
    c
}

/// spell_aoe: `cast fbal` with three dark goblins in the room: one cast, one damage line per
/// goblin, and the fight is on with all of them (area spells are never resisted).
#[tokio::test(flavor = "multi_thread")]
async fn fireball_hits_three_goblins() {
    let db = stock_db();
    let r = start(&db.url, 16).await;
    let mut c = mage(&r, &db.url).await;
    place(&r, "Merla", 1, 424).await;
    for _ in 0..3 {
        spawn(&r, 1, 424, DARK_GOBLIN).await;
    }
    let cl = Classifier::new(&r).await;
    cmd(&mut c, "").await;
    let t = ticker(&r, TICK);
    let at = c.position();
    c.send_line("cast fbal").await.unwrap();
    // by hand an area attack spell engages; the cast comes with the round
    c.until(r"(?s)fireball.*damage!.*\[HP=", WAIT)
        .await
        .unwrap();
    t.abort();
    let tail = cmd(&mut c, "").await;
    let out = c.plain_from(at);
    let hurt = on_world(&r, |w| {
        w.active_monsters
            .values()
            .filter(|a| a.room.0 == 424 && a.hp < a.maxhp)
            .count()
    })
    .await;
    assert_eq!(hurt, 3, "the fireball reaches all three goblins:\n{out}");
    assert!(out.contains("You throw a fireball!"), "{out}");
    assert!(out.contains("The fireball explodes, causing"), "{out}");
    assert!(
        !out.contains("resisted"),
        "area spells are never resisted:\n{out}"
    );
    assert!(out.contains("*Combat Engaged*"), "{out}");
    assert_clean(&cl, &format!("{out}{tail}"), &["cast fbal"]);
    r.stop().await.unwrap();
}

/// Statistical: 5,000 fireballs from the fixed level-15 mage at the giant rat and the
/// kobold thief: damage uniform on fireball's level-15 range, never resisted.
#[tokio::test(flavor = "multi_thread")]
async fn fireball_5000_casts_rat_and_kobold() {
    let db = stock_db();
    let r = start(&db.url, 17).await;
    let _c = mage(&r, &db.url).await;
    place(&r, "Merla", ARENA.0, ARENA.1).await;
    let (lo, hi) = on_world(&r, |w| {
        let a = silvermere_rules::spells::amount_range(&w.spells[&FBAL], 15);
        (a.min, a.max)
    })
    .await;
    for monster in [GIANT_RAT, KOBOLD_THIEF] {
        let c = cast_many(&r, "Merla", FBAL, monster, ARENA, 200, true, 5000).await;
        // Spellcasting 200 meets the cap: Spellcasting plus difficulty, at most 98; 2% fail
        let total = c.fails + c.damage.len();
        let sd = (total as f64 * 0.02 * 0.98).sqrt();
        assert!(
            (c.fails as f64 - total as f64 * 0.02).abs() <= 4.0 * sd,
            "{} fails of {total}, want 2%",
            c.fails
        );
        assert!(
            c.other.iter().all(|l| !l.contains("resist")),
            "an area spell was resisted: {:?}",
            &c.other[..c.other.len().min(5)]
        );
        assert_eq!(
            total,
            5000,
            "one cast per call: {:?}",
            &c.other[..c.other.len().min(5)]
        );
        let (x2, df) = damage_fit(&c.damage, lo, hi, c.mr);
        assert!(
            x2 < chi2_999(df),
            "monster {monster}: chi-square {x2:.2} with {df} df"
        );
    }
    r.stop().await.unwrap();
}

/// monster_cast: an orc shaman in the Torchlit Cave casts at the player over a fight: some
/// casts land (`cast_on_you_generic` or its WCCMSG text), and every line is classified.
#[tokio::test(flavor = "multi_thread")]
async fn orc_shaman_casts_at_the_player() {
    let db = stock_db();
    let r = start(&db.url, 18).await;
    let mut c = druid(&r, &db.url).await;
    edit_player(&r, "Thornwyn", |p| {
        p.maxhp = 30000;
        p.hp = 30000;
    })
    .await;
    place(&r, "Thornwyn", TORCHLIT_CAVE.0, TORCHLIT_CAVE.1).await;
    let m = spawn(&r, TORCHLIT_CAVE.0, TORCHLIT_CAVE.1, ORC_SHAMAN).await;
    on_world(&r, move |w| {
        if let Some(a) = w.active_monsters.values_mut().find(|a| a.number == m) {
            a.hp = 30000;
            a.maxhp = 30000;
        }
    })
    .await;
    let cl = Classifier::new(&r).await;
    cmd(&mut c, "").await;
    let t = ticker(&r, TICK);
    let at = c.position();
    c.send_line("attack orc").await.unwrap();
    // long enough for a few dozen rounds
    tokio::time::sleep(Duration::from_secs(8)).await;
    t.abort();
    let tail = cmd(&mut c, "").await;
    let out = c.plain_from(at);
    let names = cl.names(&out);
    let casts = names
        .iter()
        .filter(|n| {
            matches!(
                n.as_str(),
                "cast_on_you_generic" | "you_resisted" | "cast_at_you_failed"
            )
        })
        .count();
    let wccmsg_casts = out
        .lines()
        .filter(|l| l.contains("orc shaman") && (l.contains("cast") || l.contains("spell")))
        .count();
    assert!(
        casts + wccmsg_casts > 0,
        "no cast from the orc shaman in {} rounds:\n{out}",
        names.iter().filter(|n| *n == "prompt").count()
    );
    assert_clean(&cl, &format!("{out}{tail}"), &["attack orc"]);
    r.stop().await.unwrap();
}

/// room_spell: the Lava Pool's magma heat (roomspell 526) reaches a player standing in it
/// on every other medium cycle: one effect line per 6 fast ticks
/// (`altpass`).
#[tokio::test(flavor = "multi_thread")]
async fn lava_pool_room_spell_every_six_ticks() {
    let db = stock_db();
    let r = start(&db.url, 19).await;
    let mut c = druid(&r, &db.url).await;
    edit_player(&r, "Thornwyn", |p| {
        p.maxhp = 30000;
        p.hp = 30000;
    })
    .await;
    place(&r, "Thornwyn", LAVA_POOL.0, LAVA_POOL.1).await;
    let cl = Classifier::new(&r).await;
    cmd(&mut c, "").await;
    let at = c.position();
    tick(&r, 60).await;
    let out = cmd(&mut c, "").await;
    // the 60 ticks' output is still arriving when the first reply and prompt are in
    let t0 = std::time::Instant::now();
    while c
        .plain_from(at)
        .lines()
        .filter(|l| l.contains("damage"))
        .count()
        < 10
        && t0.elapsed() < WAIT
    {
        pump(&mut c, 20).await;
    }
    let all = c.plain_from(at);
    let hits = all.lines().filter(|l| l.contains("damage")).count();
    // 60 fed ticks plus at most a few from the real 1 s timer
    assert!(
        (10..=12).contains(&hits),
        "{hits} room-spell lines in 60+ ticks:\n{all}"
    );
    assert_clean(&cl, &format!("{all}{out}"), &[""]);
    r.stop().await.unwrap();
}
