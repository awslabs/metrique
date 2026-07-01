// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end test for the recommended `KeyedAggregator -> WorkerSink ->
//! OtelSink` pipeline. Verifies that:
//! - `Sum` + `flags(Counter)` fields flow through the aggregator and land on
//!   an OTel counter, with `#[metrics(unit = ...)]` propagated as the wire
//!   unit
//! - `Sum` + `flags(UpDownCounter)` lands on a non-monotonic counter
//! - `KeepLast` + `flags(Gauge)` lands on a gauge with keep-last semantics
//! - `Histogram` fields land on an OTel histogram (via the `Distribution`
//!   flag from the closed histogram, no explicit `flags(Histogram)` needed)
//! - `#[aggregate(key)]` fields become OTel attributes on the recorded
//!   measurements.

use std::time::Duration;

use metrique::unit::{Byte, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::value::{KeepLast, Sum};
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Gauge, UpDownCounter};
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};

/// Build a fresh `OtelSink` wired to an `InMemoryMetricExporter` and return
/// both so each test can drive the pipeline and read the exported metrics.
fn fresh_pipeline() -> (OtelSink, SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
    let sink = OtelSink::builder()
        .with_meter_provider(meter_provider.clone())
        .build();
    (sink, meter_provider, exporter)
}

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[aggregate(key)]
    operation: String,

    #[aggregate(strategy = Sum)]
    #[metrics(flags(Counter))]
    request_count: u64,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
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
            request_count: 1,
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
    // assert on shapes, attribute groups, aggregated values, and units.
    let mut counter_attrs: Vec<Vec<(String, String)>> = Vec::new();
    let mut counter_values_by_op: Vec<(String, u64)> = Vec::new();
    let mut histogram_attrs: Vec<Vec<(String, String)>> = Vec::new();
    let mut latency_unit: Option<String> = None;

    for rm in &exported {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                match (m.name(), m.data()) {
                    ("RequestCount", AggregatedMetrics::U64(MetricData::Sum(sum))) => {
                        for dp in sum.data_points() {
                            let mut attrs: Vec<(String, String)> = dp
                                .attributes()
                                .map(|kv| (kv.key.to_string(), kv.value.as_str().into_owned()))
                                .collect();
                            attrs.sort();
                            let op = attrs
                                .iter()
                                .find(|(k, _)| k == "Operation")
                                .map(|(_, v)| v.clone())
                                .unwrap_or_default();
                            counter_values_by_op.push((op, dp.value()));
                            counter_attrs.push(attrs);
                        }
                    }
                    ("Latency", AggregatedMetrics::F64(MetricData::Histogram(hist))) => {
                        latency_unit = Some(m.unit().to_owned());
                        for dp in hist.data_points() {
                            let mut attrs: Vec<(String, String)> = dp
                                .attributes()
                                .map(|kv| (kv.key.to_string(), kv.value.as_str().into_owned()))
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
        vec![("GET".to_string(), 3), ("POST".to_string(), 1),],
        "Sum + flags(Counter) should emit one counter point per Operation key with the summed value"
    );

    assert!(
        counter_attrs
            .iter()
            .all(|attrs| attrs.iter().any(|(k, _)| k == "Operation")),
        "every counter point should carry the Operation attribute, got {counter_attrs:?}"
    );

    assert!(
        histogram_attrs.iter().any(|attrs| attrs
            .iter()
            .any(|(k, v)| k == "Operation" && (v == "GET" || v == "POST"))),
        "expected Operation attribute on histogram points, got {histogram_attrs:?}"
    );
    assert_eq!(
        histogram_attrs.len(),
        2,
        "expected one histogram point per Operation key, got {histogram_attrs:?}"
    );
    assert_eq!(
        latency_unit.as_deref(),
        Some("ms"),
        "Histogram + #[metrics(unit = Millisecond)] should propagate ms as the wire unit"
    );
}

// --- Scoped per-strategy tests --------------------------------------------
//
// Each test exercises a single (strategy + flag) combination on a minimal
// struct so a regression points directly at one combination.

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct BytesEntry {
    #[aggregate(strategy = Sum)]
    #[metrics(flags(Counter), unit = Byte)]
    bytes_sent: u64,
}

#[tokio::test(flavor = "multi_thread")]
async fn counter_propagates_byte_unit() {
    let (sink, meter_provider, exporter) = fresh_pipeline();
    let aggregator = KeyedAggregator::<BytesEntry, _>::new(sink.clone());
    let worker = WorkerSink::new(aggregator, Duration::from_secs(3600));

    for n in [128u64, 256, 1024] {
        BytesEntry { bytes_sent: n }.close_and_merge(worker.clone());
    }

    worker.flush().await;
    meter_provider.force_flush().expect("force_flush");

    let exported = exporter
        .get_finished_metrics()
        .expect("get_finished_metrics");

    let mut total = 0u64;
    let mut unit: Option<String> = None;
    for rm in &exported {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if let ("BytesSent", AggregatedMetrics::U64(MetricData::Sum(sum))) =
                    (m.name(), m.data())
                {
                    unit = Some(m.unit().to_owned());
                    for dp in sum.data_points() {
                        total += dp.value();
                    }
                }
            }
        }
    }

    assert_eq!(
        total,
        128 + 256 + 1024,
        "Sum + flags(Counter) should sum all values"
    );
    assert_eq!(
        unit.as_deref(),
        Some("By"),
        "#[metrics(unit = Byte)] should propagate `By` as the wire unit"
    );
}

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct InFlightEntry {
    #[aggregate(strategy = Sum)]
    #[metrics(flags(UpDownCounter))]
    delta: f64,
}

