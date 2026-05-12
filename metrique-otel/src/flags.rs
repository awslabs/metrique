// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::any::Any;

use metrique_writer_core::value::{FlagConstructor, ForceFlag, MetricFlags, MetricOptions};

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
        // Only OTEL options can merge with OTEL options, and only when they
        // agree on the instrument kind. Wrapping the same value with two
        // different kinds (e.g. `Counter<Gauge<T>>`) is unambiguous nonsense
        // and panicking via the `merge_assert_none` path is the right signal.
        let other = (other as &dyn Any).downcast_ref::<OtelOptions>()?;
        (other.kind == self.kind).then(|| MetricFlags::upcast(Self::static_ref(self.kind)))
    }
}

pub struct CounterCtor;
pub struct UpDownCounterCtor;
pub struct HistogramCtor;
pub struct GaugeCtor;

impl FlagConstructor for CounterCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::COUNTER)
    }
}

impl FlagConstructor for UpDownCounterCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::UP_DOWN_COUNTER)
    }
}

impl FlagConstructor for HistogramCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::HISTOGRAM)
    }
}

impl FlagConstructor for GaugeCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&OtelOptions::GAUGE)
    }
}

pub type Counter<T> = ForceFlag<T, CounterCtor>;
pub type UpDownCounter<T> = ForceFlag<T, UpDownCounterCtor>;
pub type Histogram<T> = ForceFlag<T, HistogramCtor>;
pub type Gauge<T> = ForceFlag<T, GaugeCtor>;
