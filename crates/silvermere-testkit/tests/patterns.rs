//! The pattern registry loader and classifier against `tests/fixtures/patterns.json`.

use std::collections::BTreeSet;

use silvermere_testkit::captures::{Capture, captures_dir};
use silvermere_testkit::patterns::{PatternSet, PromptMode};

fn set() -> PatternSet {
    PatternSet::load_default().expect("patterns.json loads")
}

#[test]
fn every_rx_compiles_and_the_count_matches() {
    let set = set();
    assert_eq!(set.patterns().len(), set.declared_static_count());
    assert_eq!(set.patterns().len(), 406);
    assert_eq!(set.tables().count(), 8);
}

// patterns.py selftest: every example without `...` classifies as its own pattern.
#[test]
fn every_example_matches_its_own_pattern() {
    let set = set();
    let mut checked = 0;
    for p in set.patterns() {
        let ex = p.example.trim();
        if ex.is_empty() || ex.contains("...") || p.seen == "I" {
            continue;
        }
        checked += 1;
        let (hit, _) = set
            .match_line(ex)
            .unwrap_or_else(|| panic!("{}: example {ex:?} matches nothing", p.name));
        assert!(
            hit.name == p.name || hit.prio < p.prio || hit.cat == p.cat,
            "{}: example {ex:?} classified as {} ({} / {})",
            p.name,
            hit.name,
            hit.cat,
            hit.prio
        );
    }
    assert!(checked > 350, "only {checked} examples checked");
}

#[test]
fn match_order_is_priority_then_specificity() {
    let set = set();
    let (p, f) = set.match_line("You gain 120 experience.").unwrap();
    assert_eq!(p.name, "exp_gain");
    assert_eq!(f["exp"], "120");
    // without game data the monster-hit family is not loaded: prio 52 `other_hits` takes
    // the line before the prio 90 `mon_hits_you_generic` (patterns.py load_static)
    let (p, f) = set.match_line("The orc slashes you for 7 damage!").unwrap();
    assert_eq!((p.name.as_str(), f["dmg"].as_str()), ("other_hits", "7"));
    let (p, _) = set.match_line("You took 3 damage!").unwrap();
    assert_eq!(p.name, "took_damage");
    assert!(set.match_line("zzzz qqqq no such line").is_none());
}

#[test]
fn prompts_are_peeled_and_echo_is_recognised() {
    let set = set();
    let lines = ["[HP=30/MA=12]: n", "Obvious exits: north, south"];
    let events = set.classify_lines_sent(&lines, &["n".to_owned()]);
    let names: Vec<_> = events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["prompt", "echo", "room_exits"]);
    assert_eq!(events[0].fields["hp"], "30");
    assert_eq!(events[0].fields["mana"], "12");
    assert_eq!(events[0].fields["mtype"], "MA");
    let resting = set.classify_lines(&["[HP=5 (Resting) ]:"]);
    assert_eq!(resting[0].fields["state"], "Resting");
}

// patterns.py GENERIC_PROMPT uses a lookahead: it compiles through fancy-regex.
#[test]
fn generic_prompt_uses_the_lookahead_pattern() {
    let mut set = set();
    set.set_prompt_mode(PromptMode::Auto);
    let events = set.classify_lines(&["[hp=1/2,MA=8/42,CASH=180750,WARNS=Off]:"]);
    assert_eq!(events[0].name, "prompt");
    assert_eq!(events[0].fields["form"], "generic");
    assert_eq!(events[0].fields["maxhp"], "2");
    assert_eq!(events[0].fields["wealth"], "180750");
}

#[test]
fn wrapped_exits_are_joined_and_tables_take_rows() {
    let set = set();
    let events = set.classify_lines(&[
        "Obvious exits: north, south,",
        "east, closed door",
        "west",
        "[MAJORMUD]:",
    ]);
    assert_eq!(events[0].name, "room_exits");
    assert_eq!(
        events[0].fields["exits"],
        "north, south, east, closed door west"
    );
}

fn unknown_lines(set: &PatternSet, c: &Capture) -> BTreeSet<String> {
    set.classify_transcript(&c.plain)
        .into_iter()
        .filter(|e| e.name == "unknown")
        .map(|e| e.line)
        .collect()
}

// capture README "Unclassified lines": with static patterns only, the unknown lines are
// the with-data block plus the extra block, exactly.
#[test]
fn unknown_lines_in_each_capture_are_the_ones_the_readme_lists() {
    let set = set();
    let captures = Capture::all(captures_dir()).unwrap();
    if captures.is_empty() {
        eprintln!("skipped: no captures under tests/captures/");
        return;
    }
    for c in &captures {
        let (with_data, extra) = c.listed_unknown();
        let want: BTreeSet<String> = with_data.union(&extra).cloned().collect();
        let got = unknown_lines(&set, c);
        assert_eq!(got.len(), want.len(), "{}: unknown count", c.name);
        assert_eq!(got, want, "{}", c.name);
    }
}