#[tokio::test(flavor = "multi_thread")]
async fn up_down_counter_sums_signed_deltas() {
    let (sink, meter_provider, exporter) = fresh_pipeline();
    let aggregator = KeyedAggregator::<InFlightEntry, _>::new(sink.clone());
    let worker = WorkerSink::new(aggregator, Duration::from_secs(3600));

    for d in [3.0_f64, -1.0, 5.0, -2.0] {
        InFlightEntry { delta: d }.close_and_merge(worker.clone());
    }

    worker.flush().await;
    meter_provider.force_flush().expect("force_flush");

    let exported = exporter
        .get_finished_metrics()
        .expect("get_finished_metrics");

    let mut total = 0i64;
    let mut found = false;
    for rm in &exported {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if let ("Delta", AggregatedMetrics::I64(MetricData::Sum(sum))) =
                    (m.name(), m.data())
                {
                    // OTel emits an `is_monotonic` flag on `Sum` — verify we
                    // landed on the non-monotonic instrument (UpDownCounter)
                    // rather than the monotonic one (Counter).
                    assert!(
                        !sum.is_monotonic(),
                        "UpDownCounter sum should report is_monotonic == false"
                    );
                    found = true;
                    for dp in sum.data_points() {
                        total += dp.value();
                    }
                }
            }
        }
    }

    assert!(
        found,
        "expected a `Delta` UpDownCounter in exported metrics"
    );
    assert_eq!(
        total,
        (3 - 1 + 5 - 2) as i64,
        "Sum + flags(UpDownCounter) should aggregate signed deltas"
    );
}

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct PoolEntry {
    #[aggregate(strategy = KeepLast)]
    #[metrics(flags(Gauge))]
    pool_size: f64,
}

#[tokio::test(flavor = "multi_thread")]
async fn gauge_emits_last_observed_value() {
    let (sink, meter_provider, exporter) = fresh_pipeline();
    let aggregator = KeyedAggregator::<PoolEntry, _>::new(sink.clone());
    let worker = WorkerSink::new(aggregator, Duration::from_secs(3600));

    // KeepLast semantics: the final inserted value should be the one exported.
    for s in [10.0_f64, 25.0, 7.0] {
        PoolEntry { pool_size: s }.close_and_merge(worker.clone());
    }

    worker.flush().await;
    meter_provider.force_flush().expect("force_flush");

    let exported = exporter
        .get_finished_metrics()
        .expect("get_finished_metrics");

    let mut observed: Vec<f64> = Vec::new();
    for rm in &exported {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if let ("PoolSize", AggregatedMetrics::F64(MetricData::Gauge(g))) =
                    (m.name(), m.data())
                {
                    for dp in g.data_points() {
                        observed.push(dp.value());
                    }
                }
            }
        }
    }

    assert_eq!(
        observed.len(),
        1,
        "KeepLast + flags(Gauge) should emit exactly one observation per key, got {observed:?}"
    );
    assert_eq!(
        observed[0], 7.0,
        "KeepLast + flags(Gauge) should retain only the last observed value"
    );
}
