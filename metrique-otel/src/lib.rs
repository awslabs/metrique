// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod flags;
mod metrics;
pub(crate) mod rate_limit;
mod translator;

use std::sync::Arc;
use std::time::Duration;

use crate::rate_limit::{RateLimiter, rate_limited};
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
    /// State shared by every `append` on this sink and its clones.
    inner: Arc<OtelSinkInner>,
}

pub(crate) struct OtelSinkInner {
    pub(crate) instruments: InstrumentCache,
    pub(crate) scope: &'static str,
    pub(crate) object_field_policy: ObjectFieldPolicy,
    /// Limits [`ObjectFieldPolicy::drop_field_and_warn`] warnings for this sink and its clones.
    pub(crate) object_field_warnings: RateLimiter,
}

/// What an [`OtelSink`] does with an object-valued field: a value written
/// through `ValueWriter::object` (such as a `format = AsObject` field), or a
/// list of them. Build one with [`ObjectFieldPolicy::drop_field`] or
/// [`ObjectFieldPolicy::drop_field_and_warn`] and pass it to
/// [`OtelSinkBuilder::with_object_field_policy`].
///
/// The OTel SDK records every distinct set of metric attributes as a separate
/// timeseries, and has nowhere else to put per-entry context, so an object
/// field is never attached as an attribute: each distinct object would create
/// a new timeseries on every metric in the entry. The sink drops the field and
/// records the rest of the entry; the policy chooses whether the drop is
/// logged. The sink does not report object fields as validation errors.
// The private enum keeps the variants out of the public API, so behaviours are added as new
// constructors without breaking callers.
#[derive(Clone, Debug, Default)]
pub struct ObjectFieldPolicy(pub(crate) ObjectPolicyInner);

#[derive(Clone, Debug, Default)]
pub(crate) enum ObjectPolicyInner {
    #[default]
    DropField,
    DropFieldAndWarn,
}

impl ObjectFieldPolicy {
    /// Drop the field silently. The default.
    pub const fn drop_field() -> Self {
        Self(ObjectPolicyInner::DropField)
    }

    /// Drop the field and emit a `warn` naming the field and the sink's
    /// instrumentation scope. The warning is rate-limited to one per minute
    /// per sink (clones of a sink share the limit), so it names only the first
    /// field the sink drops in each minute. If a sink drops some object field on
    /// most entries, other dropped fields on that sink are rarely named, so the
    /// warning is most useful when no object fields are expected.
    pub const fn drop_field_and_warn() -> Self {
        Self(ObjectPolicyInner::DropFieldAndWarn)
    }
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
    object_field_policy: ObjectFieldPolicy,
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

    /// Choose what the sink does with object-valued fields, which are never
    /// attached as attributes. Defaults to [`ObjectFieldPolicy::drop_field`].
    pub fn with_object_field_policy(mut self, policy: ObjectFieldPolicy) -> Self {
        self.object_field_policy = policy;
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
            object_field_policy: self.object_field_policy,
            object_field_warnings: RateLimiter::new(Duration::from_secs(60)),
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
        append_with_pool(&self.inner, &entry);
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

    mod objects {
        use metrique_writer_core::BoxEntry;
        use metrique_writer_core::Unit;
        use metrique_writer_core::descriptor::FieldShape;
        use metrique_writer_core::value::{
            AsObject, FormattedValue, NotLifted, ObjectValue, Quantized, Value, ValueFormatter,
            ValueWriter, WithDimensions,
        };
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use rstest::rstest;

        use super::*;

        struct Endpoint;

        impl ObjectValue for Endpoint {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Host", "example.com");
                writer.value("Port", &443u64);
            }
        }

        /// A hand-written `Value` that calls `object()` while keeping the `Opaque` shape.
        struct OpaqueObject;

        impl Value for OpaqueObject {
            const SHAPE: FieldShape<'static> = FieldShape::Opaque;
            const UNIT: Unit = Unit::None;

            fn write(&self, writer: impl ValueWriter) {
                writer.object(&Endpoint);
            }
        }

        /// A value declared as `Object` whose `write` panics, so a test fails if the sink writes
        /// the elements of a list of these.
        struct NeverWritten;

        impl Value for NeverWritten {
            const SHAPE: FieldShape<'static> = FieldShape::Object;
            const UNIT: Unit = Unit::None;

            fn write(&self, _writer: impl ValueWriter) {
                panic!("the sink wrote an element of a declared object list");
            }
        }

        /// Owns a `T` and writes it through `FormattedValue<T, AsObject, NotLifted>`, so an
        /// `AsObject` field can sit inside owning wrappers such as `ForceFlag`.
        struct OwnedAsObject<T>(T);

        impl<T> Value for OwnedAsObject<T>
        where
            AsObject: ValueFormatter<T, NotLifted>,
        {
            const SHAPE: FieldShape<'static> = <AsObject as ValueFormatter<T, NotLifted>>::SHAPE;
            const UNIT: Unit = Unit::None;

            fn write(&self, writer: impl ValueWriter) {
                FormattedValue::<T, AsObject, NotLifted>::new(&self.0).write(writer)
            }
        }

        /// One counter, a string attribute before and after the field under test, and the field.
        fn write_around_field<'a>(w: &mut impl EntryWriter<'a>, field: &(impl Value + ?Sized)) {
            let requests: ForceFlag<u64, Counter> = ForceFlag::from(1u64);
            w.value(Cow::Borrowed("Requests"), &requests);
            w.value(Cow::Borrowed("Operation"), "GET");
            w.value(Cow::Borrowed("Field"), field);
            w.value(Cow::Borrowed("Status"), "200");
        }

