// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(docsrs, feature(doc_cfg))]

//! OpenTelemetry (OTLP) backend for [`metrique`].
//!
//! The user-facing API is the aggregation pipeline: declare a metrics struct
//! with [`#[aggregate]`](metrique_aggregation::aggregate), pick a standard
//! aggregation strategy from [`metrique_aggregation`] (`Sum`, `KeepLast`,
//! `Histogram`), and tag the OTel instrument kind on the field with
//! [`#[metrics(flags(...))]`][flags]. Pipe the result through a
//! [`KeyedAggregator`] backed by an [`OtelSink`].
//!
//! [`metrique`]: https://docs.rs/metrique
//! [`KeyedAggregator`]: metrique_aggregation::aggregator::KeyedAggregator
//!
//! ```
//! use std::time::Duration;
//! use metrique::unit::Millisecond;
//! use metrique::unit_of_work::metrics;
//! use metrique_aggregation::value::Sum;
//! use metrique_aggregation::histogram::Histogram;
//! use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
//! use metrique_otel::OtelSink;
//! use metrique_otel::flags::Counter;
//!
//! #[aggregate]
//! #[metrics(rename_all = "PascalCase")]
//! struct RequestMetrics {
//!     #[aggregate(key)] operation: String,
//!
//!     #[aggregate(strategy = Sum)]
//!     #[metrics(flags(Counter))]
//!     request_count: u64,
//!
//!     #[aggregate(strategy = Histogram<Duration>)]
//!     #[metrics(unit = Millisecond)]
//!     latency: Duration,
//! }
//!
//! # async fn run() {
//! let otel_sink = OtelSink::with_otlp_default().expect("OTLP env not configured");
//! let aggregator = KeyedAggregator::<RequestMetrics, _>::new(otel_sink.clone());
//! let worker = WorkerSink::new(aggregator, Duration::from_secs(1));
//!
//! RequestMetrics {
//!     operation: "GET".into(),
//!     request_count: 1,
//!     latency: Duration::from_millis(12),
//! }
//! .close_and_merge(worker.clone());
//!
//! worker.flush().await;
//! otel_sink.flush_async().await;
//! # }
//! ```
//!
//! Two pieces of plumbing tell `OtelSink` which OTel instrument to record on:
//!
//! - For counters, up-down counters, and gauges, tag the field with
//!   [`flags(Counter)`][flags::Counter], [`flags(UpDownCounter)`][flags::UpDownCounter],
//!   or [`flags(Gauge)`][flags::Gauge]. Non-OTel sinks (e.g. EMF) ignore the
//!   tag and treat the field as whatever its strategy says it is.
//! - For histograms, no extra tag is needed: a `Histogram` strategy closes to
//!   `HistogramClosed`, whose `Value::write` already advertises a
//!   `Distribution`, and the OTel translator maps that to a histogram
//!   instrument.
//!
//! Units are spelled with [`#[metrics(unit = ...)]`][metrique-unit] alongside
//! the kind tag, exactly like in non-aggregated entries.
//!
//! Each strategy sums or accumulates on the worker thread; the OTel SDK only
//! sees one merged observation per `#[aggregate(key)]` tuple per flush, which
//! is roughly 30x cheaper per ingest than recording on every entry.
//!
//! See `examples/otlp_aggregated.rs` for the canonical wiring and
//! `examples/otlp_*` for variations covering custom resources, views and
//! temporality, multiple entry types, and dual emission with EMF.
//!
//! # Observation semantics
//!
//! ## Repeated observations on histograms
//!
//! When a field arrives as an `Observation::Repeated { total, occurrences }`
//! (i.e. a strategy has pre-summed the distribution), `OtelSink` replays the
//! mean `total / occurrences` against the histogram instrument
//! `min(occurrences, 1024)` times. Replaying is what lets the OTel
//! histogram's `count` and `sum` line up with the original
//! occurrence count instead of reporting `1` per pre-summed batch. When
//! `occurrences` exceeds the cap, the excess is dropped (logged at `warn`,
//! rate-limited) and downstream `count` will undercount by that excess.
//!
//! Bucketing is still lossy because every replayed sample is the mean rather
//! than an individual value: histogram bucket counts and percentiles will be
//! pinched toward the mean. Callers that need faithful distributions should
//! keep raw `Floating` observations and avoid pre-summing on the way in
//! (e.g. avoid `Sum`-style strategies on histogram fields).
//!
//! ## Entry timestamps are dropped
//!
//! OTel meter readers stamp measurements with their own clock, so the
//! per-entry timestamp emitted by `metrique` is informational only on this
//! path and is discarded. Once the descriptor system (#282) provides a
//! structural way to surface it (e.g. as an attribute or via a source
//! extractor) this can become opt-in.
//!
//! [flags]: crate::flags
//! [metrique-unit]: https://docs.rs/metrique

