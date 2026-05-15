// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Controlling what hits the wire: OTel **Views** (per-instrument
//! customization) and **Temporality** (cumulative vs. delta on counters /
//! histograms).
//!
//! By default the OTLP/gRPC exporter emits cumulative aggregations and the
//! SDK picks its own histogram bucket boundaries. This example overrides
//! both:
//!   - A view replaces the default histogram buckets on `RequestLatency`
//!     with a bucket layout tuned for sub-second p99 work.
//!   - A view caps the data-point cardinality of `RequestCount` so a
//!     misbehaving caller emitting thousands of distinct `Operation` values
//!     can't blow up exporter memory.
//!   - The OTLP exporter is configured with `Temporality::Delta` so counters
//!     arrive as per-interval increments rather than monotonically rising
//!     totals — useful for sinks that prefer delta semantics (Prometheus
//!     remote-write gateways, Datadog, etc.).
//!
//! ## Running this example
//!
//! ```ignore
//! docker run --rm -p 4317:4317 -p 4318:4318 \
//!     otel/opentelemetry-collector-contrib:latest
//!
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//! OTEL_SERVICE_NAME=metrique-otel-example \
//!     cargo run -p metrique-otel --example otlp_views_and_temporality
//! ```
//!
//! In the collector logs, observe:
//!   - `RequestLatency`'s explicit bucket boundaries match those declared
//!     below (override worked);
//!   - `RequestCount` has at most `MAX_REQUEST_COUNT_DATAPOINTS` data
//!     points per export, even with many distinct operations;
//!   - aggregation temporality is reported as `Delta` rather than the
//!     default `Cumulative`.

use std::time::{Duration, Instant, SystemTime};

use metrique::ServiceMetrics;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::AttachGlobalEntrySink;
use metrique::writer::GlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Histogram};
use opentelemetry_sdk::metrics::{
    Aggregation, InstrumentKind, PeriodicReader, SdkMeterProvider, Stream, Temporality,
};

/// Cap on data points per export for `RequestCount`. Beyond this, the SDK
/// folds the overflow into a synthetic `otel.metric.overflow` data point —
/// you'll see truncation rather than unbounded memory growth.
///
/// Set deliberately small (3) so that running this example produces a
/// visible overflow point in the collector. In production you'd set this
/// to the largest expected legitimate cardinality.
const MAX_REQUEST_COUNT_DATAPOINTS: usize = 3;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    /// High-cardinality attribute that drives up data-point count if
    /// unbounded — the cardinality-limit view below caps the total.
    request_id: String,
    #[metrics(flags(Counter))]
    request_count: u64,
    #[metrics(unit = Millisecond, flags(Histogram))]
    request_latency: Duration,
}

impl RequestMetrics {
    fn init(operation: String, request_id: String) -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            operation,
            request_id,
            request_count: 0,
            request_latency: Duration::default(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Delta temporality is selected on the exporter; the SDK passes the
    // selection down to its readers. `Temporality::Delta` means counter
    // and histogram exports carry per-interval increments rather than
    // running totals — the receiver is responsible for any rollup.
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_temporality(Temporality::Delta)
        .build()
        .expect("OTLP env not configured");

    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        // View 1: tighter histogram buckets for `RequestLatency`. Match by
        // instrument name; replace the default boundaries with ones tuned
        // for sub-second latencies (in milliseconds, matching the unit
        // declared on the field).
        .with_view(|inst: &opentelemetry_sdk::metrics::Instrument| {
            if inst.name() == "RequestLatency"
                && matches!(inst.kind(), InstrumentKind::Histogram)
            {
                Some(
                    Stream::builder()
                        .with_aggregation(Aggregation::ExplicitBucketHistogram {
                            boundaries: vec![
                                1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0,
                            ],
                            record_min_max: true,
                        })
                        .build()
                        .ok()?,
                )
            } else {
                None
            }
        })
        // View 2: cap data-point cardinality on `RequestCount`. Without
        // this, a caller emitting many distinct `RequestId` values would
        // produce one data point per ID. With the cap, overflow rolls into
        // a synthetic `otel.metric.overflow` point — bounded memory.
        .with_view(|inst: &opentelemetry_sdk::metrics::Instrument| {
            if inst.name() == "RequestCount" {
                Some(
                    Stream::builder()
                        .with_cardinality_limit(MAX_REQUEST_COUNT_DATAPOINTS)
                        .build()
                        .ok()?,
                )
            } else {
                None
            }
        })
        .build();

    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_views_and_temporality")
        .build();
    let _handle = ServiceMetrics::attach((sink.clone(), ()));

    // Several requests with distinct request IDs — the view should fold
    // these into a single `RequestCount` data point per `Operation`.
    for (op, rid) in [
        ("GET", "req-1"),
        ("GET", "req-2"),
        ("GET", "req-3"),
        ("POST", "req-4"),
        ("POST", "req-5"),
    ] {
        let start = Instant::now();
        let mut m = RequestMetrics::init(op.to_owned(), rid.to_owned());
        m.request_count += 1;
        m.request_latency = start.elapsed();
    }

    sink.flush_async().await;
}
