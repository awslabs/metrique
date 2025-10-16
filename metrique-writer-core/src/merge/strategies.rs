// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Built-in merge strategies for common metric patterns.

use super::MergeValue;
use std::ops::AddAssign;

/// Simple vector-based histogram for demonstration.
///
/// Stores all individual values. Obviously inefficient but shows type transformation.
#[derive(Debug, Clone, PartialEq)]
pub struct VecHistogram {
    values: Vec<u64>,
}

impl VecHistogram {
    /// Get the count of values.
    pub fn count(&self) -> usize {
        self.values.len()
    }

    /// Get the sum of all values.
    pub fn sum(&self) -> u64 {
        self.values.iter().sum()
    }

    /// Get the average value.
    pub fn avg(&self) -> u64 {
        if self.values.is_empty() {
            0
        } else {
            self.sum() / self.values.len() as u64
        }
    }

    /// Get the minimum value.
    pub fn min(&self) -> Option<u64> {
        self.values.iter().copied().min()
    }

    /// Get the maximum value.
    pub fn max(&self) -> Option<u64> {
        self.values.iter().copied().max()
    }
}

/// Histogram merge strategy - collects all values.
///
/// Use for latency, size distributions, etc. Input type is u64, merged type is VecHistogram.
pub struct Histogram;

impl MergeValue<u64> for Histogram {
    type Merged = VecHistogram;

    fn init() -> Self::Merged {
        VecHistogram { values: Vec::new() }
    }

    fn merge(accum: &mut Self::Merged, value: &u64) {
        accum.values.push(*value);
    }
}

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

    #[test]
    fn histogram_collects_values() {
        let mut accum = Histogram::init();
        Histogram::merge(&mut accum, &50);
        Histogram::merge(&mut accum, &75);
        Histogram::merge(&mut accum, &100);
        
        assert_eq!(accum.count(), 3);
        assert_eq!(accum.sum(), 225);
        assert_eq!(accum.avg(), 75);
        assert_eq!(accum.min(), Some(50));
        assert_eq!(accum.max(), Some(100));
    }
}
