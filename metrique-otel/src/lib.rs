// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod aggregate;
pub mod flags;
mod metrics;
mod translator;

pub use flags::{Counter, Gauge, Histogram, InstrumentKind, UpDownCounter};

use std::sync::Arc;

use metrique_writer_core::{
    Entry,
    sink::{EntrySink, FlushWait},
};
use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};

use crate::{metrics::InstrumentCache, translator::append_with_pool};

/// Default OTel `InstrumentationScope` name when the caller does not set one
/// via [`OtelSinkBuilder::with_scope`].
const DEFAULT_SCOPE: &str = "metrique-otel";

#[derive(Clone)]
pub struct OtelSink {
    inner: Arc<OtelSinkInner>,
}

struct OtelSinkInner {
    meter_provider: SdkMeterProvider,
    instruments: InstrumentCache,
    /// OTel `InstrumentationScope` name applied to every metric this sink
    /// records. Set once at build time via [`OtelSinkBuilder::with_scope`];
    /// defaults to [`DEFAULT_SCOPE`].
    ///
    /// `&'static str` because the OTel SDK's `MeterProvider::meter()` requires
    /// it. When the caller supplies a `String`, the builder leaks it once at
    /// `build()` time so the borrow lives for the rest of the process — that
    /// is `O(#sinks_built)` bytes, bounded.
    scope: &'static str,
}

impl OtelSink {
    pub fn builder() -> OtelSinkBuilder {
        OtelSinkBuilder::default()
    }

