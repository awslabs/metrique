// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(docsrs, feature(doc_cfg))]

mod flags;
mod logs;
mod metrics;
mod translator;

pub use flags::{
    Counter, CounterCtor, Gauge, GaugeCtor, Histogram, HistogramCtor, InstrumentKind,
    UpDownCounter, UpDownCounterCtor,
};

use std::sync::Arc;

use metrique_writer_core::{
    Entry,
    sink::{EntrySink, FlushWait},
};
use opentelemetry_sdk::{Resource, logs::SdkLoggerProvider, metrics::SdkMeterProvider};

use crate::{metrics::InstrumentCache, translator::OtelEntryWriter};

#[derive(Clone)]
#[allow(dead_code)]
pub struct OtelSink {
    inner: Arc<OtelSinkInner>,
}

#[allow(dead_code)]
struct OtelSinkInner {
    logger_provider: SdkLoggerProvider,
    meter_provider: SdkMeterProvider,
    instruments: InstrumentCache,
}

impl OtelSink {
    pub fn builder() -> OtelSinkBuilder {
        OtelSinkBuilder::default()
    }

    /// Drive `force_flush` on both the meter and logger providers and
    /// resolve once they're both done. Errors from `force_flush` are logged
    /// at `warn` level but not surfaced — the [`EntrySink`] trait has no
    /// way to report them.
    ///
    /// Internally this uses `tokio::task::spawn_blocking` so it must be
    /// awaited on a tokio runtime. Callers of [`with_otlp_default`] already
    /// require tokio (the OTLP/gRPC exporters use it transitively), so this
    /// is not an additional constraint in practice.
    ///
    /// [`with_otlp_default`]: Self::with_otlp_default
    pub fn flush_async(&self) -> FlushWait {
        let meter = self.inner.meter_provider.clone();
        let logger = self.inner.logger_provider.clone();
        FlushWait::from_future(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = meter.force_flush() {
                    tracing::warn!(error = %e, "metrique-otel: meter provider force_flush failed");
                }
                if let Err(e) = logger.force_flush() {
                    tracing::warn!(error = %e, "metrique-otel: logger provider force_flush failed");
                }
            })
            .await;
        })
    }

    /// Build a sink whose meter and logger providers are wired to OTLP/gRPC
    /// exporters using the standard `OTEL_*` environment variables.
    ///
    // TODO(aggregation): an `aggregated_otlp_default()` helper that wires
    // `KeyedAggregator -> WorkerSink -> OtelSink` is the recommended path
    // for high-throughput callers. It depends on `ForceFlag<T>: AddAssign
    // where T: AddAssign` landing upstream; see the header of
    // `examples/otlp_grpc.rs` for context.
    pub fn with_otlp_default() -> Result<Self, OtelSinkError> {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let logger_provider = SdkLoggerProvider::builder()
            .with_batch_exporter(log_exporter)
            .build();

        Ok(OtelSinkBuilder::default()
            .with_meter_provider(meter_provider)
            .with_logger_provider(logger_provider)
            .build())
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub enum OtelSinkError {
    Otlp(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for OtelSinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Otlp(e) => write!(f, "failed to build OTLP exporter: {e}"),
        }
    }
}

impl std::error::Error for OtelSinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Otlp(e) => Some(&**e),
        }
    }
}

/// Builder for [`OtelSink`].
///
/// `with_resource` only applies when the corresponding provider is *not*
/// supplied explicitly via `with_logger_provider` / `with_meter_provider` —
/// user-supplied providers already carry their own resource.
#[derive(Default)]
pub struct OtelSinkBuilder {
    logger_provider: Option<SdkLoggerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    resource: Option<Resource>,
}

impl OtelSinkBuilder {
    pub fn with_logger_provider(mut self, provider: SdkLoggerProvider) -> Self {
        self.logger_provider = Some(provider);
        self
    }

    pub fn with_meter_provider(mut self, provider: SdkMeterProvider) -> Self {
        self.meter_provider = Some(provider);
        self
    }

    pub fn with_resource(mut self, resource: Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    pub fn build(self) -> OtelSink {
        let logger_provider = self.logger_provider.unwrap_or_else(|| {
            let mut b = SdkLoggerProvider::builder();
            if let Some(r) = self.resource.clone() {
                b = b.with_resource(r);
            }
            b.build()
        });
        let meter_provider = self.meter_provider.unwrap_or_else(|| {
            let mut b = SdkMeterProvider::builder();
            if let Some(r) = self.resource {
                b = b.with_resource(r);
            }
            b.build()
        });
        let instruments = InstrumentCache::new(meter_provider.clone());
        OtelSink {
            inner: Arc::new(OtelSinkInner {
                logger_provider,
                meter_provider,
                instruments,
            }),
        }
    }
}

impl<E: Entry + Send + 'static> EntrySink<E> for OtelSink {
    fn append(&self, entry: E) {
        let mut writer = OtelEntryWriter::new(&self.inner.instruments, &self.inner.logger_provider);
        entry.write(&mut writer);
        writer.finish();
    }

