//! Example: Local Development Metrics with KeyedAggregator
//!
//! This demonstrates using `LocalFormat` with `KeyedAggregator` to produce
//! human-readable aggregated metrics for two different API operations.
//!
//! Run with: `cargo run -p metrique-aggregation --example local-webserver`
//!
//! Try different output styles via the FORMAT env var:
//!   FORMAT=pretty   (default) — YAML-esque key-value pairs
//!   FORMAT=json     — pretty-printed JSON
//!   FORMAT=compact-json — single-line JSON
//!   FORMAT=markdown — markdown table

use metrique::ServiceMetrics;
use metrique::local::{LocalFormat, OutputStyle};
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::value::ToString;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::value::Sum;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use std::time::Duration;

/// Metrics for each API request, aggregated by operation and status.
///
/// Fields marked `#[aggregate(key)]` become the grouping key — requests with the
/// same (operation, status) pair are merged together. The remaining fields use
/// aggregation strategies: `Sum` for counters, `Histogram` for latency distributions.
#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[aggregate(key)]
    operation: String,

    #[aggregate(key)]
    #[metrics(format = ToString)]
    status: u16,

    #[aggregate(strategy = Sum)]
    request_count: u64,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    error_count: u64,
}

/// Simulate handling a request with some latency.
async fn handle_request(operation: &str, status: u16, latency_ms: u64) -> RequestMetrics {
    tokio::time::sleep(Duration::from_millis(latency_ms)).await;
    RequestMetrics {
        operation: operation.to_string(),
        status,
        request_count: 1,
        latency: Duration::from_millis(latency_ms),
        error_count: if status >= 400 { 1 } else { 0 },
    }
}

#[tokio::main]
async fn main() {
    // Set up LocalFormat writing to stderr.
    // Control output style via FORMAT env var: pretty (default), json, compact-json, markdown
    let style = match std::env::var("FORMAT").as_deref() {
        Ok("json") => OutputStyle::json(),
        Ok("compact-json") => OutputStyle::compact_json(),
        Ok("markdown") => OutputStyle::markdown_table(),
        _ => OutputStyle::pretty(),
    };
    let _handle = ServiceMetrics::attach_to_stream(
        LocalFormat::new(style).output_to_makewriter(|| std::io::stderr().lock()),
    );

    // KeyedAggregator groups entries by their `#[aggregate(key)]` fields and merges
    // the rest. WorkerSink runs the aggregator on a background thread and flushes
    // periodically (here every 500ms).
    let aggregator = KeyedAggregator::<RequestMetrics>::new(ServiceMetrics::sink());
    let sink = WorkerSink::new(aggregator, Duration::from_millis(500));

    eprintln!("Simulating requests...\n");

    // Simulate a burst of requests to two operations
    let requests = vec![
        ("GetUser", 200u16, 12u64),
        ("GetUser", 200, 15),
        ("GetUser", 200, 8),
        ("GetUser", 200, 45),
        ("GetUser", 500, 120),
        ("ListUsers", 200, 50),
        ("ListUsers", 200, 65),
        ("ListUsers", 200, 55),
        ("ListUsers", 200, 70),
        ("ListUsers", 400, 30),
    ];

    for (op, status, latency_ms) in requests {
        let metrics = handle_request(op, status, latency_ms).await;
        // close_and_merge: closes the metrics struct (resolving timers etc.),
        // then sends it to the aggregator to be merged with other entries
        // sharing the same key.
        metrics.close_and_merge(sink.clone());
    }

    // Flush forces the aggregator to emit all accumulated data now.
    sink.flush().await;

    // Give the background queue a moment to format and write.
    tokio::time::sleep(Duration::from_millis(100)).await;
}
