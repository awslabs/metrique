// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Flag markers consumed by [`OtelSink`](crate::OtelSink).
//!
//! Apply via `#[metrics(flags(...))]` to declare the OTel instrument kind the
//! sink should record observations against. At write time the sink downcasts
//! [`MetricFlags`] to [`OtelOptions`] to pick the kind.
//!
//! ```ignore
//! use metrique::unit_of_work::metrics;
//! use metrique_otel::flags::{Counter, Histogram};
//!
//! #[metrics(rename_all = "PascalCase")]
//! struct RequestMetrics {
//!     operation: String,
//!     #[metrics(flags(Counter))]   request_count: u64,
//!     #[metrics(flags(Histogram))] latency_ms: std::time::Duration,
//! }
//! ```

use std::any::Any;

use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};

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
        // Only OTEL options merge with OTEL options, and only when they agree
        // on the instrument kind. Wrapping the same value with two different
        // kinds (e.g. `Counter` + `Gauge`) is nonsense; panicking via the
        // `merge_assert_none` path is the right signal.
        let other = (other as &dyn Any).downcast_ref::<OtelOptions>()?;
        (other.kind == self.kind).then(|| MetricFlags::upcast(Self::static_ref(self.kind)))
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
