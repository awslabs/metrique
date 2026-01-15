//! Strategies for aggregating values

use metrique_writer::MetricValue;

use crate::{
    histogram::{Histogram, SortAndMerge},
    traits::AggregateValue,
};
use std::{marker::PhantomData, ops::AddAssign};

/// Sums values when aggregating
///
/// Use for request counts, error counts, bytes transferred, or any metric
/// where you want to sum values together.
pub struct Sum;

impl<T> AggregateValue<T> for Sum
where
    T: Default + AddAssign,
{
    type Aggregated = T;

    fn insert(accum: &mut T, value: T) {
        *accum += value;
    }
}

/// Aggregation strategy that preserves the most recently set value
pub struct KeepLast;

impl<T: Clone> AggregateValue<T> for KeepLast {
    type Aggregated = Option<T>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        *accum = Some(value)
    }
}

/// Wrap a given strategy to support optional values by ignoring `None`
pub struct MergeOptions<Inner> {
    _data: PhantomData<Inner>,
}

impl<T, S> AggregateValue<Option<T>> for MergeOptions<S>
where
    S: AggregateValue<T>,
{
    type Aggregated = S::Aggregated;

    fn insert(accum: &mut Self::Aggregated, value: Option<T>) {
        if let Some(v) = value {
            <S as AggregateValue<T>>::insert(accum, v);
        }
    }
}

/// Helper wrapper used by the aggregate macro to automatically copy Copy types in MergeRef
pub struct CopyWrapper<Inner> {
    data: PhantomData<Inner>,
}

impl<'a, T, S> AggregateValue<&'a T> for CopyWrapper<S>
where
    T: Copy,
    S: AggregateValue<T>,
{
    type Aggregated = S::Aggregated;

    fn insert(accum: &mut Self::Aggregated, value: &'a T) {
        <S as AggregateValue<T>>::insert(accum, *value);
    }
}

/// Flatten strategy for fields that already implement Merge
///
/// Use this when you want to aggregate a field that is itself an aggregatable type.
pub struct Flatten;

impl<T> AggregateValue<T> for Flatten
where
    T: crate::traits::Merge,
{
    type Aggregated = T::Merged;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        T::merge(accum, value);
    }
}

/// Distribution preserves all values while compressing duplicates
///
/// This is effectively a type alias for `Histogram<T, SortAndMerge>`, however,
/// when used as an aggregate strategy, it avoids the needs to name `T`.
pub struct Distribution;
impl<T: MetricValue> AggregateValue<T> for Distribution {
    type Aggregated = Histogram<T, SortAndMerge>;

    fn insert(accum: &mut Self::Aggregated, value: T) {
        accum.add_value(value);
    }
}

/// Key type for aggregations with no key fields
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct NoKey;

impl<T> crate::traits::Key<T> for NoKey {
    type Key<'a> = NoKey;

    fn from_source(_source: &T) -> Self::Key<'_> {
        NoKey
    }

    fn static_key<'a>(_key: &Self::Key<'a>) -> Self::Key<'static> {
        NoKey
    }

    fn static_key_matches<'a>(_owned: &Self::Key<'static>, _borrowed: &Self::Key<'a>) -> bool {
        true
    }
}

impl metrique_core::CloseValue for NoKey {
    type Closed = Self;

    fn close(self) -> Self::Closed {
        self
    }
}

impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for NoKey {
    fn write<'a>(&'a self, _w: &mut impl metrique_writer::EntryWriter<'a>) {}
}

impl metrique_writer::Entry for NoKey {
    fn write<'a>(&'a self, _w: &mut impl metrique_writer::EntryWriter<'a>) {}

    fn sample_group(
        &self,
    ) -> impl Iterator<Item = metrique_writer_core::entry::SampleGroupElement> {
        std::iter::empty()
    }
}
