// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end test for the recommended `KeyedAggregator -> WorkerSink ->
//! OtelSink` pipeline. Verifies that:
//! - `#[aggregate(strategy = Sum)]` fields flow through the aggregator and
//!   land on an OTel counter
//! - `#[aggregate(strategy = Histogram<T>)]` fields land on an OTel
//!   histogram via the `Distribution` flag
//! - `#[aggregate(key)]` fields become OTel attributes on the recorded
//!   measurements.

use std::time::Duration;

use metrique::unit_of_work::metrics;
use metrique_aggregation::{
    aggregate, aggregator::KeyedAggregator, histogram::Histogram, sink::WorkerSink, value::Sum,
};
use metrique_otel::{Counter, OtelSink};
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[aggregate(key)]
    operation: String,

    #[aggregate(strategy = Sum)]
    request_count: Counter<u64>,

    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,
}

#[tokio::test(flavor = "multi_thread")]
async fn aggregated_pipeline_emits_counters_and_histograms() {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider.clone())
        .build();

    let aggregator = KeyedAggregator::<RequestMetrics, _>::new(sink.clone());
    // Long flush interval — the explicit `worker.flush()` below drains it.
    let worker = WorkerSink::new(aggregator, Duration::from_secs(3600));

    for (op, lat_ms) in [("GET", 12u64), ("GET", 18), ("GET", 9), ("POST", 47)] {
        RequestMetrics {
            operation: op.to_owned(),
            request_count: Counter::from(1),
            latency: Duration::from_millis(lat_ms),
        }
        .close_and_merge(worker.clone());
    }

    worker.flush().await;
    meter_provider.force_flush().expect("force_flush");

    let exported = exporter
        .get_finished_metrics()
        .expect("get_finished_metrics");

    // Index exported metrics by name and instrument variant so we can
    // assert on shapes, attribute groups, and aggregated values directly.
    let mut counter_attrs: Vec<Vec<(String, String)>> = Vec::new();
    let mut counter_values_by_op: Vec<(String, u64)> = Vec::new();
    let mut histogram_attrs: Vec<Vec<(String, String)>> = Vec::new();

    for rm in &exported {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                match (m.name(), m.data()) {
                    ("RequestCount", AggregatedMetrics::U64(MetricData::Sum(sum))) => {
                        for dp in sum.data_points() {
                            let mut attrs: Vec<(String, String)> = dp
                                .attributes()
                                .map(|kv| {
                                    (kv.key.to_string(), kv.value.as_str().into_owned())
                                })
                                .collect();
                            attrs.sort();
                            let op = attrs
                                .iter()
                                .find(|(k, _)| k == "operation")
                                .map(|(_, v)| v.clone())
                                .unwrap_or_default();
                            counter_values_by_op.push((op, dp.value()));
                            counter_attrs.push(attrs);
                        }
                    }
                    ("Latency", AggregatedMetrics::F64(MetricData::Histogram(hist))) => {
                        for dp in hist.data_points() {
                            let mut attrs: Vec<(String, String)> = dp
                                .attributes()
                                .map(|kv| {
                                    (kv.key.to_string(), kv.value.as_str().into_owned())
                                })
                                .collect();
                            attrs.sort();
                            histogram_attrs.push(attrs);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    counter_values_by_op.sort();
    assert_eq!(
        counter_values_by_op,
        vec![
            ("GET".to_string(), 3),
            ("POST".to_string(), 1),
        ],
        "Sum strategy should produce one counter point per Operation key"
    );

    assert!(
        counter_attrs
            .iter()
            .all(|attrs| attrs.iter().any(|(k, _)| k == "operation")),
        "every counter point should carry the Operation attribute, got {counter_attrs:?}"
    );

    assert!(
        histogram_attrs
            .iter()
            .any(|attrs| attrs
                .iter()
                .any(|(k, v)| k == "operation" && (v == "GET" || v == "POST"))),
        "expected Operation attribute on histogram points, got {histogram_attrs:?}"
    );
    assert_eq!(
        histogram_attrs.len(),
        2,
        "expected one histogram point per Operation key, got {histogram_attrs:?}"
    );
}
