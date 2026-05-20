//! OTel-aware aggregation strategies that bundle merge behavior, unit, and
//! OTel instrument kind into a single `#[aggregate(strategy = …)]` annotation.
//!
//! Each strategy here is a thin layer over a `metrique-aggregation` primitive
//! plus a wrapping at close time:
//!
//! - [`OtelCounter<U>`]: sums values; closes as [`ForceFlag`]`<`[`WithUnit`]`<T, U>,`[`flags::Counter`]`>`.
//! - [`OtelUpDownCounter<U>`]: sums signed values; closes as [`ForceFlag`]`<`[`WithUnit`]`<T, U>,`[`flags::UpDownCounter`]`>`.
//! - [`OtelGauge<U>`]: keep-last; closes as [`ForceFlag`]`<`[`WithUnit`]`<T, U>,`[`flags::Gauge`]`>`.
//! - [`OtelHistogram<U>`]: collects a distribution; closes as [`WithUnit`]`<`[`HistogramClosed`]`<T>, U>`.
//!
//! `T` is supplied by the aggregate macro from the source field type, so it
//! never has to be named in user code. The default for `U` is
//! [`metrique_writer_core::unit::None`] (dimensionless).

use std::marker::PhantomData;
use std::ops::AddAssign;

use metrique_aggregation::__macro_plumbing::AggregateValue;
use metrique_aggregation::histogram::{
    ExponentialAggregationStrategy, Histogram as RawHistogram, HistogramClosed,
};
use metrique_core::CloseValue;
use metrique_writer_core::MetricValue;
use metrique_writer_core::unit::{Convert, None as Dimensionless, UnitTag, WithUnit};
use metrique_writer_core::value::ForceFlag;

use crate::flags;

// --- Counter ---------------------------------------------------------------

/// Sum values; emit as an OTel monotonic counter with unit `U`.
pub struct OtelCounter<U = Dimensionless>(PhantomData<U>);

/// Accumulator for [`OtelCounter`]. Holds the running sum.
pub struct OtelCounterAccum<T, U> {
    sum: T,
    _u: PhantomData<U>,
}

impl<T: Default, U> Default for OtelCounterAccum<T, U> {
    fn default() -> Self {
        Self {
            sum: T::default(),
            _u: PhantomData,
        }
    }
}

impl<T, U> AggregateValue<T> for OtelCounter<U>
where
    T: Default + AddAssign,
{
    type Aggregated = OtelCounterAccum<T, U>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        accum.sum += value;
    }
}

impl<T, U> CloseValue for OtelCounterAccum<T, U>
where
    T: MetricValue,
    T::Unit: Convert<U>,
    U: UnitTag,
{
    type Closed = ForceFlag<WithUnit<T, U>, flags::Counter>;

    fn close(self) -> Self::Closed {
        ForceFlag::from(WithUnit::from(self.sum))
    }
}

// --- UpDownCounter ---------------------------------------------------------

/// Sum values (signed); emit as an OTel up-down counter with unit `U`.
pub struct OtelUpDownCounter<U = Dimensionless>(PhantomData<U>);

/// Accumulator for [`OtelUpDownCounter`].
pub struct OtelUpDownCounterAccum<T, U> {
    sum: T,
    _u: PhantomData<U>,
}

impl<T: Default, U> Default for OtelUpDownCounterAccum<T, U> {
    fn default() -> Self {
        Self {
            sum: T::default(),
            _u: PhantomData,
        }
    }
}

impl<T, U> AggregateValue<T> for OtelUpDownCounter<U>
where
    T: Default + AddAssign,
{
    type Aggregated = OtelUpDownCounterAccum<T, U>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        accum.sum += value;
    }
}

impl<T, U> CloseValue for OtelUpDownCounterAccum<T, U>
where
    T: MetricValue,
    T::Unit: Convert<U>,
    U: UnitTag,
{
    type Closed = ForceFlag<WithUnit<T, U>, flags::UpDownCounter>;

    fn close(self) -> Self::Closed {
        ForceFlag::from(WithUnit::from(self.sum))
    }
}

// --- Gauge -----------------------------------------------------------------

/// Keep-last-value semantics; emit as an OTel gauge with unit `U`.
///
/// Each `insert` replaces the previous value. On flush, the most recent value
/// is what gets emitted. If no value was ever inserted, no metric is emitted
/// for this field.
pub struct OtelGauge<U = Dimensionless>(PhantomData<U>);

/// Accumulator for [`OtelGauge`].
pub struct OtelGaugeAccum<T, U> {
    last: Option<T>,
    _u: PhantomData<U>,
}

impl<T, U> Default for OtelGaugeAccum<T, U> {
    fn default() -> Self {
        Self {
            last: None,
            _u: PhantomData,
        }
    }
}

impl<T, U> AggregateValue<T> for OtelGauge<U> {
    type Aggregated = OtelGaugeAccum<T, U>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        accum.last = Some(value);
    }
}

impl<T, U> CloseValue for OtelGaugeAccum<T, U>
where
    T: MetricValue,
    T::Unit: Convert<U>,
    U: UnitTag,
{
    type Closed = Option<ForceFlag<WithUnit<T, U>, flags::Gauge>>;

    fn close(self) -> Self::Closed {
        self.last.map(|v| ForceFlag::from(WithUnit::from(v)))
    }
}

// --- Histogram -------------------------------------------------------------

/// Collect observations into a distribution; emit as an OTel histogram with
/// unit `U`.
pub struct OtelHistogram<U = Dimensionless>(PhantomData<U>);

/// Accumulator for [`OtelHistogram`].
pub struct OtelHistogramAccum<T, U> {
    inner: RawHistogram<T, ExponentialAggregationStrategy>,
    _u: PhantomData<U>,
}

impl<T, U> Default for OtelHistogramAccum<T, U> {
    fn default() -> Self {
        Self {
            inner: RawHistogram::<T, ExponentialAggregationStrategy>::default(),
            _u: PhantomData,
        }
    }
}

impl<T, U> AggregateValue<T> for OtelHistogram<U>
where
    T: MetricValue,
{
    type Aggregated = OtelHistogramAccum<T, U>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        accum.inner.add_value(value);
    }
}

impl<T, U> CloseValue for OtelHistogramAccum<T, U>
where
    T: MetricValue,
    T::Unit: Convert<U>,
    U: UnitTag,
{
    type Closed = WithUnit<HistogramClosed<T>, U>;

    fn close(self) -> Self::Closed {
        WithUnit::from(self.inner.close())
    }
}