pub mod flags;
mod metrics;
mod translator;

use std::sync::Arc;
use std::time::Duration;

use metrique_writer::rate_limit::rate_limited;
use metrique_writer_core::sink::{AnyEntrySink, FlushWait};
use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};

use crate::{metrics::InstrumentCache, translator::append_with_pool};

/// Default OTel `InstrumentationScope` name when the caller does not set one
/// via [`OtelSinkBuilder::with_scope`].
const DEFAULT_SCOPE: &str = "metrique-otel";

/// The OTel-facing sink that aggregation pipelines flush through.
///
/// Construct it via [`OtelSink::builder`] (or [`OtelSink::with_otlp_default`])
/// and hand it to a [`KeyedAggregator`]; the recommended topology is
/// [`KeyedAggregator`] -> [`WorkerSink`] -> [`OtelSink`]. The aggregator
/// merges entries on a worker thread and flushes one observation per
/// `#[aggregate(key)]` tuple into this sink per flush interval, which is
/// where the OTel SDK actually sees them.
///
/// [`KeyedAggregator`]: metrique_aggregation::aggregator::KeyedAggregator
/// [`WorkerSink`]: metrique_aggregation::sink::WorkerSink
///
/// Internally, each flushed entry is walked once, the matching OTel
/// instrument is resolved via a shared lock-free cache, and observations are
/// recorded synchronously. There is no internal buffering on our side; the
/// OTel `PeriodicReader` is the queue. Call [`Self::flush_async`] /
/// [`Self::flush`] to force an export now; otherwise the provider exports on
/// its own schedule.
#[derive(Clone)]
pub struct OtelSink {
    /// Held separately so [`Self::flush`] / [`Self::flush_async`] can drive
    /// `force_flush` directly. `SdkMeterProvider` is internally `Arc`-backed,
    /// so the clone is cheap.
    meter_provider: SdkMeterProvider,
    /// Carries the instrument cache and the resolved scope name used by every
    /// `append` on this sink.
    inner: Arc<OtelSinkInner>,
}

