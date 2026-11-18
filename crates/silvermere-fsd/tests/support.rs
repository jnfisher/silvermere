//! Helpers shared by the tests: synthetic forms and a host loop.

use silvermere_fsd::{Form, Mode, Outcome, Verify};

/// A form after [`Form::begin`], and what beginning printed.
pub fn begun(template: &[u8], spec: &[u8], mode: Mode, answers: &[u8]) -> (Form, Vec<u8>) {
    let mut f = Form::prepare(template, spec, mode, answers);
    let mut out = Vec::new();
    f.begin(&mut out);
    (f, out)
}

/// One keystroke as the host handles it with the standard verify routine: the key, the
/// pending work, then the report that the output went out.
pub fn key(f: &mut Form, k: u16) -> (Vec<u8>, Outcome) {
    key_with(f, k, &mut |f, n, a, o| f.done_at_end(n, a, o))
}

/// [`key`] with a verify routine of the test's own.
pub fn key_with(f: &mut Form, k: u16, verify: &mut Verify) -> (Vec<u8>, Outcome) {
    let mut out = Vec::new();
    f.key(k, &mut out);
    let outcome = f.process(verify, &mut out);
    f.output_sent(&mut out);
    (out, outcome)
}

/// Types `text` a character at a time; all the output.
pub fn typed(f: &mut Form, text: &str) -> Vec<u8> {
    let mut all = Vec::new();
    for b in text.bytes() {
        all.extend(key(f, u16::from(b)).0);
    }
    all
}

/// Bytes as an escaped string, for readable assertion failures.
pub fn show(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}
