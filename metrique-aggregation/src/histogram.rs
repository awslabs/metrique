use metrique_core::CloseValue;
use metrique_writer::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
use smallvec::SmallVec;
use std::marker::PhantomData;

pub trait AggregationStrategy {
    fn add_value(&mut self, value: f64);
    fn drain(&mut self) -> Vec<Observation>;
}

pub struct Histogram<T, S> {
    strategy: S,
    _value: PhantomData<T>,
}

impl<T, S: AggregationStrategy> Histogram<T, S> {
    pub fn new(strategy: S) -> Self {
        Self {
            strategy,
            _value: PhantomData,
        }
    }

    pub fn add_value(&mut self, value: f64) {
        self.strategy.add_value(value);
    }

    pub fn add_entry(&mut self, value: T)
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
                        Observation::Unsigned(v) => self.0.add_value(v as f64),
                        Observation::Floating(v) => self.0.add_value(v),
                        Observation::Repeated { total, occurrences } => {
                            let avg = total / occurrences as f64;
                            for _ in 0..occurrences {
                                self.0.add_value(avg);
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

pub struct LinearAggregationStrategy {
    pub bucket_size: f64,
    pub num_buckets: usize,
    counts: Vec<u64>,
}

impl LinearAggregationStrategy {
    pub fn new(bucket_size: f64, num_buckets: usize) -> Self {
        Self {
            bucket_size,
            num_buckets,
            counts: vec![0; num_buckets],
        }
    }
}

impl AggregationStrategy for LinearAggregationStrategy {
    fn add_value(&mut self, value: f64) {
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

pub struct SortAndMerge<const N: usize = 128> {
    values: SmallVec<[f64; N]>,
}

impl<const N: usize> SortAndMerge<N> {
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
    fn add_value(&mut self, value: f64) {
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
