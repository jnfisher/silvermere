//! `ScriptedSession` against the echo server, and every T-005 script parsing.

use std::time::Duration;

use silvermere_testkit::captures::{Capture, captures_dir};
use silvermere_testkit::scripted::{ScriptError, Step};
use silvermere_testkit::{ScriptedSession, TestServer};

const SCRIPT: &str = r#"{
  "scenario": "echo",
  "naws": null,
  "steps": [
    {"op": "connect", "until": "Name: $", "timeout": 5},
    {"send": "alice", "until": "You typed: (\\w+)", "set": {"who": "You typed: (\\w+)"}, "required": true},
    {"send": "hi {who}", "until": "You typed: hi alice", "timeout": 5},
    {"send": "again", "if_unset": "who"},
    {"send": "x", "nowait": true},
    {"op": "sleep", "seconds": 0.2},
    {"op": "disconnect"},
    {"op": "connect", "until": "Name: $", "timeout": 5},
    {"send": "quit", "wait": 0.3}
  ]
}"#;

#[tokio::test]
async fn replays_a_script_with_variables_and_reconnects() {
    let server = TestServer::echo().await.unwrap();
    let mut client = server.client().await.unwrap();
    let script = ScriptedSession::from_json_str(SCRIPT)
        .unwrap()
        .quiet_cap(Duration::from_millis(200));
    let t = script.run(&mut client).await.unwrap();

    assert_eq!(t.steps.len(), 9);
    assert_eq!(t.steps[1].vars["who"], "alice");
    assert_eq!(t.steps[2].sent.as_deref(), Some("hi alice"));
    assert_eq!(t.steps[2].matched, Some(true));
    assert!(
        t.steps[3].not_run,
        "if_unset skips a step once the variable is set"
    );
    assert_eq!(t.steps[4].matched, None, "nowait does not wait");
    assert!(t.plain.contains("You typed: hi alice"));
    assert!(
        t.plain.matches("Welcome to the test server.").count() == 2,
        "two connections"
    );
    assert!(t.plain.contains("Bye."));
    assert_eq!(t.raw, client.raw());
}

#[tokio::test]
async fn a_required_step_that_does_not_match_stops_the_run() {
    let server = TestServer::echo().await.unwrap();
    let mut client = server.client().await.unwrap();
    let script = ScriptedSession::new()
        .step(Step::connect().until("Name: $"))
        .step(
            Step::send("bob")
                .until("never shown")
                .timeout(0.3)
                .required(),
        )
        .send("not reached");
    let err = script.run(&mut client).await.unwrap_err();
    assert!(
        matches!(err, ScriptError::Required { index: 1, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn built_in_code_with_the_step_helpers() {
    let server = TestServer::echo().await.unwrap();
    let mut client = server.client().await.unwrap();
    let t = ScriptedSession::new()
        .step(Step::connect().until("Name: $"))
        .send_until("carol", "You typed: carol")
        .step(
            Step::send_raw("dave\r")
                .until("You typed: dave")
                .timeout(5.0),
        )
        .run(&mut client)
        .await
        .unwrap();
    assert!(t.steps.iter().all(|s| s.matched != Some(false)));
}

#[test]
fn every_capture_script_parses_with_its_steps() {
    let captures = Capture::all(captures_dir()).unwrap();
    if captures.is_empty() {
        eprintln!("skipped: no captures under tests/captures/");
        return;
    }
    for c in &captures {
        let script = ScriptedSession::from_script_json(c.dir.join("script.json")).unwrap();
        let n = c.script["steps"].as_array().unwrap().len();
        assert_eq!(script.steps().len(), n, "{}", c.name);
        assert_eq!(script.scenario, c.name);
    }
}