pub(crate) struct OtelSinkInner {
    pub(crate) instruments: InstrumentCache,
    pub(crate) scope: &'static str,
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
        let meter = self.meter_provider.clone();
        FlushWait::from_future(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = meter.force_flush() {
                    rate_limited!(
                        Duration::from_secs(60),
                        tracing::warn!(error = %e, "metrique-otel: meter provider force_flush failed")
                    );
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
        if let Err(e) = self.meter_provider.force_flush() {
            rate_limited!(
                Duration::from_secs(60),
                tracing::warn!(error = %e, "metrique-otel: meter provider force_flush failed")
            );
        }
    }

    /// Build a sink whose meter provider is wired to an OTLP/gRPC exporter
    /// using the standard `OTEL_*` environment variables. Equivalent to
    /// `OtelSink::builder().with_otlp_default()`.
    ///
    /// Callers that want to bind the exporter to a specific runtime should
    /// go through the builder and use
    /// [`OtelSinkBuilder::with_runtime_handle`]. Callers outside any tokio
    /// runtime should use [`Self::with_otlp_http_default`], which uses a
    /// blocking HTTP/protobuf transport that does not need one.
    ///
    /// # Panics
    ///
    /// Must be called from within a tokio runtime. The tonic-backed OTLP
    /// exporter spawns export tasks on the current runtime; calling this
    /// outside of `#[tokio::main]` (or an explicit `Runtime::enter`) will
    /// panic.
    pub fn with_otlp_default() -> Result<Self, OtelSinkError> {
        OtelSinkBuilder::default().with_otlp_default()
    }

    /// Synchronous counterpart to [`Self::with_otlp_default`]: build a sink
    /// wired to an OTLP/HTTP+protobuf exporter using a blocking `reqwest`
    /// client. No tokio runtime is required at construction time. Equivalent
    /// to `OtelSink::builder().with_otlp_http_default()`.
    ///
    /// The export loop still runs on a background thread spawned by
    /// [`PeriodicReader`], but that thread is a plain OS thread, not a
    /// tokio task.
    ///
    /// [`PeriodicReader`]: opentelemetry_sdk::metrics::PeriodicReader
    pub fn with_otlp_http_default() -> Result<Self, OtelSinkError> {
        OtelSinkBuilder::default().with_otlp_http_default()
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
    runtime_handle: Option<tokio::runtime::Handle>,
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

    /// Bind the OTel SDK's runtime-dependent components (the tonic OTLP
    /// exporter, any tokio-driven reader) to an explicit
    /// [`tokio::runtime::Handle`] instead of the one captured by
    /// `Handle::current()` at build time.
    ///
    /// Resolution order at build time:
    /// 1. the handle passed here, if set
    /// 2. otherwise the ambient `Handle::try_current()`
    /// 3. otherwise the SDK's default behavior (typically panic for tonic, or
    ///    a plain OS thread for the HTTP transport / in-memory reader)
    ///
    /// Only [`OtelSink::with_otlp_default`] actually needs a runtime today;
    /// [`OtelSink::with_otlp_http_default`] uses a blocking HTTP client and a
    /// plain OS thread for the periodic reader. A handle set here is still
    /// honored if a future reader does need one.
    pub fn with_runtime_handle(mut self, handle: tokio::runtime::Handle) -> Self {
        self.runtime_handle = Some(handle);
        self
    }

    /// # Panics
    ///
    /// The default (no provider supplied) builds an empty [`SdkMeterProvider`]
    /// with no readers attached, which does not require a tokio runtime.
    ///
    /// If a meter provider is supplied via [`Self::with_meter_provider`] that
    /// carries a [`PeriodicReader`] (or any other reader that spawns onto the
    /// current runtime), `build` must be called from within a tokio runtime
    /// or be paired with [`Self::with_runtime_handle`]; the reader will
    /// panic otherwise.
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
        // Caller-supplied scopes are leaked exactly once at build time so the
        // OTel SDK's `meter(&'static str)` API can borrow them for the rest
        // of the process. The default literal is already `'static`.
        let scope: &'static str = match self.scope {
            Some(s) => Box::leak(s.into_boxed_str()),
            None => DEFAULT_SCOPE,
        };
        let inner = Arc::new(OtelSinkInner {
            instruments: InstrumentCache::new(meter_provider.clone()),
            scope,
        });
        OtelSink {
            meter_provider,
            inner,
        }
    }

    /// Build a sink wired to an OTLP/gRPC (tonic) exporter using the standard
    /// `OTEL_*` environment variables. If [`Self::with_runtime_handle`] was
    /// called, the tonic exporter binds to that runtime; otherwise it binds
    /// to the ambient `Handle::current()` and panics if there is none.
    pub fn with_otlp_default(self) -> Result<OtelSink, OtelSinkError> {
        let _guard = self.runtime_handle.as_ref().map(|h| h.enter());
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        drop(_guard);
        Ok(self.with_meter_provider(meter_provider).build())
    }

    /// Build a sink wired to an OTLP/HTTP+protobuf exporter using a blocking
    /// `reqwest` client. No tokio runtime is required.
    pub fn with_otlp_http_default(self) -> Result<OtelSink, OtelSinkError> {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .build()
            .map_err(|e| OtelSinkError::Otlp(Box::new(e)))?;
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        Ok(self.with_meter_provider(meter_provider).build())
    }
}

impl AnyEntrySink for OtelSink {
    fn append_any(&self, entry: impl metrique_writer_core::Entry + Send + 'static) {
        append_with_pool(&self.inner.instruments, self.inner.scope, &entry);
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

    use metrique_writer_core::Entry;
    use metrique_writer_core::entry::EntryWriter;
    use metrique_writer_core::sink::EntrySink;
    use metrique_writer_core::value::ForceFlag;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader};

    use super::*;
    use crate::flags::{Counter, Gauge, Histogram, UpDownCounter};

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
