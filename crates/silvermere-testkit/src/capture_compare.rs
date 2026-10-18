//! Compare what the server sent with a T-005 capture (layer 4 "Normalisation").
//!
//! Output is split on `ESC[79D ESC[K` and CR/LF (`captures::text_to_lines`, the same cut as
//! `patterns.raw_to_lines`), every field the plan calls random is masked (the capture's own
//! `mask` list: build stamp, time-left suffix, the `X\b` direction bytes, clocks; plus the
//! caller's extra masks), and the result is compared two ways: the classified event sequence
//! (names and unmasked fields) always, and the masked lines byte for byte when the scenario
//! is deterministic.

use crate::captures::{Capture, ScriptMask, apply_script_masks, text_to_lines};
use crate::normalise::{Difference, MaskSpec, diff_events, normalise};
use crate::patterns::{Event, PatternSet, Rx};

/// What the capture's step `index` made the server send: its raw bytes, from the step's
/// recorded offset to the next step's (or to the end of `raw.bin`).
pub fn step_output(capture: &Capture, index: usize) -> Vec<u8> {
    let steps = capture.script["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let offset = |i: usize| -> Option<usize> {
        usize::try_from(steps.get(i)?["result"]["offset"].as_u64()?).ok()
    };
    let start = offset(index).unwrap_or(0);
    let end = (index + 1..steps.len())
        .find_map(offset)
        .unwrap_or(capture.raw.len());
    capture.raw[start.min(capture.raw.len())..end.min(capture.raw.len())].to_vec()
}

/// The capture's `mask` list as [`ScriptMask`]s, plus `extra` regexes.
pub fn masks_for(capture: &Capture, extra: &[&str]) -> Vec<ScriptMask> {
    let mut masks = capture.script_masks();
    masks.extend(extra.iter().map(|r| ScriptMask {
        regex: (*r).to_owned(),
        why: "caller".to_owned(),
    }));
    masks
}

/// Decodes `raw` (CP437, telnet already stripped), masks, and cuts into lines.
pub fn masked_lines(raw: &[u8], masks: &[ScriptMask]) -> Vec<String> {
    let text = silvermere_net::cp437::decode(raw);
    let lines = text_to_lines(&apply_script_masks(&text, masks));
    // A line the capture masks whole (`Also here: .*`, a monster's arrival) is a monster
    // line, and so is the noise of one in the next room (`You hear movement to the north.`,
    // which T-035's monsters now make on this side too, with the prompt it is sent with and
    // the line break that prompt's ANSI_CLRLINE starts): all dropped on both sides. Monster
    // timing is random on both servers; the differential's m2 scenarios compare its rate
    // (`tools/differential/random_events.json`, `stats.py`), not single lines.
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut it = lines.into_iter().peekable();
    while let Some(line) = it.next() {
        if line == MASKED_LINE {
            continue;
        }
        if line.starts_with(MONSTER_NOISE) {
            // the prompt follows, behind the break its own ANSI_CLRLINE starts
            let mut ahead = it.clone();
            let blank = ahead.next_if(String::is_empty).is_some();
            if ahead.peek().is_some_and(|next| is_prompt_remnant(next)) {
                it = ahead;
                it.next();
            } else if !blank && it.peek().is_some_and(|next| is_prompt_remnant(next)) {
                it.next();
            }
            continue;
        }
        out.push(line);
    }
    out
}

/// The start of the line a monster in the next room makes.
const MONSTER_NOISE: &str = "You hear movement to the ";

/// A prompt left over after masking: `[HP=35]:` with the HP masked or not.
fn is_prompt_remnant(line: &str) -> bool {
    line.ends_with("]:")
        && (line.starts_with("<masked>") || line.starts_with("[HP=") || line == "]:")
}

/// What a line that a mask replaced entirely looks like after masking.
const MASKED_LINE: &str = "<masked>";

/// The first line at which two line lists differ, as a message with a little context.
pub fn first_line_difference(expected: &[String], actual: &[String]) -> Option<String> {
    for i in 0..expected.len().max(actual.len()) {
        let (e, a) = (expected.get(i), actual.get(i));
        if e != a {
            let show = |l: Option<&String>| l.map_or("<end>".to_owned(), |l| format!("{l:?}"));
            return Some(format!(
                "line {i}: expected {} but got {}\n  expected tail: {:?}\n  actual tail:   {:?}",
                show(e),
                show(a),
                &expected[i.saturating_sub(2)..(i + 3).min(expected.len())],
                &actual[i.saturating_sub(2)..(i + 3).min(actual.len())],
            ));
        }
    }
    None
}

