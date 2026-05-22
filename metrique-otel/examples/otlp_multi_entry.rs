// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Multiple `#[aggregate]` entry types flowing through **one** `OtelSink` /
//! `SdkMeterProvider`.
//!
//! Real services emit more than one shape of metric. A request path, a
//! background job loop, and a one-shot startup record are common examples.
//! Each shape is its own `#[aggregate]` struct, given its own
//! [`KeyedAggregator`] (and worker), all flushing into the same
//! [`crate::OtelSink`] so exports go out on the same OTLP pipe.
//!
//! Each struct's field names become OTel instrument names; `#[aggregate(key)]`
//! fields become per-entry attributes on every metric in that entry. Across
//! entries, instrument names live in a flat namespace, so it's worth giving
//! struct fields names that won't collide (e.g., `RequestLatency` /
//! `JobLatency` rather than two `Latency` fields).
//!
//! [`KeyedAggregator`]: metrique_aggregation::aggregator::KeyedAggregator
//!
//! ## Running this example
//!
//! ```sh
//! docker run --rm -p 4317:4317 -p 4318:4318 \
//!     otel/opentelemetry-collector-contrib:latest
//!
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//! OTEL_SERVICE_NAME=metrique-otel-example \
//!     cargo run -p metrique-otel --example otlp_multi_entry
//! ```
//!
//! The collector should see seven metric series:
//!   - `RequestCount`, `RequestLatency`, `RequestErrors` carrying `Operation`
//!   - `JobsProcessed`, `JobQueueDepth` carrying `JobKind`
//!   - `StartupMillis`, `ConfigSourcesLoaded` (no key fields, startup is a
//!     singleton entry)

use std::time::{Duration, Instant};

use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::OtelSink;
use metrique_otel::aggregate::{OtelCounter, OtelGauge, OtelHistogram};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[aggregate(key)]
    operation: String,
    #[aggregate(strategy = OtelCounter)]
    request_count: u64,
    #[aggregate(strategy = OtelCounter)]
    request_errors: u64,
    #[aggregate(strategy = OtelHistogram<Millisecond>)]
    request_latency: Duration,
}

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct JobMetrics {
    #[aggregate(key)]
    job_kind: String,
    #[aggregate(strategy = OtelCounter)]
    jobs_processed: u64,
    #[aggregate(strategy = OtelGauge)]
    job_queue_depth: f64,
}

/// Singleton entry emitted once during process startup. No key fields means
/// no per-entry attributes, so these metrics arrive bare.
#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct StartupMetrics {
    #[aggregate(strategy = OtelHistogram<Millisecond>)]
    startup_millis: Duration,
    #[aggregate(strategy = OtelCounter)]
    config_sources_loaded: u64,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let started_at = Instant::now();

    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("OTLP env not configured");
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let otel_sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_multi_entry")
        .build();

    // One aggregator per entry shape, all flushing into the same OtelSink.
    let request_worker = WorkerSink::new(
        KeyedAggregator::<RequestMetrics, _>::new(otel_sink.clone()),
        Duration::from_secs(1),
    );
    let job_worker = WorkerSink::new(
        KeyedAggregator::<JobMetrics, _>::new(otel_sink.clone()),
        Duration::from_secs(1),
    );
    let startup_worker = WorkerSink::new(
        KeyedAggregator::<StartupMetrics, _>::new(otel_sink.clone()),
        Duration::from_secs(1),
    );

    // Startup record, emitted once.
    StartupMetrics {
        startup_millis: started_at.elapsed(),
        config_sources_loaded: 3,
    }
    .close_and_merge(startup_worker.clone());

    // Request path, multiple operations.
    for op in ["GET", "POST"] {
        let start = Instant::now();
        let request_errors = if op == "POST" { 1 } else { 0 };
        RequestMetrics {
            operation: op.to_owned(),
            request_count: 1,
            request_errors,
            request_latency: start.elapsed(),
        }
        .close_and_merge(request_worker.clone());
    }

    // Background jobs, emitted from a worker loop.
    for (kind, depth) in [("indexer", 12.0), ("compactor", 3.0)] {
        JobMetrics {
            job_kind: kind.to_owned(),
            jobs_processed: 1,
            job_queue_depth: depth,
        }
        .close_and_merge(job_worker.clone());
    }

    // Drain each aggregator (sequentially, so the example output is
    // deterministic), then drain the OtelSink's exporter.
    request_worker.flush().await;
    job_worker.flush().await;
    startup_worker.flush().await;
    otel_sink.flush_async().await;
}
