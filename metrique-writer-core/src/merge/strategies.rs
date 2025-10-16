// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Built-in merge strategies for common metric patterns.

use super::MergeValue;
use std::ops::AddAssign;

/// Counter merge strategy - sums values.
///
/// Use for metrics that accumulate over time (request counts, error counts, bytes transferred).
pub struct Counter;

impl<T> MergeValue<T> for Counter
where
    T: Default + AddAssign + Copy,
{
    type Merged = T;

    fn init() -> Self::Merged {
        T::default()
    }

    fn merge(accum: &mut Self::Merged, value: &T) {
        *accum += *value;
    }
}

/// Gauge merge strategy - keeps the last value.
///
/// Use for metrics that represent a current state (active connections, memory usage, queue depth).
pub struct Gauge;

impl<T: Clone> MergeValue<T> for Gauge {
    type Merged = T;

    fn init() -> Self::Merged {
        panic!("Gauge requires at least one value to merge")
    }

    fn merge(accum: &mut Self::Merged, value: &T) {
        *accum = value.clone();
    }
}

/// Max merge strategy - keeps the maximum value.
///
/// Use for tracking peak values (max latency, peak memory, highest queue depth).
pub struct Max;

impl<T> MergeValue<T> for Max
where
    T: Ord + Clone,
{
    type Merged = Option<T>;

    fn init() -> Self::Merged {
        None
    }

    fn merge(accum: &mut Self::Merged, value: &T) {
        match accum {
            None => *accum = Some(value.clone()),
            Some(current) if value > current => *accum = Some(value.clone()),
            _ => {}
        }
    }
}

/// Min merge strategy - keeps the minimum value.
///
/// Use for tracking minimum values (min latency, lowest queue depth).
pub struct Min;

impl<T> MergeValue<T> for Min
where
    T: Ord + Clone,
{
    type Merged = Option<T>;

    fn init() -> Self::Merged {
        None
    }

    fn merge(accum: &mut Self::Merged, value: &T) {
        match accum {
            None => *accum = Some(value.clone()),
            Some(current) if value < current => *accum = Some(value.clone()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_sums_values() {
        let mut accum = Counter::init();
        Counter::merge(&mut accum, &10u64);
        Counter::merge(&mut accum, &25u64);
        Counter::merge(&mut accum, &15u64);
        assert_eq!(accum, 50);
    }

    #[test]
    fn gauge_keeps_last() {
        let mut accum = 0u64;
        Gauge::merge(&mut accum, &10);
        Gauge::merge(&mut accum, &25);
        Gauge::merge(&mut accum, &15);
        assert_eq!(accum, 15);
    }

    #[test]
    fn max_tracks_maximum() {
        let mut accum = Max::init();
        Max::merge(&mut accum, &10u64);
        Max::merge(&mut accum, &25u64);
        Max::merge(&mut accum, &15u64);
        assert_eq!(accum, Some(25));
    }

    #[test]
    fn min_tracks_minimum() {
        let mut accum = Min::init();
        Min::merge(&mut accum, &10u64);
        Min::merge(&mut accum, &25u64);
        Min::merge(&mut accum, &15u64);
        assert_eq!(accum, Some(10));
    }
}