        /// The field under test is any owned `Value`.
        struct FieldEntry<V>(V);

        impl<V: Value> Entry for FieldEntry<V> {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                write_around_field(w, &self.0);
            }
        }

        /// The field under test is written as `FormattedValue<T, AsObject, NotLifted>`, the value
        /// a `format = AsObject` field produces.
        struct AsObjectEntry<T>(T);

        impl<T> Entry for AsObjectEntry<T>
        where
            AsObject: ValueFormatter<T, NotLifted>,
        {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                write_around_field(w, &FormattedValue::<T, AsObject, NotLifted>::new(&self.0));
            }
        }

        /// Run `append` against a fresh sink and return every `Requests` data point as its sorted
        /// attributes and its sum, sorted by attribute set. `policy` is passed to the builder when
        /// given; otherwise the builder's default applies.
        fn requests_data_points(
            policy: Option<ObjectFieldPolicy>,
            append: impl FnOnce(&OtelSink),
        ) -> Vec<(Vec<(String, String)>, u64)> {
            let exporter = InMemoryMetricExporter::default();
            let reader = PeriodicReader::builder(exporter.clone()).build();
            let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
            let mut builder = OtelSink::builder().with_meter_provider(meter_provider.clone());
            if let Some(policy) = policy {
                builder = builder.with_object_field_policy(policy);
            }
            let sink = builder.build();

            append(&sink);
            meter_provider.force_flush().expect("force_flush meter");

            let exported = exporter
                .get_finished_metrics()
                .expect("get_finished_metrics");
            let mut points = Vec::new();
            for m in exported
                .iter()
                .flat_map(|rm| rm.scope_metrics())
                .flat_map(|sm| sm.metrics())
                .filter(|m| m.name() == "Requests")
            {
                if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                    for dp in sum.data_points() {
                        let mut attrs: Vec<_> = dp
                            .attributes()
                            .map(|kv| (kv.key.to_string(), kv.value.as_str().into_owned()))
                            .collect();
                        attrs.sort();
                        points.push((attrs, dp.value()));
                    }
                }
            }
            points.sort();
            points
        }

        /// The message `ObjectFieldPolicy::drop_field_and_warn` logs for a dropped field.
        const DROP_WARNING: &str = "dropped an object-valued field";

        /// Run `f` with a subscriber that captures this thread's log output, and return its
        /// result with the captured text. The subscriber is installed for this thread only, so
        /// parallel tests don't see each other's events.
        fn captured_log<T>(f: impl FnOnce() -> T) -> (T, String) {
            #[derive(Clone, Default)]
            struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

            impl std::io::Write for Captured {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    self.0.lock().expect("capture lock").extend_from_slice(buf);
                    Ok(buf.len())
                }

                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let captured = Captured::default();
            let subscriber = tracing_subscriber::fmt()
                .with_writer({
                    let captured = captured.clone();
                    move || captured.clone()
                })
                .with_ansi(false)
                .finish();
            let result = tracing::subscriber::with_default(subscriber, f);
            let log = String::from_utf8(captured.0.lock().expect("capture lock").clone())
                .expect("utf-8 log");
            (result, log)
        }

        /// The number of drop warnings in `log`.
        fn drop_warnings(log: &str) -> usize {
            log.lines()
                .filter(|line| line.contains(DROP_WARNING))
                .count()
        }

        /// The single line of `log` that holds the drop warning.
        fn drop_warning_line(log: &str) -> &str {
            let mut lines = log.lines().filter(|line| line.contains(DROP_WARNING));
            let line = lines
                .next()
                .unwrap_or_else(|| panic!("no drop warning in: {log}"));
            assert!(
                lines.next().is_none(),
                "more than one drop warning in: {log}"
            );
            line
        }

        /// Append `entry` to a fresh sink built with `drop_field_and_warn()` and return the sorted
        /// attributes on its single `Requests` data point, with the number of drop warnings it
        /// logged. The sink is fresh, so its warning limiter is too, and the first dropped field
        /// always warns.
        fn requests_attributes(
            entry: impl Entry + Send + 'static,
        ) -> (Vec<(String, String)>, usize) {
            let (mut points, log) = captured_log(|| {
                requests_data_points(Some(ObjectFieldPolicy::drop_field_and_warn()), |sink| {
                    sink.append(entry)
                })
            });
            assert_eq!(
                points.len(),
                1,
                "expected one `Requests` data point, got {points:?}"
            );
            (points.remove(0).0, drop_warnings(&log))
        }

        /// Attributes and drop warnings for `make()` via both the direct and the boxed (dyn)
        /// path, asserted equal. The boxed path re-wraps list elements through the dyn bridge,
        /// which is where element-shape routing is most likely to diverge.
        fn attributes_and_warnings<E: Entry + Send + 'static>(
            make: impl Fn() -> E,
        ) -> (Vec<(String, String)>, usize) {
            let unboxed = requests_attributes(make());
            let boxed = requests_attributes(BoxEntry::new(make()));
            assert_eq!(unboxed, boxed, "boxed and unboxed results differ");
            unboxed
        }

        /// Attributes for a field that carries no object data: the object field policy must not
        /// be reached, so no drop warning is logged.
        fn attributes<E: Entry + Send + 'static>(make: impl Fn() -> E) -> Vec<(String, String)> {
            let (attrs, warnings) = attributes_and_warnings(make);
            assert_eq!(
                warnings, 0,
                "a field without object data reached the object field policy"
            );
            attrs
        }

        /// Attributes for an object field: the sink must drop it through the object field policy,
        /// which logs a drop warning, rather than discarding it some other way (such as through
        /// `error()`, which logs nothing).
        fn dropped_object_field<E: Entry + Send + 'static>(
            make: impl Fn() -> E,
        ) -> Vec<(String, String)> {
            let (attrs, warnings) = attributes_and_warnings(make);
            assert_eq!(
                warnings, 1,
                "expected the object field policy to drop the field"
            );
            attrs
        }

        /// The attributes `write_around_field` leaves when the field attaches nothing.
        fn without_field() -> Vec<(String, String)> {
            vec![
                ("Operation".to_owned(), "GET".to_owned()),
                ("Status".to_owned(), "200".to_owned()),
            ]
        }

        /// Every value shape `AsObject` formats (`O`, `Option<O>`, `Vec<O>`, `Option<Vec<O>>`),
        /// present and absent, attaches no attribute; the string attributes written before and
        /// after the field are kept. A present value is dropped through the object field policy;
        /// an absent one writes nothing.
        #[rstest]
        #[case::object(dropped_object_field(|| AsObjectEntry(Endpoint)))]
        #[case::some_object(dropped_object_field(|| AsObjectEntry(Some(Endpoint))))]
        #[case::none_object(attributes(|| AsObjectEntry(None::<Endpoint>)))]
        #[case::object_list(dropped_object_field(|| AsObjectEntry(vec![Endpoint, Endpoint])))]
        #[case::empty_object_list(dropped_object_field(|| AsObjectEntry(Vec::<Endpoint>::new())))]
        #[case::some_object_list(dropped_object_field(|| AsObjectEntry(Some(vec![Endpoint]))))]
        #[case::none_object_list(attributes(|| AsObjectEntry(None::<Vec<Endpoint>>)))]
        fn as_object_fields_are_not_attached(#[case] attributes: Vec<(String, String)>) {
            assert_eq!(attributes, without_field());
        }

        /// A `Value` that calls `object()` under the `Opaque` shape attaches no attribute.
        #[test]
        fn opaque_object_field_is_not_attached() {
            assert_eq!(
                dropped_object_field(|| FieldEntry(OpaqueObject)),
                without_field()
            );
        }

        /// A list whose element shape is not `Object` but contains `Object` inside
        /// `Optional`/`List` attaches no attribute, even when no element writes an object.
        #[rstest]
        #[case::list_of_none(dropped_object_field(|| FieldEntry(vec![None::<OwnedAsObject<Endpoint>>])))]
        #[case::list_of_empty_lists(dropped_object_field(|| FieldEntry(
            vec![Vec::<OwnedAsObject<Endpoint>>::new()],
        )))]
        #[case::empty_list_of_optional(dropped_object_field(|| FieldEntry(
            Vec::<OwnedAsObject<Option<Endpoint>>>::new(),
        )))]
        fn nested_object_shape_lists_are_not_attached(#[case] attributes: Vec<(String, String)>) {
            assert_eq!(attributes, without_field());
        }

        /// A list declared as holding objects is dropped without writing its elements.
        #[rstest]
        #[case::list(dropped_object_field(|| FieldEntry(vec![NeverWritten, NeverWritten])))]
        #[case::optional_list(dropped_object_field(|| FieldEntry(Some(vec![NeverWritten]))))]
        fn declared_object_list_elements_are_not_written(
            #[case] attributes: Vec<(String, String)>,
        ) {
            assert_eq!(attributes, without_field());
        }

        /// An `AsObject` field wrapped in `ForceFlag`, `WithDimensions`, or `Quantized` attaches
        /// no attribute, alone or as a list. The `*_elements` cases wrap each element of the list
        /// instead, so the list is dropped only if the wrapper keeps the element's `Object` shape.
        #[rstest]
        #[case::force_flag(dropped_object_field(|| FieldEntry(
            ForceFlag::<_, Counter>::from(OwnedAsObject(Endpoint)),
        )))]
        #[case::force_flag_list(dropped_object_field(|| FieldEntry(
            ForceFlag::<_, Counter>::from(OwnedAsObject(vec![Endpoint])),
        )))]
        #[case::with_dimensions(dropped_object_field(|| FieldEntry(
            WithDimensions::<_, 1>::new(OwnedAsObject(Endpoint), "Region", "us-east-1"),
        )))]
        #[case::with_dimensions_list(dropped_object_field(|| FieldEntry(
            WithDimensions::<_, 1>::new(OwnedAsObject(vec![Endpoint]), "Region", "us-east-1"),
        )))]
        #[case::quantized(dropped_object_field(|| FieldEntry(
            Quantized::<_>::from(OwnedAsObject(Endpoint)),
        )))]
        #[case::quantized_list(dropped_object_field(|| FieldEntry(
            Quantized::<_>::from(OwnedAsObject(vec![Endpoint])),
        )))]
        #[case::force_flag_elements(dropped_object_field(|| FieldEntry(
            vec![ForceFlag::<_, Counter>::from(OwnedAsObject(Endpoint))],
        )))]
        #[case::with_dimensions_elements(dropped_object_field(|| FieldEntry(
            vec![WithDimensions::<_, 1>::new(OwnedAsObject(Endpoint), "Region", "us-east-1")],
        )))]
        #[case::quantized_elements(dropped_object_field(|| FieldEntry(
            vec![Quantized::<_>::from(OwnedAsObject(Endpoint))],
        )))]
        #[case::box_elements(dropped_object_field(|| FieldEntry(
            vec![Box::new(OwnedAsObject(Endpoint))],
        )))]
        fn wrapped_object_fields_are_not_attached(#[case] attributes: Vec<(String, String)>) {
            assert_eq!(attributes, without_field());
        }

        /// Scalar lists, including optional, nested, and `None`-holding ones, attach as one
        /// comma-joined attribute.
        #[rstest]
        #[case::u64_list(attributes(|| FieldEntry(vec![1u64, 2])), "1,2")]
        #[case::str_list(attributes(|| FieldEntry(vec!["a", "b"])), "a,b")]
        #[case::string_list(attributes(|| FieldEntry(vec!["a".to_owned(), "b".to_owned()])), "a,b")]
        #[case::f64_list(attributes(|| FieldEntry(vec![1.5f64, 2.0])), "1.5,2")]
        #[case::optional_list(attributes(|| FieldEntry(Some(vec![1u64, 2]))), "1,2")]
        #[case::nested_list(attributes(|| FieldEntry(vec![vec![1u64], vec![2, 3]])), "1,2,3")]
        #[case::list_with_none(attributes(|| FieldEntry(vec![Some(1u64), None, Some(2)])), "1,2")]
        #[case::empty_list(attributes(|| FieldEntry(Vec::<u64>::new())), "")]
        fn scalar_lists_are_joined_attributes(
            #[case] attributes: Vec<(String, String)>,
            #[case] joined: &str,
        ) {
            assert_eq!(
                attributes,
                vec![
                    ("Field".to_owned(), joined.to_owned()),
                    ("Operation".to_owned(), "GET".to_owned()),
                    ("Status".to_owned(), "200".to_owned()),
                ]
            );
        }

        #[rstest]
        #[case::policy_default(ObjectFieldPolicy::default())]
        #[case::drop_field(ObjectFieldPolicy::drop_field())]
        #[case::builder_default(OtelSinkBuilder::default().object_field_policy)]
        fn builder_defaults_to_drop_field(#[case] policy: ObjectFieldPolicy) {
            assert!(
                matches!(policy.0, ObjectPolicyInner::DropField),
                "{policy:?}"
            );
        }

        /// `drop_field_and_warn` records the entry without the object field and emits a warning
        /// naming the field and the sink's scope.
        #[test]
        fn drop_field_and_warn_keeps_the_entry_and_warns() {
            let (points, log) = captured_log(|| {
                requests_data_points(Some(ObjectFieldPolicy::drop_field_and_warn()), |sink| {
                    sink.append(FieldEntry(OpaqueObject))
                })
            });

            assert_eq!(points, vec![(without_field(), 1)]);
            let warning = drop_warning_line(&log);
            assert!(
                warning.contains("WARN")
                    && warning.contains("field=Field")
                    && warning.contains("scope=metrique-otel"),
                "expected a WARN naming the field and scope, got: {warning}"
            );
        }

        /// The default policy, whether left implicit or set with `drop_field()`, drops object
        /// fields without logging.
        #[rstest]
        #[case::builder_default(None)]
        #[case::drop_field(Some(ObjectFieldPolicy::drop_field()))]
        fn drop_field_is_silent(#[case] policy: Option<ObjectFieldPolicy>) {
            let (points, log) = captured_log(|| {
                requests_data_points(policy, |sink| {
                    sink.append(FieldEntry(OpaqueObject));
                    sink.append(AsObjectEntry(vec![Endpoint]));
                })
            });

            assert_eq!(points, vec![(without_field(), 2)]);
            assert!(
                !log.contains(DROP_WARNING),
                "expected no warning, got: {log}"
            );
        }

        /// One sink warns at most once per interval and its clones share that limit; a separately
        /// built sink has its own limit.
        #[test]
        fn drop_warnings_are_limited_per_sink() {
            let (points, log) = captured_log(|| {
                requests_data_points(Some(ObjectFieldPolicy::drop_field_and_warn()), |sink| {
                    let clone = sink.clone();
                    sink.append(FieldEntry(OpaqueObject));
                    clone.append(FieldEntry(OpaqueObject));
                    OtelSink::builder()
                        .with_object_field_policy(ObjectFieldPolicy::drop_field_and_warn())
                        .build()
                        .append(FieldEntry(OpaqueObject));
                })
            });

            assert_eq!(points, vec![(without_field(), 2)]);
            assert_eq!(drop_warnings(&log), 2, "got: {log}");
        }

        /// The warning names the sink's configured scope.
        #[test]
        fn drop_warning_names_a_custom_scope() {
            let ((), log) = captured_log(|| {
                OtelSink::builder()
                    .with_scope("custom-scope")
                    .with_object_field_policy(ObjectFieldPolicy::drop_field_and_warn())
                    .build()
                    .append(FieldEntry(OpaqueObject));
            });

            let warning = drop_warning_line(&log);
            assert!(warning.contains("scope=custom-scope"), "got: {warning}");
        }
    }
}
