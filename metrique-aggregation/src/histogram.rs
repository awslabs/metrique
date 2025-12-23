use metrique_core::CloseValue;
use metrique_writer::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
use std::marker::PhantomData;

pub trait AggregationStrategy {
    fn add_value(&mut self, value: f64);
    fn drain(&mut self) -> Vec<Observation>;
}

pub struct Histogram<T, U, B> {
    bucketer: B,
    _value: PhantomData<T>,
    _unit: PhantomData<U>,
}

impl<T, U, B: AggregationStrategy> Histogram<T, U, B> {
    pub fn new(bucketer: B) -> Self {
        Self {
            bucketer,
            _value: PhantomData,
            _unit: PhantomData,
        }
    }

    pub fn add_value(&mut self, value: f64) {
        self.bucketer.add_value(value);
    }

    pub fn add_entry(&mut self, value: T)
    where
        T: MetricValue,
    {
        struct Capturer<'a, B>(&'a mut B);
        impl<'b, B: AggregationStrategy> ValueWriter for Capturer<'b, B> {
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

        let capturer = Capturer(&mut self.bucketer);
        value.write(capturer);
    }
}

impl<T, U: metrique_writer::unit::UnitTag, B: AggregationStrategy> CloseValue for Histogram<T, U, B> {
    type Closed = HistogramClosed<U>;

    fn close(mut self) -> Self::Closed {
        HistogramClosed {
            observations: self.bucketer.drain(),
            _unit: PhantomData,
        }
    }
}

pub struct HistogramClosed<U> {
    observations: Vec<Observation>,
    _unit: PhantomData<U>,
}

impl<U: metrique_writer::unit::UnitTag> Value for HistogramClosed<U> {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(self.observations.clone(), U::UNIT, [], MetricFlags::empty())
    }
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