    fn flush_async(&self) -> FlushWait {
        OtelSink::flush_async(self)
    }
}

// Note on lifecycle: `OtelSink` deliberately does not implement `Drop` to
// call `shutdown` on the providers. Users can pass externally-owned
// providers via `OtelSinkBuilder::with_meter_provider` /
// `with_logger_provider`, and shutting those down when the sink drops would
// be surprising. If explicit shutdown is needed for sinks that own their
// providers, expose an `OtelSink::shutdown(&self)` later.

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::time::SystemTime;

    use metrique_writer_core::entry::EntryWriter;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader};

    use super::*;
    use crate::{Counter, Gauge, Histogram, UpDownCounter};

    #[test]
    fn builder_default_constructs_a_sink() {
        // Stage 1: with no exporters wired up, the sink should still build
        // and clone/drop cleanly.
        let sink = OtelSink::builder().build();
        let _cloned = sink.clone();
    }

    /// Hand-rolled `Entry` so the test does not depend on the `metrique`
    /// derive macro and can target a single counter field directly.
    struct CounterEntry {
        name: &'static str,
        value: Counter<u64>,
    }

    impl Entry for CounterEntry {
        fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
            w.timestamp(SystemTime::now());
            w.value(Cow::Borrowed(self.name), &self.value);
        }
    }

    #[test]
    fn counter_observation_lands_in_exporter() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(CounterEntry {
            name: "Requests",
            value: Counter::from(7u64),
        });
        meter_provider.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");

        let names: Vec<&str> = exported
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .map(|m| m.name())
            .collect();
        assert!(
            names.iter().any(|n| *n == "Requests"),
            "expected 'Requests' metric, found {names:?}"
        );
    }

    /// Single entry covering all four instrument kinds, so the dispatch
    /// inside `InstrumentCache::record` is exercised end-to-end.
    struct AllKindsEntry {
        counter: Counter<u64>,
        up_down: UpDownCounter<f64>,
        histogram: Histogram<f64>,
        gauge: Gauge<f64>,
    }

    impl Entry for AllKindsEntry {
        fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
            w.timestamp(SystemTime::now());
            w.value(Cow::Borrowed("Counter"), &self.counter);
            w.value(Cow::Borrowed("UpDown"), &self.up_down);
            w.value(Cow::Borrowed("Hist"), &self.histogram);
            w.value(Cow::Borrowed("Gauge"), &self.gauge);
        }
    }

    #[test]
    fn all_instrument_kinds_land_in_exporter() {
        use opentelemetry_sdk::metrics::data::AggregatedMetrics;

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(AllKindsEntry {
            counter: Counter::from(3u64),
            up_down: UpDownCounter::from(-2.0_f64),
            histogram: Histogram::from(12.5f64),
            gauge: Gauge::from(0.42f64),
        });
        meter_provider.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");

        // Build (name, AggregatedMetrics-variant) pairs so we can assert each
        // field landed as the correct OTEL instrument type, not just by name.
        let mut by_name: Vec<(&str, &str)> = Vec::new();
        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    let variant = match m.data() {
                        AggregatedMetrics::U64(_) => "u64",
                        AggregatedMetrics::I64(_) => "i64",
                        AggregatedMetrics::F64(_) => "f64",
                    };
                    by_name.push((m.name(), variant));
                }
            }
        }

        for expected in [
            ("Counter", "u64"),
            ("UpDown", "i64"),
            ("Hist", "f64"),
            ("Gauge", "f64"),
        ] {
            assert!(
                by_name.contains(&expected),
                "missing {expected:?} in exported metrics: {by_name:?}"
            );
        }
    }

    /// Verifies that `Unit` translates to a UCUM string and that per-value
    /// dimensions land on the exported data point. We hand-roll a `Value`
    /// that calls `writer.metric()` directly so the test can pin the exact
    /// unit, observations, and dimensions without going through the
    /// `#[metrics]` macro.
    #[test]
    fn unit_and_dimensions_round_trip() {
        use metrique_writer_core::{
            Observation, Unit,
            unit::PositiveScale,
            value::{ForceFlag, MetricFlags, Value, ValueWriter},
        };
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

        use crate::CounterCtor;

        struct RawCounterPoint;
        impl Value for RawCounterPoint {
            fn write(&self, w: impl ValueWriter) {
                w.metric(
                    [Observation::Unsigned(42)],
                    Unit::Byte(PositiveScale::One),
                    [("Operation", "GET"), ("Status", "200")],
                    MetricFlags::empty(),
                );
            }
        }

        struct UnitDimEntry;
        impl Entry for UnitDimEntry {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                // Wrap the raw point with `CounterCtor` so the OTEL flag is
                // injected on top of the bare metric call.
                let v: ForceFlag<RawCounterPoint, CounterCtor> = ForceFlag::from(RawCounterPoint);
                w.value(Cow::Borrowed("ResponseSize"), &v);
            }
        }

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(UnitDimEntry);
        meter_provider.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");

        let mut found_unit: Option<String> = None;
        let mut found_value: Option<u64> = None;
        let mut found_attrs: Vec<(String, String)> = Vec::new();

        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() != "ResponseSize" {
                        continue;
                    }
                    found_unit = Some(m.unit().to_owned());
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                        for dp in sum.data_points() {
                            found_value = Some(dp.value());
                            for kv in dp.attributes() {
                                found_attrs
                                    .push((kv.key.to_string(), kv.value.as_str().into_owned()));
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(found_unit.as_deref(), Some("By"));
        assert_eq!(found_value, Some(42));
        found_attrs.sort();
        assert_eq!(
            found_attrs,
            vec![
                ("Operation".to_string(), "GET".to_string()),
                ("Status".to_string(), "200".to_string()),
            ]
        );
    }

    /// Entry mixing a string field with a counter — string flows to a log
    /// record, counter flows to the meter, both should land in their
    /// respective exporters within a single `append()`.
    #[test]
    fn string_field_emits_log_record() {
        use opentelemetry::logs::AnyValue;
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};

        struct MixedEntry {
            operation: String,
            requests: Counter<u64>,
        }

        impl Entry for MixedEntry {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                w.timestamp(SystemTime::UNIX_EPOCH);
                w.value(Cow::Borrowed("Operation"), &self.operation);
                w.value(Cow::Borrowed("Requests"), &self.requests);
            }
        }

        let log_exporter = InMemoryLogExporter::default();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(log_exporter.clone())
            .build();

        let metric_exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(metric_exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_logger_provider(logger_provider.clone())
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(MixedEntry {
            operation: "GET".to_owned(),
            requests: Counter::from(1u64),
        });

        meter_provider.force_flush().expect("force_flush meter");
        logger_provider.force_flush().expect("force_flush logger");

        let logs = log_exporter.get_emitted_logs().expect("get_emitted_logs");
        assert_eq!(logs.len(), 1, "expected exactly one log record");
        let record = &logs[0].record;

        assert_eq!(record.timestamp(), Some(SystemTime::UNIX_EPOCH));

        let attrs: Vec<(String, String)> = record
            .attributes_iter()
            .filter_map(|(k, v)| match v {
                AnyValue::String(s) => Some((k.to_string(), s.as_str().to_owned())),
                _ => None,
            })
            .collect();
        assert_eq!(
            attrs,
            vec![("Operation".to_string(), "GET".to_string())],
            "log should carry the string field, not the counter"
        );

        // Counter still made it to the meter exporter — string handling
        // didn't accidentally short-circuit the metric path.
        let exported_metrics = metric_exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");
        let metric_names: Vec<&str> = exported_metrics
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .map(|m| m.name())
            .collect();
        assert!(
            metric_names.contains(&"Requests"),
            "expected counter to still be exported alongside log: {metric_names:?}"
        );
    }

    /// `flush_async` drives `force_flush` on both providers and resolves
    /// once they're both done — exercised end-to-end (no direct
    /// `provider.force_flush()` calls).
    #[tokio::test(flavor = "multi_thread")]
    async fn flush_async_drains_providers() {
        use opentelemetry::logs::AnyValue;
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};

        struct MixedEntry;
        impl Entry for MixedEntry {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                let op = "GET".to_owned();
                let count: Counter<u64> = Counter::from(3u64);
                w.value(Cow::Borrowed("Operation"), &op);
                w.value(Cow::Borrowed("Requests"), &count);
            }
        }

        let log_exporter = InMemoryLogExporter::default();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(log_exporter.clone())
            .build();
        let metric_exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(metric_exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_logger_provider(logger_provider)
            .with_meter_provider(meter_provider)
            .build();

        sink.append(MixedEntry);

        // Only `flush_async` — no manual `provider.force_flush()`.
        sink.flush_async().await;

        let logs = log_exporter.get_emitted_logs().expect("get_emitted_logs");
        assert_eq!(logs.len(), 1, "expected one log from flush_async path");
        let has_op = logs[0].record.attributes_iter().any(|(k, v)| {
            k.as_str() == "Operation" && matches!(v, AnyValue::String(s) if s.as_str() == "GET")
        });
        assert!(has_op, "expected Operation=GET attribute on log record");

        let exported = metric_exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");
        let names: Vec<&str> = exported
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .map(|m| m.name())
            .collect();
        assert!(
            names.contains(&"Requests"),
            "expected Requests counter via flush_async, got {names:?}"
        );
    }

    /// Entry with only metric fields should *not* emit an empty log record.
    #[test]
    fn metrics_only_entry_emits_no_log() {
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};

        let log_exporter = InMemoryLogExporter::default();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(log_exporter.clone())
            .build();

        let sink = OtelSink::builder()
            .with_logger_provider(logger_provider.clone())
            .build();

        sink.append(CounterEntry {
            name: "Requests",
            value: Counter::from(1u64),
        });
        logger_provider.force_flush().expect("force_flush logger");

        let logs = log_exporter.get_emitted_logs().expect("get_emitted_logs");
        assert!(
            logs.is_empty(),
            "expected no log records for a metrics-only entry, got {logs:?}"
        );
    }
}
