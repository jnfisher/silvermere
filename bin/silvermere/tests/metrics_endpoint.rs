//! `GET /metrics` on a running server returns the metrics of `docs/metrics.md`.

mod common;

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread")]
async fn metrics_endpoint_lists_every_metric_after_the_scripted_run() {
    let r = common::run_metrics_script().await;
    let admin = r.admin_addr.unwrap();
    let mut s = tokio::net::TcpStream::connect(admin).await.unwrap();
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut raw))
        .await
        .unwrap()
        .unwrap();
    for name in common::ALL_METRICS {
        assert!(raw.contains(name), "{name} missing from /metrics:\n{raw}");
    }
    // The scripted `l` was dispatched, so the info family is counted, not just exported at 0.
    let info = raw
        .lines()
        .find_map(|l| l.strip_prefix("silvermere_commands_total{family=\"info\"} "))
        .unwrap_or_else(|| panic!("no info series in /metrics:\n{raw}"));
    assert!(
        info.trim().parse::<f64>().unwrap() >= 1.0,
        "info commands = {info}:\n{raw}"
    );
    r.stop().await.unwrap();
}
