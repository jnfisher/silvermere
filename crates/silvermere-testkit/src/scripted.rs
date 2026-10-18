//! `ScriptedSession`: replays a T-005 `script.json` (or one built in code) over a
//! [`TelnetClient`].
//!
//! Step semantics are `capture.Runner.step` (`tools/differential/vendor/play/capture.py`): `send` appends a
//! CR unless `raw`, `{name}` is replaced by a variable set earlier, `until` is a regex over
//! the plain text since the send, `until_cursor` waits for a cursor position, `set` stores
//! group 1 of the last match, `if_unset`/`skip_if` skip a step, `nowait` sends and goes on.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::captures::{plain, unescape};
use crate::client::{ClientError, TelnetClient, Wait};
use crate::patterns::Rx;

fn default_op() -> String {
    "send".to_owned()
}
fn default_wait() -> f64 {
    1.0
}
fn default_timeout() -> f64 {
    10.0
}

/// One step of a script. Unknown fields (`note`, `result`, ...) are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    #[serde(default = "default_op")]
    pub op: String,
    #[serde(default)]
    pub send: Option<String>,
    #[serde(default)]
    pub raw: bool,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub until_cursor: Option<String>,
    /// Quiet period in seconds when there is no `until`.
    #[serde(default = "default_wait")]
    pub wait: f64,
    #[serde(default = "default_timeout")]
    pub timeout: f64,
    #[serde(default)]
    pub nowait: bool,
    #[serde(default)]
    pub seconds: Option<f64>,
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    #[serde(default)]
    pub if_unset: Option<String>,
    #[serde(default)]
    pub skip_if: Option<String>,
    #[serde(default)]
    pub required: bool,
}

impl Step {
    fn blank(op: &str) -> Self {
        Self {
            op: op.to_owned(),
            send: None,
            raw: false,
            until: None,
            until_cursor: None,
            wait: default_wait(),
            timeout: default_timeout(),
            nowait: false,
            seconds: None,
            set: BTreeMap::new(),
            if_unset: None,
            skip_if: None,
            required: false,
        }
    }

    /// Send `text` and a CR, then wait for the output to go quiet.
    pub fn send(text: &str) -> Self {
        Self {
            send: Some(text.to_owned()),
            ..Self::blank("send")
        }
    }

    /// Send `text` exactly, no CR.
    pub fn send_raw(text: &str) -> Self {
        Self {
            raw: true,
            ..Self::send(text)
        }
    }

    /// Open (or reopen) the connection and wait for the greeting.
    pub fn connect() -> Self {
        Self::blank("connect")
    }

    pub fn disconnect() -> Self {
        Self::blank("disconnect")
    }

    /// End the wait when this regex matches the plain text since the step started.
    pub fn until(mut self, regex: &str) -> Self {
        self.until = Some(regex.to_owned());
        self
    }

    pub fn timeout(mut self, seconds: f64) -> Self {
        self.timeout = seconds;
        self
    }

    pub fn quiet(mut self, seconds: f64) -> Self {
        self.wait = seconds;
        self
    }

    /// The run stops with an error if this step's `until` does not match.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Store group 1 of the last match of `regex` in this step's output as `{name}`.
    pub fn set(mut self, name: &str, regex: &str) -> Self {
        self.set.insert(name.to_owned(), regex.to_owned());
        self
    }
}

#[derive(Debug, Deserialize)]
struct ScriptFile {
    #[serde(default)]
    scenario: String,
    #[serde(default)]
    naws: Option<(u16, u16)>,
    steps: Vec<Step>,
}

/// What one step did.
#[derive(Debug, Clone)]
pub struct StepResult {
    pub index: usize,
    pub op: String,
    /// The text sent, `{name}` substituted.
    pub sent: Option<String>,
    /// Stream offset when the step started.
    pub offset: usize,
    pub waited: Duration,
    /// `None` for steps with nothing to match.
    pub matched: Option<bool>,
    /// Plain text that arrived between the previous step and this send.
    pub skipped: String,
    pub vars: BTreeMap<String, String>,
    /// Skipped by `if_unset` or `skip_if`.
    pub not_run: bool,
}

/// The result of a run.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub steps: Vec<StepResult>,
    /// The client's whole raw stream at the end.
    pub raw: Vec<u8>,
    /// Its `bridge.plain` text.
    pub plain: String,
}

/// Errors from a run.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("reading script: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing script: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Client(#[from] ClientError),
    #[error("step {index} ({op}) did not match {pattern:?}")]
    Required {
        index: usize,
        op: String,
        pattern: String,
    },
}

/// A sequence of steps.
#[derive(Debug, Clone, Default)]
pub struct ScriptedSession {
    pub scenario: String,
    /// Window size to answer `DO NAWS` with (`naws` in the script); apply it with
    /// [`TelnetClient::with_naws`] before connecting.
    pub naws: Option<(u16, u16)>,
    steps: Vec<Step>,
    quiet_cap: Option<Duration>,
}

