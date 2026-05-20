// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Multiple `#[metrics]` entry types flowing through **one** `OtelSink` /
//! `SdkMeterProvider`.
//!
//! Real services emit more than one shape of metric. A request path,
//! a background job loop, and a one-shot startup record are common
//! examples. Each shape is its own `#[metrics]` struct; together they share
//! a single meter provider so all exports go out on the same OTLP pipe.
//!
//! Each struct's field names become individual OTel instrument names; string
//! fields become per-entry attributes on every metric in that entry. Across
//! entries, instrument names live in a flat namespace, so it's worth giving
//! struct fields names that won't collide (e.g., `RequestLatency` /
//! `JobLatency` rather than two `Latency` fields).
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
//!   - `StartupMillis`, `ConfigSourcesLoaded` (no attributes, startup is
//!     a singleton entry)

use std::time::{Duration, Instant, SystemTime};

use metrique::ServiceMetrics;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::AttachGlobalEntrySink;
use metrique::writer::GlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Gauge, Histogram};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(Counter))]
    request_count: u64,
    #[metrics(flags(Counter))]
    request_errors: u64,
    #[metrics(unit = Millisecond, flags(Histogram))]
    request_latency: Duration,
}

impl RequestMetrics {
    fn init(operation: String) -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            operation,
            request_count: 0,
            request_errors: 0,
            request_latency: Duration::default(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[metrics(rename_all = "PascalCase")]
struct JobMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    job_kind: String,
    #[metrics(flags(Counter))]
    jobs_processed: u64,
    #[metrics(flags(Gauge))]
    job_queue_depth: f64,
}

impl JobMetrics {
    fn init(job_kind: String) -> JobMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            job_kind,
            jobs_processed: 0,
            job_queue_depth: 0.0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

/// Singleton entry emitted once during process startup. No string fields →
/// no per-entry attributes, so these metrics arrive bare.
#[metrics(rename_all = "PascalCase")]
struct StartupMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    #[metrics(unit = Millisecond, flags(Histogram))]
    startup_millis: Duration,
    #[metrics(flags(Counter))]
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
    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_multi_entry")
        .build();
    let _handle = ServiceMetrics::attach((sink.clone(), ()));

    // Startup record, emitted once.
    {
        let mut s = StartupMetrics {
            timestamp: SystemTime::now(),
            startup_millis: Duration::default(),
            config_sources_loaded: 0,
        }
        .append_on_drop(ServiceMetrics::sink());
        s.config_sources_loaded = 3;
        s.startup_millis = started_at.elapsed();
    }

    // Request path, multiple operations.
    for op in ["GET", "POST"] {
        let start = Instant::now();
        let mut m = RequestMetrics::init(op.to_owned());
        m.request_count += 1;
        if op == "POST" {
            m.request_errors += 1;
        }
        m.request_latency = start.elapsed();
    }

    // Background jobs, emitted from a worker loop.
    for (kind, depth) in [("indexer", 12.0), ("compactor", 3.0)] {
        let mut j = JobMetrics::init(kind.to_owned());
        j.jobs_processed += 1;
        j.job_queue_depth = depth;
    }

    sink.flush_async().await;
}
