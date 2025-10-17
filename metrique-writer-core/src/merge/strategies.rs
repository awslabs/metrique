// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Built-in aggregation strategies for common metric patterns.

use super::AggregateValue;
use crate::MetricValue;
use crate::unit::{Unit, UnitTag};
use crate::value::{MetricFlags, Observation, Value, ValueWriter};
use std::ops::AddAssign;

/// Histogram aggregation strategy - collects all values.
///
/// Use for latency, size distributions, etc. Works with any type that implements MetricValue.
pub struct VecHistogram;

impl<T> AggregateValue<T> for VecHistogram
where
    T: MetricValue,
{
    type Aggregated = HistogramValue<T>;

    fn init() -> Self::Aggregated {
        HistogramValue {
            observations: Vec::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
        // Use a collector that directly mutates our observations vector
        let collector = ObservationCollector::new(&mut accum.observations);
        value.write(collector);
    }
}

/// A histogram value that holds observations and unit information.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramValue<T: MetricValue> {
    observations: Vec<Observation>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: MetricValue> Value for HistogramValue<T> {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            self.observations.iter().copied(),
            T::Unit::UNIT,
            std::iter::empty(),
            MetricFlags::empty(),
        );
    }
}

impl<T: MetricValue> HistogramValue<T> {
    /// Get the count of observations.
    pub fn count(&self) -> usize {
        self.observations.len()
    }

    /// Get all observations as a slice.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }
}

/// A ValueWriter that collects observations for histogram aggregation.
struct ObservationCollector<'a> {
    observations: &'a mut Vec<Observation>,
}

impl<'a> ObservationCollector<'a> {
    fn new(observations: &'a mut Vec<Observation>) -> Self {
        Self { observations }
    }
}

impl ValueWriter for ObservationCollector<'_> {
    fn string(self, _value: &str) {
        // Ignore string values for histogram
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        _unit: Unit,
        _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        self.observations.extend(distribution);
    }

    fn error(self, _error: crate::ValidationError) {
        // Ignore errors for histogram
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
        let mut accum = VecHistogram::init();
        VecHistogram::aggregate(&mut accum, &50);
        VecHistogram::aggregate(&mut accum, &75);
        VecHistogram::aggregate(&mut accum, &100);

        assert_eq!(accum.count(), 3);
        assert_eq!(accum.sum(), 225);
        assert_eq!(accum.avg(), 75);
        assert_eq!(accum.min(), Some(50));
        assert_eq!(accum.max(), Some(100));
    }
}
