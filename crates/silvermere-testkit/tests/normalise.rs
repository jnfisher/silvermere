//! Event normalisation and comparison.

use silvermere_testkit::normalise::{Difference, MASKED, MaskSpec};
use silvermere_testkit::{PatternSet, diff_events, normalise};

fn classify(lines: &[&str]) -> Vec<silvermere_testkit::Event> {
    PatternSet::load_default().unwrap().classify_lines(lines)
}

fn masks() -> MaskSpec {
    MaskSpec::load_default().unwrap()
}

#[test]
fn damage_numbers_and_the_build_stamp_are_masked() {
    let a = classify(&[
        "M A J O R  M U D v1.11p-WG (Dec 30 2005 14:20:05)",
        "You smack rat for 7 damage!",
        "You gain 120 experience.",
        "You took 3 damage!",
        "[E] . Enter the Realm [14 game minutes left]",
        "There are 3 users in the game.",
    ]);
    let b = classify(&[
        "M A J O R  M U D v1.11p-WG (Jan 02 2026 09:01:44)",
        "You smack rat for 19 damage!",
        "You gain 7 experience.",
        "You took 11 damage!",
        "[E] . Enter the Realm [3 game minutes left]",
        "There are 12 users in the game.",
    ]);
    assert_ne!(a, b, "raw events differ");
    let (a, b) = (normalise(&a, &masks()), normalise(&b, &masks()));
    assert!(diff_events(&a, &b).is_empty(), "{:?}", diff_events(&a, &b));
    assert_eq!(a[1].fields["dmg"], MASKED);
    assert_eq!(
        a[1].fields["target"], "rat",
        "unmasked fields keep their value"
    );
    assert!(a[1].masked.contains("dmg"));
}

#[test]
fn a_different_monster_or_wording_is_not_masked() {
    let a = normalise(&classify(&["You smack rat for 7 damage!"]), &masks());
    let b = normalise(&classify(&["You smack bat for 7 damage!"]), &masks());
    let d = diff_events(&a, &b);
    assert!(
        matches!(&d[..], [Difference::Field { field, .. }] if field == "target"),
        "{d:?}"
    );
}

#[test]
fn obvious_exits_wording_is_caught() {
    let a = normalise(&classify(&["Obvious exits: north, south"]), &masks());
    let b = normalise(&classify(&["Obvious exits: north, up"]), &masks());
    let d = diff_events(&a, &b);
    assert!(
        matches!(&d[..], [Difference::Field { field, left, right, .. }]
            if field == "exits" && left.as_deref() == Some("north, south") && right.as_deref() == Some("north, up")),
        "{d:?}"
    );
}

#[test]
fn different_names_and_lengths_are_reported() {
    let a = normalise(
        &classify(&["You gain 5 experience.", "Obvious exits: north"]),
        &masks(),
    );
    let b = normalise(&classify(&["Obvious exits: north"]), &masks());
    let d = diff_events(&a, &b);
    assert!(matches!(&d[0], Difference::Name { index: 0, .. }), "{d:?}");
    assert!(
        matches!(
            &d[1],
            Difference::Missing {
                index: 1,
                extra_in: "left",
                ..
            }
        ),
        "{d:?}"
    );
}

#[test]
fn lives_are_masked_only_when_the_script_says() {
    let a = classify(&["You now have 3 lives left."]);
    let b = classify(&["You now have 2 lives left."]);
    let plain = masks();
    assert!(!diff_events(&normalise(&a, &plain), &normalise(&b, &plain)).is_empty());
    let scripted = masks().with_script_events();
    assert!(diff_events(&normalise(&a, &scripted), &normalise(&b, &scripted)).is_empty());
}
