// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Programmatic OTel `Resource` configuration with semantic conventions.
//!
//! `otlp_grpc.rs` and `otlp_aggregated.rs` rely on `OTEL_SERVICE_NAME` to
//! identify the producer. This example shows the programmatic path: build a
//! `Resource` with semantic-convention attributes (`service.name`,
//! `service.version`, `service.instance.id`, `deployment.environment`,
//! `host.name`) and hand it to `SdkMeterProvider::builder().with_resource()`.
//!
//! Resource attributes attach to every metric the provider produces, distinct
//! from per-entry attributes (which come from string fields on a `#[metrics]`
//! struct) and per-metric dimensions (which come from `ValueWriter::metric`).
//!
//! Note: `OtelSinkBuilder::with_resource` only applies when no meter provider
//! is supplied. For runnable apps (where you need a reader + exporter), set
//! the resource on `SdkMeterProvider::builder()` directly as shown here.
//!
//! ## Running this example
//!
//! ```sh
//! docker run --rm -p 4317:4317 -p 4318:4318 \
//!     otel/opentelemetry-collector-contrib:latest
//!
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//!     cargo run -p metrique-otel --example otlp_resource_attributes
//! ```
//!
//! In the collector logs, every metric will carry the resource attributes
//! shown under `Resource attributes:`.

use std::time::{SystemTime, UNIX_EPOCH};

use metrique::ServiceMetrics;
use metrique::unit_of_work::metrics;
use metrique::writer::AttachGlobalEntrySink;
use metrique::writer::GlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::flags::Counter;
use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(Counter))]
    request_count: u64,
}

impl RequestMetrics {
    fn init(operation: String) -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            operation,
            request_count: 0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Build a Resource with OTel semantic-convention attributes.
    // `with_service_name` is a typed shortcut for `service.name`; everything
    // else goes through `with_attribute`. Keys are stable across OTel SDKs,
    // see https://opentelemetry.io/docs/specs/semconv/resource/.
    let resource = Resource::builder()
        .with_service_name("metrique-otel-example")
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .with_attribute(KeyValue::new("service.instance.id", instance_id()))
        .with_attribute(KeyValue::new("deployment.environment", "local-dev"))
        .with_attribute(KeyValue::new(
            "host.name",
            std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_owned()),
        ))
        .build();

    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("OTLP env not configured");
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .with_resource(resource)
        .build();

    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_resource_attributes")
        .build();
    let _handle = ServiceMetrics::attach((sink.clone(), ()));

    for op in ["GET", "POST", "DELETE"] {
        let mut m = RequestMetrics::init(op.to_owned());
        m.request_count += 1;
    }

    sink.flush_async().await;
}

/// Stand-in for a real instance ID source (e.g., container ID, pod UID, or
/// `uuid::Uuid::new_v4()`). Kept dependency-free.
fn instance_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("instance-{nanos}")
}
