// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Raw `OtelSink` path: each request becomes one observation per metric
//! field, with string fields on the entry attached as attributes to every
//! metric in that entry. Suitable for low/medium volume.
//!
//! The recommended high-throughput topology is `KeyedAggregator -> WorkerSink
//! -> OtelSink` (see `otlp_aggregated.rs`).
//!
//! ## Running this example
//!
//! Start a local OTLP gRPC collector first:
//!
//! ```ignore
//! docker run --rm -p 4317:4317 -p 4318:4318 \
//!     otel/opentelemetry-collector-contrib:latest \
//!     --config=/etc/otelcol-contrib/config.yaml
//! ```
//!
//! Then point the example at it and run:
//!
//! ```ignore
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//! OTEL_SERVICE_NAME=metrique-otel-example \
//!     cargo run -p metrique-otel --example otlp_grpc
//! ```
//!
//! The collector should see four metric series — `RequestCount`,
//! `QueueDelta`, `Latency`, `CpuUsage` — each carrying an `Operation`
//! attribute of `GET` or `POST`.

use std::time::{Duration, SystemTime};

use metrique::ServiceMetrics;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::AttachGlobalEntrySink;
use metrique::writer::GlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Gauge, Histogram, UpDownCounter};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    operation: String,

    #[metrics(flags(Counter))]
    request_count: u64,
    #[metrics(flags(UpDownCounter))]
    queue_delta: f64,

    #[metrics(unit = Millisecond, flags(Histogram))]
    latency: Duration,

    #[metrics(flags(Gauge))]
    cpu_usage: f64,
}

impl RequestMetrics {
    fn init(operation: String) -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            operation,
            request_count: 0,
            queue_delta: 0.0,
            latency: Duration::default(),
            cpu_usage: 0.0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Equivalent to `OtelSink::with_otlp_default()`, but inlined so we can
    // also set a custom `InstrumentationScope` via `with_scope`.
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("OTLP env not configured");
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_grpc")
        .build();
    // Keep a clone for the final flush — `attach` consumes the sink that's
    // bound to the global, but `OtelSink: Clone` is cheap (Arc-backed).
    let _handle = ServiceMetrics::attach((sink.clone(), ()));

    handle_request("GET").await;
    handle_request("POST").await;

    // Drain the periodic reader / batch processor before the process exits;
    // otherwise the last in-flight batch is lost.
    sink.flush_async().await;
}

async fn handle_request(operation: &str) {
    let start = std::time::Instant::now();
    let mut metrics = RequestMetrics::init(operation.to_owned());

    metrics.request_count += 1;
    metrics.queue_delta += 1.0;
    metrics.cpu_usage = 0.42;

    metrics.latency = start.elapsed();
}
