// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Built-in aggregation strategies for common metric patterns.

use super::AggregateValue;
use crate::value::{MetricFlags, Observation, Value, ValueWriter};
use crate::unit::Unit;
use std::ops::AddAssign;

/// Simple vector-based histogram for demonstration.
///
/// Stores all individual values. Obviously inefficient but shows type transformation.
#[derive(Debug, Clone, PartialEq)]
pub struct VecHistogram {
    values: Vec<u64>,
}

impl Value for VecHistogram {
    fn write(&self, writer: impl ValueWriter) {
        // Write as metric with multiple observations
        writer.metric(
            self.values.iter().map(|&v| Observation::Floating(v as f64)),
            Unit::None,
            std::iter::empty(),
            MetricFlags::empty(),
        );
    }
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

/// Histogram aggregation strategy - collects all values.
///
/// Use for latency, size distributions, etc. Input type is u64, aggregated type is VecHistogram.
pub struct Histogram;

impl AggregateValue<u64> for Histogram {
    type Aggregated = VecHistogram;

    fn init() -> Self::Aggregated {
        VecHistogram { values: Vec::new() }
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &u64) {
        accum.values.push(*value);
    }
}

/// Counter aggregation strategy - sums values.
///
/// Use for metrics that accumulate over time (request counts, error counts, bytes transferred).
pub struct Counter;

impl<T> AggregateValue<T> for Counter
where
    T: Default + AddAssign + Copy,
{
    type Aggregated = T;

    fn init() -> Self::Aggregated {
        T::default()
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
        *accum += *value;
    }
}

/// Gauge aggregation strategy - keeps the last value.
///
/// Use for metrics that represent a current state (active connections, memory usage, queue depth).
pub struct Gauge;

impl<T: Clone> AggregateValue<T> for Gauge {
    type Aggregated = T;

    fn init() -> Self::Aggregated {
        panic!("Gauge requires at least one value to aggregate")
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
        *accum = value.clone();
    }
}

/// Max aggregation strategy - keeps the maximum value.
///
/// Use for tracking peak values (max latency, peak memory, highest queue depth).
pub struct Max;

impl<T> AggregateValue<T> for Max
where
    T: Ord + Clone,
{
    type Aggregated = Option<T>;

    fn init() -> Self::Aggregated {
        None
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
        match accum {
            None => *accum = Some(value.clone()),
            Some(current) if value > current => *accum = Some(value.clone()),
            _ => {}
        }
    }
}

/// Min aggregation strategy - keeps the minimum value.
///
/// Use for tracking minimum values (min latency, lowest queue depth).
pub struct Min;

impl<T> AggregateValue<T> for Min
where
    T: Ord + Clone,
{
    type Aggregated = Option<T>;

    fn init() -> Self::Aggregated {
        None
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
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
        Counter::aggregate(&mut accum, &10u64);
        Counter::aggregate(&mut accum, &25u64);
        Counter::aggregate(&mut accum, &15u64);
        assert_eq!(accum, 50);
    }

    #[test]
    fn gauge_keeps_last() {
        let mut accum = 0u64;
        Gauge::aggregate(&mut accum, &10);
        Gauge::aggregate(&mut accum, &25);
        Gauge::aggregate(&mut accum, &15);
        assert_eq!(accum, 15);
    }

    #[test]
    fn max_tracks_maximum() {
        let mut accum = Max::init();
        Max::aggregate(&mut accum, &10u64);
        Max::aggregate(&mut accum, &25u64);
        Max::aggregate(&mut accum, &15u64);
        assert_eq!(accum, Some(25));
    }

    #[test]
    fn min_tracks_minimum() {
        let mut accum = Min::init();
        Min::aggregate(&mut accum, &10u64);
        Min::aggregate(&mut accum, &25u64);
        Min::aggregate(&mut accum, &15u64);
        assert_eq!(accum, Some(10));
    }

    #[test]
    fn histogram_collects_values() {
        let mut accum = Histogram::init();
        Histogram::aggregate(&mut accum, &50);
        Histogram::aggregate(&mut accum, &75);
        Histogram::aggregate(&mut accum, &100);
        
        assert_eq!(accum.count(), 3);
        assert_eq!(accum.sum(), 225);
        assert_eq!(accum.avg(), 75);
        assert_eq!(accum.min(), Some(50));
        assert_eq!(accum.max(), Some(100));
    }
}