impl ScriptedSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_script_json(path: impl AsRef<Path>) -> Result<Self, ScriptError> {
        Self::from_json_str(&std::fs::read_to_string(path)?)
    }

    pub fn from_json_str(json: &str) -> Result<Self, ScriptError> {
        let f: ScriptFile = serde_json::from_str(json)?;
        Ok(Self {
            scenario: f.scenario,
            naws: f.naws,
            steps: f.steps,
            quiet_cap: None,
        })
    }

    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Shorthand for `.step(Step::send(text))`.
    pub fn send(self, text: &str) -> Self {
        self.step(Step::send(text))
    }

    /// Shorthand for `.step(Step::send(text).until(regex))`.
    pub fn send_until(self, text: &str, regex: &str) -> Self {
        self.step(Step::send(text).until(regex))
    }

    /// Never wait longer than `cap` for output to go quiet, so a test against a fast
    /// in-process server does not sit through the original's one-second quiet periods.
    pub fn quiet_cap(mut self, cap: Duration) -> Self {
        self.quiet_cap = Some(cap);
        self
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn quiet(&self, seconds: f64) -> Duration {
        let d = Duration::from_secs_f64(seconds.max(0.0));
        self.quiet_cap.map_or(d, |cap| d.min(cap))
    }

    /// Runs every step over `client`.
    pub async fn run(&self, client: &mut TelnetClient) -> Result<Transcript, ScriptError> {
        let mut vars: BTreeMap<String, String> = BTreeMap::new();
        let mut results = Vec::with_capacity(self.steps.len());
        let mut connected_once = false;
        let fmt = |vars: &BTreeMap<String, String>, s: &str| {
            vars.iter()
                .fold(s.to_owned(), |s, (k, v)| s.replace(&format!("{{{k}}}"), v))
        };
        for (index, st) in self.steps.iter().enumerate() {
            let mut res = StepResult {
                index,
                op: st.op.clone(),
                sent: None,
                offset: client.position(),
                waited: Duration::ZERO,
                matched: None,
                skipped: String::new(),
                vars: BTreeMap::new(),
                not_run: false,
            };
            let skip = st.if_unset.as_ref().is_some_and(|k| vars.contains_key(k))
                || match &st.skip_if {
                    Some(src) => Rx::new(src)
                        .map_err(|_| ClientError::Regex(src.clone()))?
                        .is_match(&client.plain()),
                    None => false,
                };
            if skip {
                res.not_run = true;
                results.push(res);
                continue;
            }
            let until = st.until.as_deref().map(|u| fmt(&vars, u));
            let wait = Wait {
                until: until.clone(),
                until_cursor: st.until_cursor.clone(),
                quiet: self.quiet(st.wait),
                timeout: Duration::from_secs_f64(st.timeout),
            };
            match st.op.as_str() {
                "connect" => {
                    if connected_once || !client.is_open() {
                        client.reconnect().await?;
                    }
                    connected_once = true;
                    let got = client.wait_from(res.offset, &wait).await?;
                    res.waited = got.waited;
                    res.matched = Some(got.matched);
                }
                "disconnect" => client.disconnect().await,
                "sleep" => {
                    let secs = Duration::from_secs_f64(st.seconds.unwrap_or(0.0));
                    if client.is_open() {
                        let w = Wait {
                            quiet: self.quiet(st.seconds.unwrap_or(0.0)),
                            timeout: secs,
                            ..wait
                        };
                        let got = client.wait_from(res.offset, &w).await?;
                        res.waited = got.waited;
                        if until.is_some() {
                            res.matched = Some(got.matched);
                        }
                    } else {
                        tokio::time::sleep(secs).await;
                        res.waited = secs;
                    }
                    self.set_vars(st, client, res.offset, &mut vars, &mut res);
                }
                _ => {
                    let text = fmt(&vars, &unescape(st.send.as_deref().unwrap_or_default()));
                    res.skipped = if st.raw {
                        client.send_text(&text).await?
                    } else {
                        client.send_line(&text).await?
                    };
                    res.offset = client.position();
                    res.sent = Some(text);
                    if !st.nowait {
                        let got = client.wait_from(res.offset, &wait).await?;
                        res.waited = got.waited;
                        res.matched = Some(got.matched);
                        self.set_vars(st, client, res.offset, &mut vars, &mut res);
                    }
                }
            }
            if st.required && res.matched == Some(false) {
                return Err(ScriptError::Required {
                    index,
                    op: st.op.clone(),
                    pattern: until.unwrap_or_default(),
                });
            }
            results.push(res);
        }
        Ok(Transcript {
            steps: results,
            raw: client.raw().to_vec(),
            plain: client.plain(),
        })
    }

    fn set_vars(
        &self,
        st: &Step,
        client: &TelnetClient,
        offset: usize,
        vars: &mut BTreeMap<String, String>,
        res: &mut StepResult,
    ) {
        let out = plain(&client.plain_from(offset));
        for (name, src) in &st.set {
            if let Ok(rx) = Rx::new(src)
                && let Some(v) = rx.last_group(&out, 1)
            {
                vars.insert(name.clone(), v.clone());
                res.vars.insert(name.clone(), v);
            }
        }
    }
}
