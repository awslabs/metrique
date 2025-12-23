use metrique_core::CloseValue;
use metrique_writer::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
use std::marker::PhantomData;

pub trait Bucketer {
    fn bucket(&self, value: f64) -> usize;
    fn num_buckets(&self) -> usize;
    fn distribution(&self, counts: &[u64]) -> Vec<Observation>;
}

pub struct Histogram<T, U, B> {
    bucketer: B,
    counts: Vec<u64>,
    _value: PhantomData<T>,
    _unit: PhantomData<U>,
}

impl<T, U, B: Bucketer> Histogram<T, U, B> {
    pub fn new(bucketer: B) -> Self {
        let num_buckets = bucketer.num_buckets();
        Self {
            bucketer,
            counts: vec![0; num_buckets],
            _value: PhantomData,
            _unit: PhantomData,
        }
    }

    pub fn add_value(&mut self, value: Observation) {
        match value {
            Observation::Unsigned(v) => self.counts[self.bucketer.bucket(v as f64)] += 1,
            Observation::Floating(v) => self.counts[self.bucketer.bucket(v)] += 1,
            Observation::Repeated { total, occurrences } => {
                self.counts[self.bucketer.bucket(total / occurrences as f64)] += occurrences
            }
            _ => {}
        }
    }

    pub fn add_entry(&mut self, value: T)
    where
        T: MetricValue,
    {
        struct Capturer<'a, T, U, B>(&'a mut Histogram<T, U, B>);
        impl<'b, T, U, B: Bucketer> ValueWriter for Capturer<'b, T, U, B> {
            fn string(self, _value: &str) {}
            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                _unit: metrique_writer::Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                for obs in distribution {
                    self.0.add_value(obs);
                }
            }
            fn error(self, _error: metrique_writer::ValidationError) {}
        }

        let capturer = Capturer(self);
        value.write(capturer);
    }
}

impl<T, U: metrique_writer::unit::UnitTag, B: Bucketer> CloseValue for Histogram<T, U, B> {
    type Closed = HistogramClosed<U, B>;

    fn close(self) -> Self::Closed {
        HistogramClosed {
            bucketer: self.bucketer,
            counts: self.counts,
            _unit: PhantomData,
        }
    }
}

pub struct HistogramClosed<U, B> {
    bucketer: B,
    counts: Vec<u64>,
    _unit: PhantomData<U>,
}

impl<U: metrique_writer::unit::UnitTag, B: Bucketer> Value for HistogramClosed<U, B> {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            self.bucketer.distribution(&self.counts),
            U::UNIT,
            [],
            MetricFlags::empty(),
        )
    }
}

pub struct LinearBucketer {
    pub bucket_size: f64,
    pub num_buckets: usize,
}

impl Bucketer for LinearBucketer {
    fn bucket(&self, value: f64) -> usize {
        ((value / self.bucket_size).floor() as usize).min(self.num_buckets - 1)
    }

    fn num_buckets(&self) -> usize {
        self.num_buckets
    }

    fn distribution(&self, counts: &[u64]) -> Vec<Observation> {
        counts
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
            .collect()
    }
}
