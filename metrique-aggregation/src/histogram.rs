//! Histogram types for aggregating multiple observations into distributions.
//!
//! When emitting high-frequency metrics, you often want to aggregate multiple observations
//! into a single metric entry rather than emitting each one individually. This module provides
//! histogram types that collect observations and emit them as distributions.
//!
//! # When to use histograms
//!
//! Use histograms when you have many observations of the same metric within a single unit of work:
//!
//! - A distributed query that fans out to multiple backend services
//! - Processing a batch of items where you want to track per-item latency
//! - Any operation that generates multiple measurements you want to aggregate
//!
//! For most applications, [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md)
//! is a better approach than aggregation. Consider histograms when you need precise distributions
//! for high-frequency events.
//!
//! # Example
//!
//! ```
//! use metrique::unit_of_work::metrics;
//! use metrique_aggregation::histogram::Histogram;
//! use metrique_writer::unit::Millisecond;
//! use std::time::Duration;
//!
//! #[metrics(rename_all = "PascalCase")]
//! struct QueryMetrics {
//!     query_id: String,
//!
//!     #[metrics(unit = Millisecond)]
//!     backend_latency: Histogram<Duration>,
//! }
//!
//! fn execute_query(query_id: String) {
//!     let mut metrics = QueryMetrics {
//!         query_id,
//!         backend_latency: Histogram::default(),
//!     };
//!
//!     // Record multiple observations
//!     metrics.backend_latency.add_value(Duration::from_millis(45));
//!     metrics.backend_latency.add_value(Duration::from_millis(67));
//!     metrics.backend_latency.add_value(Duration::from_millis(52));
//!
//!     // When metrics drops, emits a single entry with the distribution
//! }
//! ```
//!
//! # Choosing an aggregation strategy
//!
//! By default, histograms use [`ExponentialAggregationStrategy`]. To use a different strategy,
//! specify it as the second type parameter:
//!
//! ```
//! use metrique_aggregation::histogram::{Histogram, SortAndMerge};
//! use std::time::Duration;
//!
//! let histogram: Histogram<Duration, SortAndMerge> = Histogram::new(SortAndMerge::new());
//! ```
//!
//! ## ExponentialAggregationStrategy (default)
//!
//! Uses exponential bucketing with ~6.25% error. This is the best choice for most use cases:
//!
//! - Provides consistent relative precision across wide value ranges
//! - Memory efficient with fixed bucket count (464 buckets)
//! - Fast recording and draining operations
//!
//! Use this when you need good precision across values that span multiple orders of magnitude
//! (e.g., latencies from microseconds to seconds).
//!
//! ## AtomicExponentialAggregationStrategy
//!
//! Thread-safe version of exponential bucketing. Use with [`crate::histogram::SharedHistogram`] when you need
//! to record values from multiple threads concurrently:
//!
//! ```
//! use metrique_aggregation::histogram::{SharedHistogram, AtomicExponentialAggregationStrategy};
//! use std::time::Duration;
//!
//! let histogram: SharedHistogram<Duration, AtomicExponentialAggregationStrategy> =
//!     SharedHistogram::new(AtomicExponentialAggregationStrategy::new());
//! ```
//!
//! ## SortAndMerge
//!
//! Stores all observations exactly and sorts them on emission:
//!
//! - Perfect precision - no bucketing error
//! - Memory usage grows with observation count
//! - Slower drain operation due to sorting
//!
//! Use this when you need exact values and have a bounded number of observations (typically < 1000).

use metrique_core::CloseValue;
use metrique_writer::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
use smallvec::SmallVec;
use std::marker::PhantomData;

/// Strategy for aggregating observations in a histogram.
///
/// Implementations determine how values are stored and converted to observations
/// when the histogram is closed.
pub trait AggregationStrategy {
    /// Record a single observation.
    fn record(&mut self, value: f64);

    /// Drain all observations and return them as a vector.
    ///
    /// This resets the strategy's internal state.
    fn drain(&mut self) -> Vec<Observation>;
}

/// Thread-safe strategy for aggregating observations in a histogram.
///
/// Like [`AggregationStrategy`] but allows recording values through a shared reference.
pub trait SharedAggregationStrategy {
    /// Record a single observation through a shared reference.
    fn record(&self, value: f64);

    /// Drain all observations and return them as a vector.
    ///
    /// This resets the strategy's internal state.
    fn drain(&self) -> Vec<Observation>;
}

