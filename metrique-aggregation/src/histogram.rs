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

/// Linear bucketing strategy that groups observations into fixed-width buckets.
///
/// Values are grouped into buckets of equal width. For example, with `bucket_size = 10.0`,
/// values 0-9.99 go in bucket 0, 10-19.99 in bucket 1, etc.
///
/// This strategy uses less memory than storing all observations but loses some precision
/// since values are grouped into buckets.
pub struct LinearAggregationStrategy {
    /// Width of each bucket.
    pub bucket_size: f64,
    /// Total number of buckets.
    pub num_buckets: usize,
    counts: Vec<u64>,
}

impl LinearAggregationStrategy {
    /// Create a new linear aggregation strategy.
    ///
    /// # Arguments
    /// * `bucket_size` - Width of each bucket
    /// * `num_buckets` - Total number of buckets
    pub fn new(bucket_size: f64, num_buckets: usize) -> Self {
        Self {
            bucket_size,
            num_buckets,
            counts: vec![0; num_buckets],
        }
    }
}

impl Default for LinearAggregationStrategy {
    fn default() -> Self {
        Self::new(10.0, 100)
    }
}

impl AggregationStrategy for LinearAggregationStrategy {
    fn record(&mut self, value: f64) {
        let bucket = ((value / self.bucket_size).floor() as usize).min(self.num_buckets - 1);
        self.counts[bucket] += 1;
    }

    fn drain(&mut self) -> Vec<Observation> {
        let observations = self
            .counts
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .map(|(bucket, count)| {
                let bucket_value = bucket as f64 * self.bucket_size;
                Observation::Repeated {
                    total: bucket_value * *count as f64,
                    occurrences: *count,
                }
            })
            .collect();
        self.counts.fill(0);
        observations
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

/// Thread-safe linear bucketing strategy using atomic counters.
///
/// Like [`LinearAggregationStrategy`] but uses atomic operations to allow concurrent
/// recording from multiple threads.
pub struct AtomicLinearAggregationStrategy {
    /// Width of each bucket.
    pub bucket_size: f64,
    /// Total number of buckets.
    pub num_buckets: usize,
    counts: Vec<std::sync::atomic::AtomicU64>,
}

impl AtomicLinearAggregationStrategy {
    /// Create a new atomic linear aggregation strategy.
    ///
    /// # Arguments
    /// * `bucket_size` - Width of each bucket
    /// * `num_buckets` - Total number of buckets
    pub fn new(bucket_size: f64, num_buckets: usize) -> Self {
        Self {
            bucket_size,
            num_buckets,
            counts: (0..num_buckets)
                .map(|_| std::sync::atomic::AtomicU64::new(0))
                .collect(),
        }
    }
}

impl Default for AtomicLinearAggregationStrategy {
    fn default() -> Self {
        Self::new(10.0, 100)
    }
}

impl AtomicAggregationStrategy for AtomicLinearAggregationStrategy {
    fn record(&self, value: f64) {
        let bucket = ((value / self.bucket_size).floor() as usize).min(self.num_buckets - 1);
        self.counts[bucket].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn drain(&self) -> Vec<Observation> {
        self.counts
            .iter()
            .enumerate()
            .filter_map(|(bucket, count)| {
                let c = count.swap(0, std::sync::atomic::Ordering::Relaxed);
                if c > 0 {
                    let bucket_value = bucket as f64 * self.bucket_size;
                    Some(Observation::Repeated {
                        total: bucket_value * c as f64,
                        occurrences: c,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}
