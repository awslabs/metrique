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
pub trait AtomicAggregationStrategy {
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
/// Requires `&mut self` to add values. For thread-safe access, use [`AtomicHistogram`].
pub struct Histogram<T, S> {
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

impl<T, S: AggregationStrategy> CloseValue for Histogram<T, S> {
    type Closed = HistogramClosed;

    fn close(mut self) -> Self::Closed {
        HistogramClosed {
            observations: self.strategy.drain(),
        }
    }
}

/// Thread-safe histogram that collects multiple observations and emits them as a distribution.
///
/// Like [`Histogram`] but allows adding values through a shared reference, making it
/// suitable for concurrent access patterns.
pub struct AtomicHistogram<T, S> {
    strategy: S,
    _value: PhantomData<T>,
}

impl<T, S: AtomicAggregationStrategy> AtomicHistogram<T, S> {
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
        impl<'b, S: AtomicAggregationStrategy> ValueWriter for Capturer<'b, S> {
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

impl<T, S: AtomicAggregationStrategy> CloseValue for AtomicHistogram<T, S> {
    type Closed = HistogramClosed;

    fn close(self) -> Self::Closed {
        HistogramClosed {
            observations: self.strategy.drain(),
        }
    }
}

/// Closed histogram value containing aggregated observations.
///
/// This is the result of closing a histogram and is emitted as a metric distribution.
pub struct HistogramClosed {
    observations: Vec<Observation>,
}

impl Value for HistogramClosed {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            self.observations.clone(),
            metrique_writer::Unit::None,
            [],
            MetricFlags::empty(),
        )
    }
}

impl MetricValue for HistogramClosed {
    type Unit = metrique_writer::unit::None;
}

/// Exponential bucketing strategy using the histogram crate.
///
/// Uses exponential bucketing with configurable precision. Default configuration
/// uses 4-bit mantissa precision (16 buckets per order of magnitude, ~6.25% error).
pub struct ExponentialAggregationStrategy {
    inner: histogram::Histogram,
}

/// Sparse exponential bucketing strategy using a HashMap.
///
/// Like [`ExponentialAggregationStrategy`] but stores buckets in a HashMap instead of
/// allocating a full vector. More memory efficient for sparse data.
pub struct SparseExponentialAggregationStrategy {
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

impl SparseExponentialAggregationStrategy {
    /// Create a new sparse exponential aggregation strategy with default configuration.
    pub fn new() -> Self {
        let config = histogram::Config::new(4, 32).expect("known good configuration");
        Self {
            inner: histogram::Histogram::with_config(&config),
        }
    }
}

impl Default for SparseExponentialAggregationStrategy {
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

impl AggregationStrategy for SparseExponentialAggregationStrategy {
    fn record(&mut self, value: f64) {
        self.inner.add(value as u64, 1).ok();
    }

    fn drain(&mut self) -> Vec<Observation> {
        let sparse = histogram::SparseHistogram::from(&self.inner);
        let config = histogram::Config::new(4, 32).unwrap();
        self.inner = histogram::Histogram::with_config(&config);
        
        sparse
            .into_iter()
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

impl<const N: usize> Default for SortAndMerge<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> AggregationStrategy for SortAndMerge<N> {
    fn record(&mut self, value: f64) {
        self.values.push(value);
    }

    fn drain(&mut self) -> Vec<Observation> {
        self.values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let observations = self
            .values
            .iter()
            .copied()
            .map(Observation::Floating)
            .collect();
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

impl AtomicAggregationStrategy for AtomicExponentialAggregationStrategy {
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