/// A histogram that collects multiple observations and emits them as a distribution.
///
/// Use this when you have many observations of the same metric within a single unit of work.
/// The histogram aggregates values in memory and emits them as a single metric entry.
///
/// Requires `&mut self` to add values. For thread-safe access, use [`SharedHistogram`].
pub struct Histogram<T, S = ExponentialAggregationStrategy> {
    strategy: S,
    _value: PhantomData<T>,
}

impl<T, S: AggregationStrategy> Histogram<T, S> {
    /// Create a new histogram with the given aggregation strategy.
    pub fn new(strategy: S) -> Self {
        Self {
            strategy,
            _value: PhantomData,
        }
    }

    /// Add a value to the histogram.
    ///
    /// The value is converted to observations using the metric value's implementation,
    /// then recorded in the aggregation strategy.
    pub fn add_value(&mut self, value: T)
    where
        T: MetricValue,
    {
        struct Capturer<'a, S>(&'a mut S);
        impl<'b, S: AggregationStrategy> ValueWriter for Capturer<'b, S> {
            fn string(self, _value: &str) {}
            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                _unit: metrique_writer::Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                for obs in distribution {
                    match obs {
                        Observation::Unsigned(v) => self.0.record(v as f64),
                        Observation::Floating(v) => self.0.record(v),
                        Observation::Repeated { total, occurrences } => {
                            let avg = total / occurrences as f64;
                            for _ in 0..occurrences {
                                self.0.record(avg);
                            }
                        }
                        _ => {}
                    }
                }
            }
            fn error(self, _error: metrique_writer::ValidationError) {}
        }

        let capturer = Capturer(&mut self.strategy);
        value.write(capturer);
    }
}

impl<T, S: Default + AggregationStrategy> Default for Histogram<T, S> {
    fn default() -> Self {
        Self::new(S::default())
    }
}

impl<T: MetricValue, S: AggregationStrategy> CloseValue for Histogram<T, S> {
    type Closed = HistogramClosed<T>;

    fn close(mut self) -> Self::Closed {
        HistogramClosed {
            observations: self.strategy.drain(),
            _value: PhantomData,
        }
    }
}

/// Thread-safe histogram that collects multiple observations and emits them as a distribution.
///
/// Like [`Histogram`] but allows adding values through a shared reference, making it
/// suitable for concurrent access patterns.
pub struct SharedHistogram<T, S = AtomicExponentialAggregationStrategy> {
    strategy: S,
    _value: PhantomData<T>,
}

impl<T, S: Default> Default for SharedHistogram<T, S> {
    fn default() -> Self {
        Self {
            strategy: Default::default(),
            _value: Default::default(),
        }
    }
}

impl<T, S: SharedAggregationStrategy> SharedHistogram<T, S> {
    /// Create a new atomic histogram with the given aggregation strategy.
    pub fn new(strategy: S) -> Self {
        Self {
            strategy,
            _value: PhantomData,
        }
    }

    /// Add a value to the histogram through a shared reference.
    ///
    /// The value is converted to observations using the metric value's implementation,
    /// then recorded in the aggregation strategy.
    pub fn add_value(&self, value: T)
    where
        T: MetricValue,
    {
        struct Capturer<'a, S>(&'a S);
        impl<'b, S: SharedAggregationStrategy> ValueWriter for Capturer<'b, S> {
            fn string(self, _value: &str) {}
            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                _unit: metrique_writer::Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                for obs in distribution {
                    match obs {
                        Observation::Unsigned(v) => self.0.record(v as f64),
                        Observation::Floating(v) => self.0.record(v),
                        Observation::Repeated { total, occurrences } => {
                            let avg = total / occurrences as f64;
                            for _ in 0..occurrences {
                                self.0.record(avg);
                            }
                        }
                        _ => {}
                    }
                }
            }
            fn error(self, _error: metrique_writer::ValidationError) {}
        }

        let capturer = Capturer(&self.strategy);
        value.write(capturer);
    }
}

impl<T: MetricValue, S: SharedAggregationStrategy> CloseValue for SharedHistogram<T, S> {
    type Closed = HistogramClosed<T>;

    fn close(self) -> Self::Closed {
        HistogramClosed {
            observations: self.strategy.drain(),
            _value: PhantomData,
        }
    }
}

/// Closed histogram value containing aggregated observations.
///
/// This is the result of closing a histogram and is emitted as a metric distribution.
pub struct HistogramClosed<T> {
    observations: Vec<Observation>,
    _value: PhantomData<T>,
}

