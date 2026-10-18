//! `normalise DIR... [--masks FILE]`: prints the normalised event list of
//! each T-005 capture directory as JSON lines (`{capture, name, cat, fields}`), the Rust side
//! of `tools/differential/tests/cross_check.py`.

use std::process::ExitCode;

use serde_json::json;
use silvermere_testkit::{Capture, MaskSpec, PatternSet, normalise};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("normalise: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut dirs = Vec::new();
    let mut masks_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--masks" => masks_path = Some(args.next().ok_or("--masks needs a file")?),
            _ => dirs.push(a),
        }
    }
    if dirs.is_empty() {
        return Err("usage: normalise DIR... [--masks FILE]".to_owned());
    }
    let masks = match masks_path {
        Some(p) => MaskSpec::load(p),
        None => MaskSpec::load_default(),
    }
    .map_err(|e| e.to_string())?;
    let set = PatternSet::load_default().map_err(|e| e.to_string())?;
    for dir in dirs {
        let capture = Capture::load(&dir).map_err(|e| e.to_string())?;
        let events = normalise(&set.classify_transcript(&capture.plain), &masks);
        for e in events {
            println!(
                "{}",
                json!({"capture": capture.name, "name": e.name, "cat": e.cat, "fields": e.fields})
            );
        }
    }
    Ok(())
}
