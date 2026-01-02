//! Strategies for aggregating values

use crate::traits::AggregateValue;
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

    fn add_value(accum: &mut T, value: T) {
        *accum += value;
    }
}

/// Aggregation strategy that preserves the most recently set value
///
/// NOTE: When using this strategy with types that are not copy, you
/// will need to use `aggregate(owned)`
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

/// Helper struct used by the proc macro to attempt to copy values
pub struct IfYouSeeThisUseAggregateOwned<Inner> {
    data: PhantomData<Inner>,
}

impl<'a, T, S> AggregateValue<&'a T> for IfYouSeeThisUseAggregateOwned<S>
where
    T: Copy,
    S: AggregateValue<T>,
{
    type Aggregated = S::Aggregated;

    fn add_value(accum: &mut Self::Aggregated, value: &'a T) {
        <S as AggregateValue<T>>::add_value(accum, *value);
    }
}
