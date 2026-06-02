// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Instrument-kind tags for OTel-bound fields.
//!
//! Apply one of [`Counter`], [`UpDownCounter`], or [`Gauge`] via
//! `#[metrics(flags(...))]` to tell [`OtelSink`][crate::OtelSink] which OTel
//! instrument the field should record onto. Histograms use the standard
//! `Histogram` strategy from `metrique-aggregation` and need no flag; the
//! closed value advertises a `Distribution`, which the translator already
//! maps to a histogram instrument.
//!
//! Non-OTel sinks ignore these flags and treat the field as whatever its
//! aggregation strategy or value type already says it is, so the same struct
//! can be fanned out to OTel and EMF in parallel.
//!
//! ```
//! use metrique::unit_of_work::metrics;
//! use metrique_aggregation::aggregate;
//! use metrique_aggregation::value::Sum;
//! use metrique_otel::flags::Counter;
//!
//! #[aggregate]
//! #[metrics(rename_all = "PascalCase")]
//! struct Reqs {
//!     #[aggregate(strategy = Sum)]
//!     #[metrics(flags(Counter))]
//!     request_count: u64,
//! }
//! ```

use std::any::Any;
use std::time::Duration;

use crate::rate_limit::rate_limited;
use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};

fn warn_conflict(kept: InstrumentKind, dropped: InstrumentKind) {
    rate_limited!(
        Duration::from_secs(60),
        tracing::warn!(
            kept = ?kept,
            dropped = ?dropped,
            "metrique-otel: conflicting OTel instrument kinds wrapping the same value; first-wins"
        )
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InstrumentKind {
    Counter,
    UpDownCounter,
    Histogram,
    Gauge,
}

#[derive(Debug)]
pub(crate) struct OtelOptions {
    pub(crate) kind: InstrumentKind,
}

impl OtelOptions {
    pub(crate) const COUNTER: Self = Self {
        kind: InstrumentKind::Counter,
    };
    pub(crate) const UP_DOWN_COUNTER: Self = Self {
        kind: InstrumentKind::UpDownCounter,
    };
    pub(crate) const HISTOGRAM: Self = Self {
        kind: InstrumentKind::Histogram,
    };
    pub(crate) const GAUGE: Self = Self {
        kind: InstrumentKind::Gauge,
    };

    fn static_ref(kind: InstrumentKind) -> &'static OtelOptions {
        match kind {
            InstrumentKind::Counter => &Self::COUNTER,
            InstrumentKind::UpDownCounter => &Self::UP_DOWN_COUNTER,
            InstrumentKind::Histogram => &Self::HISTOGRAM,
            InstrumentKind::Gauge => &Self::GAUGE,
        }
    }
}

impl MetricOptions for OtelOptions {
    fn try_merge(&self, other: &dyn MetricOptions) -> Option<MetricFlags<'static>> {
        // Non-OTel options aren't ours to merge with; signal "not me" so
        // upstream can fall through to other merge strategies.
        let other = (other as &dyn Any).downcast_ref::<OtelOptions>()?;

        if other.kind == self.kind {
            return Some(MetricFlags::upcast(Self::static_ref(self.kind)));
        }

        // Conflicting kinds wrapping the same value (e.g. `Counter` + `Gauge`)
        // is a programming error, but a release-build panic deep inside
        // `Value::write` is worse than first-wins: it takes down the whole
        // entry. In `ForceFlag::write` (see metrique-writer-core) the inner
        // wrap is `self` here and the outer wrap is `other`, so "first wins"
        // = the inner-most kind survives — deterministic and documentable.
        debug_assert!(
            false,
            "conflicting OTel instrument kinds: kept {:?}, dropped {:?}",
            self.kind, other.kind
        );
        warn_conflict(self.kind, other.kind);
        Some(MetricFlags::upcast(Self::static_ref(self.kind)))
    }
}

/// Tag for fields that record onto an OTel monotonic counter.
#[non_exhaustive]
pub struct Counter;
/// Tag for fields that record onto an OTel up-down counter.
#[non_exhaustive]
pub struct UpDownCounter;
/// Tag for fields that record onto an OTel histogram instrument.
#[non_exhaustive]
pub struct Histogram;
/// Tag for fields that record onto an OTel asynchronous gauge.
#[non_exhaustive]
pub struct Gauge;

impl FlagConstructor for Counter {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::COUNTER)
    }
}

impl FlagConstructor for UpDownCounter {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::UP_DOWN_COUNTER)
    }
}

impl FlagConstructor for Histogram {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::HISTOGRAM)
    }
}

impl FlagConstructor for Gauge {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::GAUGE)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::panic::AssertUnwindSafe;
    use std::time::SystemTime;

    use metrique_writer_core::Entry;
    use metrique_writer_core::entry::EntryWriter;
    use metrique_writer_core::sink::EntrySink;
    use metrique_writer_core::value::ForceFlag;
    use opentelemetry_sdk::metrics::data::AggregatedMetrics;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    use super::{Counter, Gauge};
    use crate::OtelSink;

    /// Inner `Counter` + outer `Gauge` wrap of the same value used to be a
    /// release-build panic deep inside `MetricFlags::try_merge`. With
    /// first-wins semantics, the inner (Counter) kind survives, the outer
    /// (Gauge) is dropped, and a rate-limited warn is logged. In debug
    /// builds the `debug_assert!` panics on purpose so the conflict shows
    /// up loudly during development — this test handles both modes.
    #[test]
    fn conflicting_kinds_first_wins_no_release_panic() {
        struct ConflictEntry {
            v: ForceFlag<ForceFlag<u64, Counter>, Gauge>,
        }
        impl Entry for ConflictEntry {
            fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
                w.timestamp(SystemTime::now());
                w.value(Cow::Borrowed("Conflict"), &self.v);
            }
        }

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let sink = OtelSink::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            sink.append(ConflictEntry {
                v: ForceFlag::from(ForceFlag::from(7u64)),
            });
        }));

        if cfg!(debug_assertions) {
            // In debug, the `debug_assert!(false, ...)` must fire.
            assert!(
                result.is_err(),
                "debug build should panic via debug_assert on conflicting kinds"
            );
            return;
        }

        // In release, the conflict must not panic and the inner (Counter)
        // kind must win: the exported metric is a u64 Sum.
        result.expect("release build must not panic on conflicting kinds");
        meter_provider.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");
        let mut kind: Option<&'static str> = None;
        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() == "Conflict" {
                        kind = Some(match m.data() {
                            AggregatedMetrics::U64(_) => "u64",
                            AggregatedMetrics::I64(_) => "i64",
                            AggregatedMetrics::F64(_) => "f64",
                        });
                    }
                }
            }
        }
        assert_eq!(
            kind,
            Some("u64"),
            "inner Counter (first-wins) should produce a u64 sum, got {kind:?}"
        );
    }
}
