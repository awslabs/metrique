//! Recommended high-throughput topology: `KeyedAggregator -> WorkerSink ->
//! OtelSink`. Many source entries roll up into a single OTLP export per
//! (key) tuple.
//!
//! Compared to the raw `otlp_grpc.rs` example, this path:
//! - groups requests by `Operation` (the `#[aggregate(key)]` field) so each
//!   distinct operation gets its own attribute set on the OTLP wire
//! - sums request counts inside the aggregator before recording on the OTel
//!   counter, so high-volume callers don't pay one OTEL `add()` per request
//! - collects per-request latencies into a `Histogram` distribution that is
//!   merged into one OTLP histogram export per flush.
//!
//! Run with the same environment variables as `otlp_grpc.rs`:
//!
//! ```ignore
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//! OTEL_SERVICE_NAME=metrique-otel-example \
//!     cargo run -p metrique-otel --example otlp_aggregated
//! ```

use std::time::Duration;

use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{
    aggregate,
    aggregator::KeyedAggregator,
    histogram::Histogram,
    sink::WorkerSink,
    value::Sum,
};
use metrique_otel::OtelSink;

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    /// Becomes an OTEL attribute on every metric this aggregator group emits.
    #[aggregate(key)]
    operation: String,

    /// Summed across all requests sharing the same `operation`; flushed as a
    /// single `add()` on an OTEL counter.
    #[aggregate(strategy = Sum)]
    request_count: u64,

    /// Each `add_value` is preserved exactly; on flush, the merged
    /// distribution emits multi-observation data the `OtelSink` recognizes
    /// (via the `Distribution` flag) and records on an OTEL histogram.
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let otel_sink = OtelSink::with_otlp_default().expect("OTLP env not configured");

    // KeyedAggregator -> WorkerSink: the worker owns the aggregator,
    // accepts entries from any thread, and flushes every `flush_interval`.
    let aggregator = KeyedAggregator::<RequestMetrics, _>::new(otel_sink.clone());
    let worker = WorkerSink::new(aggregator, Duration::from_secs(1));

    // Simulate two operations with several requests each. The aggregator
    // groups by `operation`, so the final OTLP export has two attribute
    // groups (`Operation=GET`, `Operation=POST`) with summed counts and
    // merged latency distributions.
    for (op, latency_ms) in [
        ("GET", 12),
        ("GET", 18),
        ("GET", 9),
        ("POST", 47),
        ("POST", 53),
    ] {
        // `close_and_merge` closes the entry (resolving timers, etc.) and
        // hands the merged form to the worker for aggregation.
        RequestMetrics {
            operation: op.to_owned(),
            request_count: 1,
            latency: Duration::from_millis(latency_ms),
        }
        .close_and_merge(worker.clone());
    }

    // Force a flush of the aggregator, then drain the OTLP exporter.
    worker.flush().await;
    otel_sink.flush_async().await;
}