    /// Drive `force_flush` on the meter provider and resolve once it's done.
    /// Errors from `force_flush` are logged at `warn` level.
    ///
    /// Callers outside a tokio runtime should use [`Self::flush`] instead.
    ///
    /// # Panics
    ///
    /// The returned [`FlushWait`] must be awaited on a tokio runtime; it uses
    /// `tokio::task::spawn_blocking` internally and will panic if polled
    /// outside of one.
    pub fn flush_async(&self) -> FlushWait {
        let meter = self.inner.meter_provider.clone();
        FlushWait::from_future(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = meter.force_flush() {
                    tracing::warn!(error = %e, "metrique-otel: meter provider force_flush failed");
                }
            })
            .await;
        })
    }

    /// Synchronous counterpart to [`Self::flush_async`]: drive `force_flush`
    /// on the meter provider directly, blocking the calling thread until the
    /// flush completes. Errors are logged at `warn` level.
    ///
    /// `SdkMeterProvider::force_flush` is itself synchronous; `flush_async`
    /// wraps it in `spawn_blocking` only to remain well-behaved when called
    /// from inside a tokio executor. From a non-tokio context, prefer this
    /// method; from an async context, prefer [`Self::flush_async`] so the
    /// tokio runtime doesn't get blocked.
    pub fn flush(&self) {
        if let Err(e) = self.inner.meter_provider.force_flush() {
            tracing::warn!(error = %e, "metrique-otel: meter provider force_flush failed");
        }
    }

    /// Build a sink whose meter provider is wired to an OTLP/gRPC exporter
    /// using the standard `OTEL_*` environment variables.
    ///
    /// Callers outside a tokio runtime should use
    /// [`Self::with_otlp_http_default`] instead, which uses a blocking
    /// HTTP/protobuf transport that does not need a runtime.
    ///
    /// # Panics
    ///
    /// Must be called from within a tokio runtime. The tonic-backed OTLP
    /// exporter and the [`PeriodicReader`] both spawn export tasks on the
    /// current runtime; calling this outside of `#[tokio::main]` (or an
    /// explicit `Runtime::enter`) will panic.
    ///
    /// [`PeriodicReader`]: opentelemetry_sdk::metrics::PeriodicReader
    pub fn with_otlp_default() -> Result<Self, OtelSinkError> {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        Ok(OtelSinkBuilder::default()
            .with_meter_provider(meter_provider)
            .build())
    }

    /// Synchronous counterpart to [`Self::with_otlp_default`]: build a sink
    /// wired to an OTLP/HTTP+protobuf exporter using a blocking `reqwest`
    /// client. No tokio runtime is required at construction time.
    ///
    /// Uses the standard `OTEL_*` environment variables to discover the
    /// endpoint. The endpoint must be an HTTP URL (gRPC endpoints are not
    /// accepted here; use [`Self::with_otlp_default`] for those).
    ///
    /// The export loop still runs on a background thread spawned by
    /// [`PeriodicReader`], but that thread is a plain OS thread, not a
    /// tokio task.
    ///
    /// [`PeriodicReader`]: opentelemetry_sdk::metrics::PeriodicReader
    pub fn with_otlp_http_default() -> Result<Self, OtelSinkError> {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        Ok(OtelSinkBuilder::default()
            .with_meter_provider(meter_provider)
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
/// `with_resource` only applies when the meter provider is *not* supplied
/// explicitly via `with_meter_provider` — a user-supplied provider already
/// carries its own resource.
#[derive(Default)]
pub struct OtelSinkBuilder {
    meter_provider: Option<SdkMeterProvider>,
    resource: Option<Resource>,
    scope: Option<String>,
}

impl OtelSinkBuilder {
    pub fn with_meter_provider(mut self, provider: SdkMeterProvider) -> Self {
        self.meter_provider = Some(provider);
        self
    }

    pub fn with_resource(mut self, resource: Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Set the OTel `InstrumentationScope` name applied to every metric this
    /// sink records. Defaults to `"metrique-otel"` when not set.
    ///
    /// Use this to disambiguate metrics on the collector side when an
    /// application drives more than one [`OtelSink`] (e.g. one per
    /// service component or per logical concern). A single sink always
    /// records under one scope — looking through wrapper types like
    /// `BoxEntry` or aggregator entries to recover the originating entry
    /// type would require the (not-yet-merged) entry-descriptor system.
    pub fn with_scope(mut self, name: impl Into<String>) -> Self {
        self.scope = Some(name.into());
        self
    }

    /// # Panics
    ///
    /// The default (no provider supplied) builds an empty [`SdkMeterProvider`]
    /// with no readers attached, which does not require a tokio runtime.
    ///
    /// If a meter provider is supplied via [`Self::with_meter_provider`] that
    /// carries a [`PeriodicReader`] (or any other reader that spawns onto the
    /// current runtime), `build` must be called from within a tokio runtime;
    /// the reader will panic otherwise.
    ///
    /// [`PeriodicReader`]: opentelemetry_sdk::metrics::PeriodicReader
    pub fn build(self) -> OtelSink {
        let meter_provider = self.meter_provider.unwrap_or_else(|| {
            let mut b = SdkMeterProvider::builder();
            if let Some(r) = self.resource {
                b = b.with_resource(r);
            }
            b.build()
        });
        let instruments = InstrumentCache::new(meter_provider.clone());
        // Caller-supplied scopes are leaked exactly once at build time so the
        // OTel SDK's `meter(&'static str)` API can borrow them for the rest
        // of the process. The default literal is already `'static`.
        let scope: &'static str = match self.scope {
            Some(s) => Box::leak(s.into_boxed_str()),
            None => DEFAULT_SCOPE,
        };
        OtelSink {
            inner: Arc::new(OtelSinkInner {
                meter_provider,
                instruments,
                scope,
            }),
        }
    }
}

impl<E: Entry + Send + 'static> EntrySink<E> for OtelSink {
    fn append(&self, entry: E) {
        append_with_pool(&self.inner.instruments, self.inner.scope, entry);
    }

    fn flush_async(&self) -> FlushWait {
        OtelSink::flush_async(self)
    }
}

// Note on lifecycle: `OtelSink` deliberately does not implement `Drop` to
// call `shutdown` on the meter provider. Users can pass an externally-owned
// provider via `OtelSinkBuilder::with_meter_provider`, and shutting it down
// when the sink drops would be surprising. If explicit shutdown is needed,
// expose an `OtelSink::shutdown(&self)` later.

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::time::SystemTime;

    use metrique_writer_core::entry::EntryWriter;
    use metrique_writer_core::value::ForceFlag;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader};

    use super::*;

    #[test]
    fn builder_default_constructs_a_sink() {
        let sink = OtelSink::builder().build();
        let _cloned = sink.clone();
    }

    /// Hand-rolled `Entry` so the test does not depend on the `metrique`
    /// derive macro and can target a single counter field directly.
    struct CounterEntry {
        name: &'static str,
        value: ForceFlag<u64, Counter>,
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
            value: ForceFlag::from(7u64),
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
            names.contains(&"Requests"),
            "expected 'Requests' metric, found {names:?}"
        );
    }

    /// Single entry covering all four instrument kinds, so the dispatch
    /// inside `InstrumentCache::record` is exercised end-to-end.
    struct AllKindsEntry {
        counter: ForceFlag<u64, Counter>,
        up_down: ForceFlag<f64, UpDownCounter>,
        histogram: ForceFlag<f64, Histogram>,
        gauge: ForceFlag<f64, Gauge>,
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
            counter: ForceFlag::from(3u64),
            up_down: ForceFlag::from(-2.0_f64),
            histogram: ForceFlag::from(12.5f64),
            gauge: ForceFlag::from(0.42f64),
        });
        meter_provider.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");

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
                let v: ForceFlag<RawCounterPoint, Counter> = ForceFlag::from(RawCounterPoint);
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

    /// String fields on the entry become attributes on every metric in the
    /// same entry — including metrics declared *before* the string field,
    /// since the writer buffers metric records until `finish()`.
    #[test]
    fn string_field_attaches_as_attribute_to_metrics() {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

        struct MixedEntry {
            requests: ForceFlag<u64, Counter>,
            operation: String,
        }

        impl Entry for MixedEntry {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                // Metric is emitted before the string, so this also exercises
                // the buffer-then-flush flow.
                w.value(Cow::Borrowed("Requests"), &self.requests);
                w.value(Cow::Borrowed("Operation"), &self.operation);
            }
        }

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(MixedEntry {
            requests: ForceFlag::from(1u64),
            operation: "GET".to_owned(),
        });

        meter_provider.force_flush().expect("force_flush meter");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");

        let mut found_attrs: Vec<(String, String)> = Vec::new();
        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() != "Requests" {
                        continue;
                    }
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                        for dp in sum.data_points() {
                            for kv in dp.attributes() {
                                found_attrs
                                    .push((kv.key.to_string(), kv.value.as_str().into_owned()));
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            found_attrs,
            vec![("Operation".to_string(), "GET".to_string())],
            "expected Operation=GET to ride along as a metric attribute"
        );
    }

    /// `flush_async` drives `force_flush` on the meter provider and resolves
    /// once it's done — exercised end-to-end (no direct
    /// `provider.force_flush()` calls).
    #[tokio::test(flavor = "multi_thread")]
    async fn flush_async_drains_meter_provider() {
        struct E;
        impl Entry for E {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                let count: ForceFlag<u64, Counter> = ForceFlag::from(3u64);
                w.value(Cow::Borrowed("Requests"), &count);
            }
        }

        let metric_exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(metric_exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider)
            .build();

        sink.append(E);
        sink.flush_async().await;

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

    /// `OtelSinkBuilder::with_scope` controls the OTel `InstrumentationScope`
    /// name reported on every metric this sink records.
    #[test]
    fn with_scope_sets_instrumentation_scope_name() {
        struct E {
            n: ForceFlag<u64, Counter>,
        }
        impl Entry for E {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                w.value(Cow::Borrowed("N"), &self.n);
            }
        }

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .with_scope("my-service")
            .build();

        sink.append(E {
            n: ForceFlag::from(1u64),
        });
        meter_provider.force_flush().expect("force_flush");

        let scopes: Vec<String> = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics")
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .map(|sm| sm.scope().name().to_string())
            .collect();
        assert!(
            scopes.iter().any(|s| s == "my-service"),
            "expected scope 'my-service', got {scopes:?}"
        );
    }

    /// Without `with_scope`, the sink falls back to the `DEFAULT_SCOPE` literal.
    #[test]
    fn default_scope_is_metrique_otel() {
        struct E {
            n: ForceFlag<u64, Counter>,
        }
        impl Entry for E {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                w.value(Cow::Borrowed("N"), &self.n);
            }
        }

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        sink.append(E {
            n: ForceFlag::from(1u64),
        });
        meter_provider.force_flush().expect("force_flush");

        let scopes: Vec<String> = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics")
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .map(|sm| sm.scope().name().to_string())
            .collect();
        assert!(
            scopes.iter().any(|s| s == DEFAULT_SCOPE),
            "expected default scope {DEFAULT_SCOPE:?}, got {scopes:?}"
        );
    }

    /// `flush` is the synchronous counterpart to `flush_async`: callable from
    /// a plain (non-tokio) `#[test]` thread and still drains the meter
    /// provider end-to-end.
    #[test]
    fn flush_drains_meter_provider_sync() {
        struct E;
        impl Entry for E {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                let count: ForceFlag<u64, Counter> = ForceFlag::from(5u64);
                w.value(Cow::Borrowed("SyncFlushed"), &count);
            }
        }

        let metric_exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(metric_exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider)
            .build();

        sink.append(E);
        sink.flush();

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
            names.contains(&"SyncFlushed"),
            "expected SyncFlushed counter via sync flush(), got {names:?}"
        );
    }

    /// `with_otlp_http_default` must be constructible from a plain (non-tokio)
    /// `#[test]` thread. The HTTP exporter uses a blocking reqwest client and
    /// the `PeriodicReader` background worker is a plain OS thread, so no
    /// tokio runtime is required at build time.
    ///
    /// We only assert that construction succeeds; we deliberately do not
    /// `append` or `flush`, since that would attempt to push metrics to the
    /// default `localhost:4318` endpoint and stall on a real network attempt.
    #[test]
    fn with_otlp_http_default_constructs_outside_tokio() {
        let sink = OtelSink::with_otlp_http_default()
            .expect("with_otlp_http_default must construct without tokio");
        // Cloning is cheap (Arc-backed); proves the value is a usable sink.
        let _cloned = sink.clone();
    }
}
