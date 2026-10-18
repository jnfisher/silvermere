//! A scripted in-process run observed through `metrics-util`'s `DebuggingRecorder`: every
//! metric of `docs/metrics.md` is emitted at least once, and labels stay bounded.

mod common;

use std::collections::BTreeSet;

use metrics_util::debugging::{DebugValue, DebuggingRecorder};

#[tokio::test(flavor = "multi_thread")]
async fn the_scripted_run_emits_every_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder.install().expect("first recorder in this process");
    let r = common::run_metrics_script().await;
    r.stop().await.unwrap();
    let snap = snapshotter.snapshot().into_vec();
    let seen: BTreeSet<String> = snap
        .iter()
        .map(|(k, ..)| k.key().name().to_string())
        .collect();
    for name in common::ALL_METRICS {
        assert!(seen.contains(name), "{name} not emitted; saw {seen:?}");
    }
    // The scripted `l` was counted: the info family is at least 1, not just registered at 0.
    let info = snap.iter().find_map(|(k, _, _, v)| {
        let key = k.key();
        (key.name() == "silvermere_commands_total"
            && key
                .labels()
                .any(|l| l.key() == "family" && l.value() == "info"))
        .then_some(v)
    });
    assert!(
        matches!(info, Some(DebugValue::Counter(n)) if *n >= 1),
        "commands_total{{family=info}} = {info:?}"
    );
    // Labels: only bounded values (no names, no raw command text).
    for (key, ..) in &snap {
        for l in key.key().labels() {
            let ok = match (key.key().name(), l.key()) {
                ("silvermere_commands_total", "family") => {
                    silvermere_engine::metrics::FAMILIES.contains(&l.value())
                }
                ("silvermere_db_latency_seconds", "op") => {
                    ["load", "save_character", "save_state", "event_insert"].contains(&l.value())
                }
                ("silvermere_build_info", "version" | "backend") => true,
                _ => false,
            };
            assert!(
                ok,
                "unexpected label {}={} on {}",
                l.key(),
                l.value(),
                key.key().name()
            );
        }
    }
}
