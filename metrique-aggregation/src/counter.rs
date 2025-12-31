//! Counter aggregation strategy.
//!
//! Counter sums values together, making it ideal for counts, totals, and accumulated metrics.

use crate::aggregate::AggregateValue;
use std::{marker::PhantomData, ops::AddAssign};

/// Counter aggregation strategy that sums values.
///
/// Use for request counts, error counts, bytes transferred, or any metric
/// where you want to sum values together.
pub struct Counter;

impl<T> AggregateValue<T> for Counter
where
    T: Default + AddAssign + Copy,
{
    type Aggregated = T;

    fn add_value(accum: &mut T, value: T) {
        *accum += value;
    }
}

/// Aggregation strategy that preserves the most recently set value
pub struct LastValueWins;

impl<T: Clone> AggregateValue<T> for LastValueWins {
    type Aggregated = Option<T>;

    fn add_value(accum: &mut Self::Aggregated, value: T) {
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

    fn add_value(accum: &mut Self::Aggregated, value: Option<T>) {
        if let Some(v) = value {
            <S as AggregateValue<T>>::add_value(accum, v);
        }
    }
}
