//! The `a|b|c` component picker.

use rand::{Rng as _, SeedableRng};
use rand_chacha::ChaCha8Rng;
use silvermere_text::component::{COMPSIZ, MAXALTS, NOT_AVAILABLE, Picked, Rng, Which, pick};

/// A seeded generator for the random path.
struct Seeded(ChaCha8Rng);

impl Rng for Seeded {
    fn below(&mut self, n: u8) -> u8 {
        assert!(n >= 1, "genrdn(0,0) is never asked for");
        (self.0.next_u32() % u32::from(n)) as u8
    }
}

fn seeded() -> Seeded {
    Seeded(ChaCha8Rng::seed_from_u64(0x1010))
}

// which counts from 1
#[test]
fn index_picks_that_alternative() {
    assert_eq!(
        pick(Some("a|b|c"), Which::Index(2), &mut seeded()),
        Picked {
            text: "b".into(),
            chosen: 2
        }
    );
    assert_eq!(
        pick(Some("a|b|c"), Which::Index(1), &mut seeded()),
        Picked {
            text: "a".into(),
            chosen: 1
        }
    );
    assert_eq!(
        pick(Some("a|b|c"), Which::Index(3), &mut seeded()),
        Picked {
            text: "c".into(),
            chosen: 3
        }
    );
}

// at most MAXALTS alternatives; the text after the fifth '|' is
// never reachable
#[test]
fn maxalts_caps_the_alternatives() {
    assert_eq!(MAXALTS, 5);
    let text = "a|b|c|d|e|f";
    assert_eq!(pick(Some(text), Which::Index(5), &mut seeded()).text, "e");
    // the sixth shows nothing in the original: empty here
    assert_eq!(
        pick(Some(text), Which::Index(6), &mut seeded()),
        Picked {
            text: String::new(),
            chosen: 6
        }
    );
    // and the random path never draws it
    let mut rng = seeded();
    for _ in 0..200 {
        assert_ne!(pick(Some(text), Which::Random, &mut rng).text, "f");
    }
}

// a missing text gives N/A
#[test]
fn null_text_gives_not_available() {
    assert_eq!(
        pick(None, Which::Index(1), &mut seeded()),
        Picked {
            text: NOT_AVAILABLE.into(),
            chosen: 0
        }
    );
    assert_eq!(pick(None, Which::Random, &mut seeded()).text, "N/A");
}

// the copy stops at COMPSIZ-1 characters
#[test]
fn result_is_cut_at_compsiz_minus_one() {
    assert_eq!(COMPSIZ, 40);
    let long = "x".repeat(60);
    let text = format!("{long}|short");
    let p = pick(Some(&text), Which::Index(1), &mut seeded());
    assert_eq!(p.text.len(), COMPSIZ - 1);
    assert_eq!(p.text, "x".repeat(39));
    assert_eq!(
        pick(Some(&text), Which::Index(2), &mut seeded()).text,
        "short"
    );
}

// which 0 draws at random, 0..cnt-1 like the host's genrdn
#[test]
fn random_hits_every_alternative() {
    let mut rng = seeded();
    let mut seen = [0u32; 5];
    for _ in 0..500 {
        let p = pick(Some("one|two|three|four|five"), Which::Random, &mut rng);
        let expected = ["one", "two", "three", "four", "five"][usize::from(p.chosen - 1)];
        assert_eq!(p.text, expected);
        seen[usize::from(p.chosen - 1)] += 1;
    }
    assert!(
        seen.iter().all(|&n| n > 0),
        "every alternative drawn: {seen:?}"
    );
    // a single alternative is always it
    assert_eq!(
        pick(Some("only"), Which::Random, &mut rng),
        Picked {
            text: "only".into(),
            chosen: 1
        }
    );
}

// chosen lets a second call pick the matching part of another text
#[test]
fn chosen_number_selects_the_matching_part_of_another_text() {
    let mut rng = seeded();
    for _ in 0..20 {
        let first = pick(Some("sword|axe|mace"), Which::Random, &mut rng);
        let second = pick(
            Some("slash|chop|crush"),
            Which::Index(first.chosen),
            &mut rng,
        );
        let pair = match first.text.as_str() {
            "sword" => "slash",
            "axe" => "chop",
            "mace" => "crush",
            other => panic!("unexpected {other}"),
        };
        assert_eq!(second.text, pair);
        assert_eq!(second.chosen, first.chosen);
    }
}
