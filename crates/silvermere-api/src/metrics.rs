//! `GET /metrics`: the Prometheus text format. The names are fixed here because T-061's
//! soak reads them; do not rename.
//!
//! | Metric | Type | Emitted by |
//! |---|---|---|
//! | `silvermere_sessions` | gauge | the engine loop (T-018) |
//! | `silvermere_tick_seconds` | histogram | the engine loop (T-018) |
//! | `silvermere_commands_total` | counter | the command dispatcher (T-016) |
//! | `silvermere_persist_queue_depth` | gauge | the persistence worker (T-023) |
//! | `silvermere_db_latency_seconds` | histogram | the persistence worker (T-023) |

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use metrics_exporter_prometheus::{
    Matcher, PrometheusBuilder, PrometheusHandle, PrometheusRecorder,
};

use crate::router::ApiState;

pub const SESSIONS: &str = "silvermere_sessions";
pub const COMMANDS_TOTAL: &str = "silvermere_commands_total";
pub const TICK_SECONDS: &str = "silvermere_tick_seconds";
pub const PERSIST_QUEUE_DEPTH: &str = "silvermere_persist_queue_depth";
pub const DB_LATENCY_SECONDS: &str = "silvermere_db_latency_seconds";

const SECONDS_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0,
];

/// The recorder and the handle that renders it. [`install`] makes it the global recorder;
/// tests keep the recorder local and record through `metrics::with_local_recorder`.
pub fn build() -> (PrometheusRecorder, PrometheusHandle) {
    let recorder = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_seconds".into()), SECONDS_BUCKETS)
        .expect("buckets are non-empty")
        .build_recorder();
    let handle = recorder.handle();
    (recorder, handle)
}

/// Installs the process-wide recorder (the engine thread and the persistence worker emit
/// through the `metrics` facade) and returns the handle for [`ApiState`].
pub fn install() -> Result<PrometheusHandle, metrics::SetRecorderError<PrometheusRecorder>> {
    let (recorder, handle) = build();
    metrics::set_global_recorder(recorder)?;
    describe();
    Ok(handle)
}

/// Registers the help texts.
pub fn describe() {
    metrics::describe_gauge!(SESSIONS, "Sessions connected to the engine");
    metrics::describe_counter!(COMMANDS_TOTAL, "Commands run");
    metrics::describe_histogram!(TICK_SECONDS, metrics::Unit::Seconds, "Fast tick duration");
    metrics::describe_gauge!(
        PERSIST_QUEUE_DEPTH,
        "Writes queued for the persistence worker"
    );
    metrics::describe_histogram!(
        DB_LATENCY_SECONDS,
        metrics::Unit::Seconds,
        "Persistence batch commit latency"
    );
}

pub async fn render(State(state): State<ApiState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
}