impl<T> Value for HistogramClosed<T>
where
    T: MetricValue,
{
    fn write(&self, writer: impl ValueWriter) {
        use metrique_writer::unit::UnitTag;
        writer.metric(
            self.observations.iter().copied(),
            T::Unit::UNIT,
            [],
            MetricFlags::empty(),
        )
    }
}

impl<T> MetricValue for HistogramClosed<T>
where
    T: MetricValue,
{
    type Unit = T::Unit;
}

/// Exponential bucketing strategy using the histogram crate.
///
/// Uses exponential bucketing with configurable precision. Default configuration
/// uses 4-bit mantissa precision (16 buckets per order of magnitude, ~6.25% error).
pub struct ExponentialAggregationStrategy {
    inner: histogram::Histogram,
}

impl ExponentialAggregationStrategy {
    /// Create a new exponential aggregation strategy with default configuration.
    pub fn new() -> Self {
        let config = histogram::Config::new(4, 32).expect("known good configuration");
        Self {
            inner: histogram::Histogram::with_config(&config),
        }
    }
}

impl Default for ExponentialAggregationStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl AggregationStrategy for ExponentialAggregationStrategy {
    fn record(&mut self, value: f64) {
        self.inner.add(value as u64, 1).ok();
    }

    fn drain(&mut self) -> Vec<Observation> {
        let snapshot = std::mem::replace(
            &mut self.inner,
            histogram::Histogram::with_config(&histogram::Config::new(4, 32).unwrap()),
        );
        snapshot
            .iter()
            .filter(|bucket| bucket.count() > 0)
            .map(|bucket| {
                let range = bucket.range();
                let midpoint = range.start() + (range.end() - range.start()) / 2;
                Observation::Repeated {
                    total: midpoint as f64 * bucket.count() as f64,
                    occurrences: bucket.count(),
                }
            })
            .collect()
    }
}

/// Strategy that stores all observations and sorts them on emission.
///
/// This preserves all observations exactly but uses more memory than bucketing strategies.
/// Uses a `SmallVec` to avoid allocations for small numbers of observations.
///
/// The const generic `N` controls the inline capacity before heap allocation.
#[derive(Default)]
pub struct SortAndMerge<const N: usize = 128> {
    values: SmallVec<[f64; N]>,
}

impl<const N: usize> SortAndMerge<N> {
    /// Create a new sort-and-merge strategy.
    pub fn new() -> Self {
        Self {
            values: SmallVec::new(),
        }
    }
}

impl<const N: usize> AggregationStrategy for SortAndMerge<N> {
    fn record(&mut self, value: f64) {
        self.values.push(value);
    }

    fn drain(&mut self) -> Vec<Observation> {
        self.values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut observations = Vec::new();
        let mut iter = self.values.iter().copied();

        if let Some(first) = iter.next() {
            let mut current_value = first;
            let mut current_count = 1;

            for value in iter {
                if value == current_value {
                    current_count += 1;
                } else {
                    observations.push(Observation::Repeated {
                        total: current_value * current_count as f64,
                        occurrences: current_count,
                    });
                    current_value = value;
                    current_count = 1;
                }
            }

            observations.push(Observation::Repeated {
                total: current_value * current_count as f64,
                occurrences: current_count,
            });
        }

        self.values.clear();
        observations
    }
}

/// Thread-safe exponential bucketing strategy using atomic counters.
///
/// Like [`ExponentialAggregationStrategy`] but uses atomic operations to allow concurrent
/// recording from multiple threads.
pub struct AtomicExponentialAggregationStrategy {
    inner: histogram::AtomicHistogram,
}

impl AtomicExponentialAggregationStrategy {
    /// Create a new atomic exponential aggregation strategy with default configuration.
    pub fn new() -> Self {
        let config = histogram::Config::new(4, 32).expect("known good configuration");
        Self {
            inner: histogram::AtomicHistogram::with_config(&config),
        }
    }
}

impl Default for AtomicExponentialAggregationStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedAggregationStrategy for AtomicExponentialAggregationStrategy {
    fn record(&self, value: f64) {
        self.inner.add(value as u64, 1).ok();
    }

    fn drain(&self) -> Vec<Observation> {
        self.inner
            .drain()
            .iter()
            .filter(|bucket| bucket.count() > 0)
            .map(|bucket| {
                let range = bucket.range();
                let midpoint = range.start() + (range.end() - range.start()) / 2;
                Observation::Repeated {
                    total: midpoint as f64 * bucket.count() as f64,
                    occurrences: bucket.count(),
                }
            })
            .collect()
    }
}
