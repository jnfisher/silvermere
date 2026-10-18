//! The `a|b|c` picker that the room, item and monster texts use for their alternative
//! wordings.

/// The most `|` alternatives a component holds. Later ones
/// are never reachable.
pub const MAXALTS: usize = 5;

/// The longest result the game shows is `COMPSIZ - 1`, 39 characters.
pub const COMPSIZ: usize = 40;

/// The text the picker gives for a missing text.
pub const NOT_AVAILABLE: &str = "N/A";

/// Which alternative to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// `0`: one at random, drawn with the host's `genrdn`.
    Random,
    /// `1` up: that alternative. Out of range shows nothing in the original; it gives
    /// an empty string here.
    Index(u8),
}

/// The random source the picker needs. Placeholder for the engine's `Rng` trait;
/// T-015 reconciles the two.
pub trait Rng {
    /// A uniform integer in `0..n` for `n >= 1`, like the host's `genrdn(0,n)`, which
    /// never gives `n`.
    fn below(&mut self, n: u8) -> u8;
}

/// The picked alternative and its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    /// The alternative, cut at `COMPSIZ-1` characters.
    pub text: String,
    /// The number of the alternative taken, 1 up, so a
    /// second call can pick the matching part of another text. 0 for a `None` text.
    pub chosen: u8,
}

/// Splits `text` at `|` into at most `MAXALTS` alternatives.
/// The last one stops at the next `|` like the others, so text after the fifth `|` is
/// unreachable.
fn alternatives(text: &str) -> Vec<&str> {
    let mut alts = Vec::with_capacity(MAXALTS);
    for part in text.split('|') {
        if alts.len() == MAXALTS {
            break;
        }
        alts.push(part);
    }
    alts
}

/// Picks an alternative of a component text.
///
/// `None` gives `"N/A"`. `Which::Index(n)` takes alternative `n` (1 up);
/// `Which::Random` draws one with `rng`. The result is cut at `COMPSIZ-1` characters
/// (the original counted bytes; they agree for the game's CP437 text). An index past the
/// last alternative, which shows nothing in the original, returns an empty string and logs
/// a warning.
pub fn pick(text: Option<&str>, which: Which, rng: &mut dyn Rng) -> Picked {
    let Some(text) = text else {
        return Picked {
            text: NOT_AVAILABLE.to_string(),
            chosen: 0,
        };
    };
    let alts = alternatives(text);
    let cnt = u8::try_from(alts.len()).expect("at most MAXALTS alternatives");
    let index = match which {
        Which::Index(n) if n != 0 => n - 1,
        _ => rng.below(cnt),
    };
    let chosen = index + 1;
    let Some(alt) = alts.get(usize::from(index)) else {
        tracing::warn!(
            which = chosen,
            alternatives = cnt,
            text,
            "component index past the last alternative; empty result"
        );
        return Picked {
            text: String::new(),
            chosen,
        };
    };
    Picked {
        text: alt.chars().take(COMPSIZ - 1).collect(),
        chosen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(u8);
    impl Rng for Fixed {
        fn below(&mut self, n: u8) -> u8 {
            assert!(n >= 1);
            self.0 % n
        }
    }

    #[test]
    fn splits_at_most_maxalts() {
        assert_eq!(alternatives("a|b|c"), ["a", "b", "c"]);
        assert_eq!(alternatives("a|b|c|d|e|f|g"), ["a", "b", "c", "d", "e"]);
        assert_eq!(alternatives(""), [""]);
    }

    #[test]
    fn random_draws_below_the_count() {
        let p = pick(Some("a|b|c"), Which::Random, &mut Fixed(2));
        assert_eq!(
            p,
            Picked {
                text: "c".into(),
                chosen: 3
            }
        );
        let p = pick(Some("a|b|c"), Which::Random, &mut Fixed(3));
        assert_eq!(p.chosen, 1);
    }
}