/// Byte-for-byte comparison after masking: `Err` carries the first differing line.
pub fn compare_bytes(expected: &[u8], actual: &[u8], masks: &[ScriptMask]) -> Result<(), String> {
    let (e, a) = (masked_lines(expected, masks), masked_lines(actual, masks));
    match first_line_difference(&e, &a) {
        None => Ok(()),
        Some(msg) => Err(msg),
    }
}

/// Like [`compare_bytes`] but with the ANSI colour sequences kept: only the line cut (CR LF)
/// and the masks apply. `Err` carries the first differing line, escapes made visible.
pub fn compare_with_colour(
    expected: &[u8],
    actual: &[u8],
    masks: &[ScriptMask],
) -> Result<(), String> {
    let cut = |raw: &[u8]| -> Vec<String> {
        let text = apply_script_masks(&silvermere_net::cp437::decode(raw), masks);
        let lines: Vec<String> = text
            .replace("\r\n", "\n")
            .split('\n')
            .map(|l| l.replace('\x1b', "<ESC>"))
            .collect();
        // a line the capture masks whole is a monster line: drop it (its colour codes too)
        // (its terminator's reset, `ESC[0m`, leads the next line: dropped with it)
        let mut kept = Vec::with_capacity(lines.len());
        let mut dropped = false;
        let mut after_noise = false;
        for mut l in lines {
            // a monster's noise in the next room, and the prompt it is sent with (the room
            // follows the prompt on the same line: no CR LF between them in colour)
            if visible(&l).starts_with(MONSTER_NOISE) {
                after_noise = true;
                continue;
            }
            if std::mem::take(&mut after_noise)
                && let Some(end) = l.find("]:")
                && l[..end].contains("<masked>")
            {
                l = l[end + 2..].to_owned();
                if l.is_empty() {
                    continue;
                }
            }
            if l.contains("<masked>") && visible(&l).is_empty() {
                dropped = true;
                continue;
            }
            kept.push(if dropped {
                l.strip_prefix("<ESC>[0m").unwrap_or(&l).to_owned()
            } else {
                l
            });
            dropped = false;
        }
        kept
    };
    match first_line_difference(&cut(expected), &cut(actual)) {
        None => Ok(()),
        Some(msg) => Err(msg),
    }
}

/// What is left of a colour-visible line once the escapes (`<ESC>[...m`) and masks are gone.
fn visible(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line.replace("<masked>", "");
    while let Some(i) = rest.find("<ESC>[") {
        out.push_str(&rest[..i]);
        match rest[i + 6..].find(|c: char| c.is_ascii_alphabetic()) {
            Some(j) => rest = rest[i + 6 + j + 1..].to_owned(),
            None => {
                rest.clear();
                break;
            }
        }
    }
    out.push_str(&rest);
    out.trim().to_owned()
}

/// Classifies masked lines with the pattern registry and applies the field masks.
pub fn events_of(lines: &[String], patterns: &PatternSet, fields: &MaskSpec) -> Vec<Event> {
    normalise(&patterns.classify_lines(lines), fields)
}

/// Event-sequence comparison; the empty list means equal.
pub fn compare_events(
    expected: &[u8],
    actual: &[u8],
    masks: &[ScriptMask],
    patterns: &PatternSet,
    fields: &MaskSpec,
) -> Vec<Difference> {
    let e = events_of(&masked_lines(expected, masks), patterns, fields);
    let a = events_of(&masked_lines(actual, masks), patterns, fields);
    diff_events(&e, &a)
}

