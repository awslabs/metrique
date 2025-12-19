use metrique::{
    CloseValue,
    test_util::{TestEntrySink, test_entry_sink},
    unit_of_work::metrics,
};
use metrique_writer::{
    MetricFlags, MetricValue, Observation, Value, ValueWriter,
    unit::{Byte, Millisecond, UnitTag},
};

trait Bucketer {
    fn bucket(&self, value: f64) -> usize;
    fn num_buckets(&self) -> usize;
    fn distribution(&self, counts: &[u64]) -> Vec<Observation>;
}

struct Histogram<T, U, B> {
    bucketer: B,
    counts: Vec<u64>,
    _value: std::marker::PhantomData<T>,
    _unit: std::marker::PhantomData<U>,
}

impl<T, U, B: Bucketer> Histogram<T, U, B> {
    fn new(bucketer: B) -> Self {
        let num_buckets = bucketer.num_buckets();
        Self {
            bucketer,
            counts: vec![0; num_buckets],
            _value: std::marker::PhantomData,
            _unit: std::marker::PhantomData,
        }
    }

    fn add_value(&mut self, value: Observation) {
        match value {
            Observation::Unsigned(v) => self.counts[self.bucketer.bucket(v as f64)] += 1,
            Observation::Floating(v) => self.counts[self.bucketer.bucket(v)] += 1,
            Observation::Repeated { total, occurrences } => {
                self.counts[self.bucketer.bucket(total / occurrences as f64)] += occurrences
            }
            _ => {}
        }
    }

    fn add_entry(&mut self, value: T)
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

impl<T, U: UnitTag, B: Bucketer> CloseValue for Histogram<T, U, B> {
    type Closed = HistogramClosed<U, B>;

    fn close(self) -> Self::Closed {
        HistogramClosed {
            bucketer: self.bucketer,
            counts: self.counts,
            _unit: std::marker::PhantomData,
        }
    }
}

struct HistogramClosed<U, B> {
    bucketer: B,
    counts: Vec<u64>,
    _unit: std::marker::PhantomData<U>,
}

impl<U: UnitTag, B: Bucketer> Value for HistogramClosed<U, B> {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            self.bucketer.distribution(&self.counts),
            U::UNIT,
            [],
            MetricFlags::empty(),
        )
    }
}

// Example bucketer: linear buckets
// this is not great, use an H2 histogram
struct LinearBucketer {
    bucket_size: f64,
    num_buckets: usize,
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

#[metrics(rename_all = "PascalCase")]
struct TestMetrics {
    latency: Histogram<u32, Millisecond, LinearBucketer>,
    size: Histogram<u32, Byte, LinearBucketer>,
}

fn main() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut metrics = TestMetrics {
        latency: Histogram::new(LinearBucketer {
            bucket_size: 10.0,
            num_buckets: 10,
        }),
        size: Histogram::new(LinearBucketer {
            bucket_size: 1024.0,
            num_buckets: 5,
        }),
    };

    // Add entries - now just pass the value
    metrics.latency.add_entry(5u32);
    metrics.latency.add_entry(15u32);
    metrics.latency.add_entry(25u32);
    metrics.latency.add_entry(25u32);

    metrics.size.add_entry(512u32);
    metrics.size.add_entry(2048u32);
    metrics.size.add_entry(2048u32);

    metrics.append_on_drop(sink);

    let entries = inspector.entries();
    println!("{:?}", entries[0].metrics);

    // Verify the histogram emitted correctly
    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
}