/// Both comparisons, for a deterministic scenario step; panics with a readable message.
pub fn assert_matches_capture(
    what: &str,
    capture: &Capture,
    step: usize,
    actual: &[u8],
    extra_masks: &[&str],
) {
    let masks = masks_for(capture, extra_masks);
    let expected = step_output(capture, step);
    assert!(
        !expected.is_empty(),
        "capture {} step {step} has no output",
        capture.name
    );
    assert!(!actual.is_empty(), "{what}: the server sent nothing");
    let patterns = PatternSet::load_default().expect("patterns.json");
    let fields = MaskSpec::load_default().expect("masks.json");
    let diffs = compare_events(&expected, actual, &masks, &patterns, &fields);
    assert!(
        diffs.is_empty(),
        "{what}: event sequences differ: {diffs:?}"
    );
    if let Err(msg) = compare_bytes(&expected, actual, &masks) {
        panic!(
            "{what}: bytes differ from capture {} step {step}: {msg}",
            capture.name
        );
    }
}

/// Whether `text` matches `regex` (helper for tests that mask ad hoc).
pub fn matches(regex: &str, text: &str) -> bool {
    Rx::new(regex).is_ok_and(|r| r.is_match(text))
}

/// One replayed step: what the capture and the server sent.
#[derive(Debug)]
pub struct StepReport {
    pub index: usize,
    pub sent: String,
    /// `None` when the event sequence and the masked bytes both equal the capture.
    pub problem: Option<String>,
}

/// Replays steps `range` of `capture` on `client` (already in the state the capture was in
/// before `range.start`), waiting for each step's `until` regex, and compares each step's
/// output with the capture's. `nowait` steps and steps with variables are not replayed.
pub async fn replay_steps(
    client: &mut crate::client::TelnetClient,
    capture: &Capture,
    range: std::ops::Range<usize>,
    extra_masks: &[&str],
    timeout: std::time::Duration,
) -> Vec<StepReport> {
    let steps = capture.script["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let masks = masks_for(capture, extra_masks);
    let patterns = PatternSet::load_default().expect("patterns.json");
    let fields = MaskSpec::load_default().expect("masks.json");
    let mut out = Vec::new();
    for index in range {
        let st = &steps[index];
        let sent = match (st["send"].as_str(), st["raw"].as_str()) {
            (Some(s), _) if !s.contains('{') => s.to_owned(),
            (None, Some(_)) | (None, None) => String::new(),
            _ => continue,
        };
        if st["nowait"].as_bool().unwrap_or(false) || st["op"].as_str().unwrap_or("send") != "send"
        {
            continue;
        }
        let until = st["until"].as_str().unwrap_or_default();
        let at = client.position();
        let res = if st["raw"].as_str().is_some() {
            client
                .send_raw(
                    crate::captures::unescape(st["raw"].as_str().unwrap_or_default()).as_bytes(),
                )
                .await
        } else {
            client.send_line(&crate::captures::unescape(&sent)).await
        };
        let mut problem = res.err().map(|e| e.to_string());
        if problem.is_none() && !until.is_empty() {
            if let Err(e) = client.until(until, timeout).await {
                problem = Some(format!("waiting for {until:?}: {e}"));
            }
        } else if problem.is_none() {
            // no `until`: the capture waited for this much quiet (default one second)
            let quiet = st["wait"].as_f64().unwrap_or(1.0);
            let _ = client.read(std::time::Duration::from_secs_f64(quiet)).await;
        }
        if problem.is_none() {
            let actual = client.raw()[at..].to_vec();
            let expected = step_output(capture, index);
            let diffs = compare_events(&expected, &actual, &masks, &patterns, &fields);
            // text first; then, with the ANSI colours kept, the escapes too
            let bytes = compare_bytes(&expected, &actual, &masks).err().or_else(|| {
                compare_with_colour(&expected, &actual, &masks)
                    .err()
                    .map(|m| format!("colour: {m}"))
            });
            problem = if !diffs.is_empty() {
                let first = diffs.first().map(|d| format!("{d:?}")).unwrap_or_default();
                Some(format!(
                    "{} event differences, first {first}; {}",
                    diffs.len(),
                    bytes.unwrap_or_default()
                ))
            } else {
                bytes
            };
        }
        out.push(StepReport {
            index,
            sent,
            problem,
        });
    }
    out
}

/// Panics listing every step of `reports` that differs.
pub fn assert_clean(what: &str, reports: &[StepReport]) {
    let bad: Vec<String> = reports
        .iter()
        .filter_map(|r| {
            r.problem
                .as_ref()
                .map(|p| format!("step {} ({:?}): {p}", r.index, r.sent))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{what}: {} steps differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
    assert!(!reports.is_empty(), "{what}: no steps were replayed");
}
