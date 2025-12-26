use assert2::check;
use metrique::test_util::TestEntrySink;
use metrique::unit::Microsecond;
use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{
    AggregationStrategy, AtomicExponentialAggregationStrategy, ExponentialAggregationStrategy,
    Histogram, SharedAggregationStrategy, SharedHistogram, SortAndMerge,
};
use metrique_writer::Observation;
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{Byte, Millisecond, UnitTag};
use metrique_writer::value::{MetricFlags, MetricValue, ValueWriter, WithDimensions};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rstest::rstest;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, ExponentialAggregationStrategy>,
    #[metrics(unit = Byte)]
    size: Histogram<u32, ExponentialAggregationStrategy>,

    #[metrics(unit = Microsecond)]
    high_precision: Histogram<Duration, SortAndMerge>,
}

#[test]
fn units_correctly_emitted() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut metrics = TestMetrics::default().append_on_drop(sink);
    metrics.high_precision.add_value(Duration::from_secs(1));
    metrics.high_precision.add_value(Duration::from_secs(1));
    drop(metrics);
    let entry = inspector.entries()[0].clone();
    check!(
        entry.metrics["HighPrecision"].distribution
            == vec![Observation::Repeated {
                total: 2_000_000 as f64,
                occurrences: 2
            }]
    );
}

#[test]
fn test_histogram() {
    let sink = test_entry_sink();
    let mut metrics = TestMetrics::default();
    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));
    metrics.latency.add_value(Duration::from_millis(25));
    metrics.latency.add_value(Duration::from_millis(25));

    metrics.size.add_value(512u32);
    metrics.size.add_value(2048u32);
    metrics.size.add_value(2048u32);

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    check!(latency_metric.unit.to_string() == "Milliseconds");

    // Verify distribution values are approximately correct
    let mut total_latency = 0.0;
    let mut count_latency = 0;
    for obs in &latency_metric.distribution {
        match obs {
            metrique_writer::Observation::Repeated { total, occurrences } => {
                total_latency += total;
                count_latency += occurrences;
            }
            _ => panic!("Expected Repeated observations"),
        }
    }
    check!(count_latency == 4);
    let avg_latency = total_latency / count_latency as f64;
    check!(
        (avg_latency - 17.5).abs() < 0.5,
        "Average latency should be ~17.5ms, got {}",
        avg_latency
    );

    let size_metric = &entries[0].metrics["Size"];
    check!(size_metric.unit.to_string() == "Bytes");

    let mut total_size = 0.0;
    let mut count_size = 0;
    for obs in &size_metric.distribution {
        match obs {
            metrique_writer::Observation::Repeated { total, occurrences } => {
                total_size += total;
                count_size += occurrences;
            }
            _ => panic!("Expected Repeated observations"),
        }
    }
    check!(count_size == 3);
    let avg_size = total_size / count_size as f64;
    check!(
        (avg_size - 1536.0).abs() < 50.0,
        "Average size should be ~1536 bytes, got {}",
        avg_size
    );
}

#[test]
fn test_sort_and_merge() {
    use metrique_aggregation::histogram::SortAndMerge;

    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: Histogram<Duration, SortAndMerge>,
    }

    let mut metrics = Metrics {
        latency: Histogram::new(SortAndMerge::new()),
    };

    metrics.latency.add_value(Duration::from_millis(25));
    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    check!(latency_metric.unit.to_string() == "Milliseconds");

    // Verify values are sorted and exact
    let dist = &latency_metric.distribution;
    check!(dist.len() == 3);

    check!(
        dist[0]
            == metrique_writer::Observation::Repeated {
                total: 5.0,
                occurrences: 1
            }
    );
    check!(
        dist[1]
            == metrique_writer::Observation::Repeated {
                total: 15.0,
                occurrences: 1
            }
    );
    check!(
        dist[2]
            == metrique_writer::Observation::Repeated {
                total: 25.0,
                occurrences: 1
            }
    );
}

#[test]
fn test_sort_and_merge_merges_duplicates() {
    use metrique_aggregation::histogram::SortAndMerge;

    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: Histogram<Duration, SortAndMerge>,
    }

    let mut metrics = Metrics {
        latency: Histogram::new(SortAndMerge::new()),
    };

    metrics.latency.add_value(Duration::from_millis(1));
    metrics.latency.add_value(Duration::from_millis(2));
    metrics.latency.add_value(Duration::from_millis(2));
    metrics.latency.add_value(Duration::from_millis(3));
    metrics.latency.add_value(Duration::from_millis(3));
    metrics.latency.add_value(Duration::from_millis(3));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    let dist = &latency_metric.distribution;
    check!(dist.len() == 3);

    check!(
        dist[0]
            == metrique_writer::Observation::Repeated {
                total: 1.0,
                occurrences: 1
            }
    );
    check!(
        dist[1]
            == metrique_writer::Observation::Repeated {
                total: 4.0,
                occurrences: 2
            }
    );
    check!(
        dist[2]
            == metrique_writer::Observation::Repeated {
                total: 9.0,
                occurrences: 3
            }
    );
}

#[test]
fn test_atomic_histogram() {
    use metrique_aggregation::histogram::AtomicExponentialAggregationStrategy;

    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: SharedHistogram<Duration, AtomicExponentialAggregationStrategy>,
    }

    let metrics = Metrics {
        latency: SharedHistogram::new(AtomicExponentialAggregationStrategy::new()),
    };

    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));
    metrics.latency.add_value(Duration::from_millis(25));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    check!(latency_metric.unit.to_string() == "Milliseconds");

    // Verify distribution values are approximately correct
    let mut total = 0.0;
    let mut count = 0;
    for obs in &latency_metric.distribution {
        match obs {
            metrique_writer::Observation::Repeated {
                total: t,
                occurrences,
            } => {
                total += t;
                count += occurrences;
            }
            _ => panic!("Expected Repeated observations"),
        }
    }
    check!(count == 3);
    let avg = total / count as f64;
    check!(
        (avg - 15.0).abs() < 0.5,
        "Average latency should be ~15ms, got {}",
        avg
    );
}

#[test]
fn test_histogram_with_dimensions() {
    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: WithDimensions<Histogram<Duration, ExponentialAggregationStrategy>, 1>,
    }

    let mut metrics = Metrics {
        latency: WithDimensions::new(
            Histogram::new(ExponentialAggregationStrategy::new()),
            "Operation",
            "GetItem",
        ),
    };

    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    check!(latency_metric.unit.to_string() == "Milliseconds");
}

#[test]
fn test_sort_and_merge_with_nan() {
    use metrique_aggregation::histogram::SortAndMerge;

    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: Histogram<f64, SortAndMerge>,
    }

    let mut metrics = Metrics {
        latency: Histogram::new(SortAndMerge::new()),
    };

    metrics.latency.add_value(5.0);
    metrics.latency.add_value(f64::NAN);
    metrics.latency.add_value(15.0);

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    check!(entries.len() == 1);

    let latency_metric = &entries[0].metrics["Latency"];
    let dist = &latency_metric.distribution;

    // NaN values should be filtered out, leaving only valid values
    check!(dist.len() == 2);
    check!(
        dist[0]
            == metrique_writer::Observation::Repeated {
                total: 5.0,
                occurrences: 1
            }
    );
    check!(
        dist[1]
            == metrique_writer::Observation::Repeated {
                total: 15.0,
                occurrences: 1
            }
    );
}

// Test harness for validating histogram accuracy

/// Calculate percentile from a list of values. Percentile should be in the range 0->100
fn calculate_percentile(sorted_values: &[f64], percentile: f64) -> f64 {
    let index = (percentile / 100.0 * (sorted_values.len() - 1) as f64).round() as usize;
    sorted_values[index]
}

/// Calculate percentile from a list of buckets. Percentile should be in the range 0->100
fn calculate_percentile_from_buckets(observations: &[Observation], percentile: f64) -> f64 {
    // Build cumulative distribution from bucketed observations
    let mut buckets: Vec<(f64, u64)> = Vec::new();
    let mut total_count = 0u64;

    for obs in observations {
        match obs {
            Observation::Repeated { total, occurrences } => {
                let value = total / *occurrences as f64;
                buckets.push((value, *occurrences));
                total_count += occurrences;
            }
            Observation::Floating(v) => {
                buckets.push((*v, 1));
                total_count += 1;
            }
            Observation::Unsigned(v) => {
                buckets.push((*v as f64, 1));
                total_count += 1;
            }
            _ => {}
        }
    }

    // Sort by value
    buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Use same formula as ground truth: (percentile / 100.0 * (total_count - 1)).round()
    let target_index = (percentile / 100.0 * (total_count - 1) as f64).round() as u64;
    let mut cumulative = 0u64;

    for &(value, count) in &buckets {
        if cumulative + count > target_index {
            // Target index falls within this bucket
            return value;
        }
        cumulative += count;
    }

    buckets.last().map(|(v, _)| *v).unwrap_or(0.0)
}

fn test_histogram_accuracy<S: AggregationStrategy>(
    mut strategy: S,
    values: Vec<f64>,
    max_error_pct: f64,
) {
    let mut ground_truth = values.clone();
    ground_truth.sort_by(|a, b| a.partial_cmp(b).unwrap());

    for &value in &values {
        strategy.record(value);
    }

    let observations = strategy.drain();

    for percentile in [50.0, 90.0, 95.0, 99.0, 99.9] {
        let actual = calculate_percentile(&ground_truth, percentile);
        let reported_val = calculate_percentile_from_buckets(&observations, percentile);
        let error_pct = ((reported_val - actual).abs() / actual) * 100.0;

        check!(
            error_pct <= max_error_pct,
            "p{}: actual={}, reported={}, error={}% (max={}%)",
            percentile,
            actual,
            reported_val,
            error_pct,
            max_error_pct
        );
    }
}

#[track_caller]
fn check_accuracy(expected: f64, buckets: &[Observation], percentile: f64, error_bound: f64) {
    let actual = calculate_percentile_from_buckets(&buckets, percentile);
    let error_pct = ((actual - expected).abs() / expected) * 100.0;
    check!(
        error_pct <= error_bound,
        "p{percentile}: expected={expected}, actual={actual}, error={error_pct}% (max={error_bound}%)",
    );
}

fn test_shared_histogram_accuracy<S: SharedAggregationStrategy>(
    strategy: S,
    values: Vec<f64>,
    max_error_pct: f64,
) {
    let mut ground_truth = values.clone();
    ground_truth.sort_by(|a, b| a.partial_cmp(b).unwrap());

    for &value in &values {
        strategy.record(value);
    }

    let observations = strategy.drain();

    for percentile in [50.0, 90.0, 95.0, 99.0, 99.9] {
        let actual = calculate_percentile(&ground_truth, percentile);
        let reported_val = calculate_percentile_from_buckets(&observations, percentile);
        let error_pct = ((reported_val - actual).abs() / actual) * 100.0;

        check!(
            error_pct <= max_error_pct,
            "p{}: actual={}, reported={}, error={}% (max={}%)",
            percentile,
            actual,
            reported_val,
            error_pct,
            max_error_pct
        );
    }
}

#[rstest]
#[case::exponential_uniform_1k(1000, 1.0, 1000.0, 7.0)]
#[case::exponential_uniform_10k(10000, 1.0, 10000.0, 7.0)]
#[case::exponential_wide_range(1000, 1.0, 1000000.0, 7.0)]
fn test_exponential_strategy_accuracy(
    #[case] sample_size: usize,
    #[case] min_val: f64,
    #[case] max_val: f64,
    #[case] max_error_pct: f64,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let values: Vec<f64> = (0..sample_size)
        .map(|_| rng.random_range(min_val..=max_val))
        .collect();

    test_histogram_accuracy(ExponentialAggregationStrategy::new(), values, max_error_pct);
}

#[rstest]
#[case::sort_uniform_100(100, 1.0, 1000.0)]
#[case::sort_uniform_1k(1000, 1.0, 10000.0)]
fn test_sort_and_merge_accuracy(
    #[case] sample_size: usize,
    #[case] min_val: f64,
    #[case] max_val: f64,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let values: Vec<f64> = (0..sample_size)
        .map(|_| rng.random_range(min_val..=max_val).floor()) // Use integers to avoid floating point issues
        .collect();

    test_histogram_accuracy(SortAndMerge::<128>::new(), values, 0.0);
}

#[rstest]
#[case::atomic_uniform_1k(1000, 1.0, 1000.0, 7.0)]
#[case::atomic_wide_range(1000, 1.0, 1000000.0, 2.0)]
fn test_atomic_exponential_accuracy(
    #[case] sample_size: usize,
    #[case] min_val: f64,
    #[case] max_val: f64,
    #[case] max_error_pct: f64,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let values: Vec<f64> = (0..sample_size)
        .map(|_| rng.random_range(min_val..=max_val))
        .collect();

    test_shared_histogram_accuracy(
        AtomicExponentialAggregationStrategy::new(),
        values,
        max_error_pct,
    );
}

// Custom metric value that emits zero occurrences
struct ZeroOccurrences;

impl MetricValue for ZeroOccurrences {
    type Unit = Millisecond;
}

impl metrique_writer::value::Value for ZeroOccurrences {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            [Observation::Repeated {
                total: 100.0,
                occurrences: 0,
            }],
            Millisecond::UNIT,
            [],
            MetricFlags::empty(),
        );
    }
}

#[test]
fn test_histogram_ignores_zero_occurrences() {
    let mut histogram: Histogram<ZeroOccurrences, ExponentialAggregationStrategy> =
        Histogram::default();
    histogram.add_value(ZeroOccurrences);
    // Should not panic, just ignore the invalid observation
}

#[test]
fn test_shared_histogram_ignores_zero_occurrences() {
    let histogram: SharedHistogram<ZeroOccurrences, AtomicExponentialAggregationStrategy> =
        SharedHistogram::default();
    histogram.add_value(ZeroOccurrences);
    // Should not panic, just ignore the invalid observation
}

#[test]
fn test_histogram_microsecond_accuracy() {
    let mut histogram = Histogram::<Duration>::new(ExponentialAggregationStrategy::new());
    let mut samples = vec![];
    for _i in 0..100 {
        samples.push(Duration::from_micros(5));
    }
    samples.push(Duration::from_micros(100));
    samples.push(Duration::from_millis(1));
    for v in samples {
        histogram.add_value(v);
    }

    #[metrics]
    struct TestMetrics {
        histogram: Histogram<Duration>,
    }

    let entry = test_metric(TestMetrics { histogram });
    let buckets = &entry.metrics["histogram"].distribution;
    check_accuracy(0.005, buckets, 50.0, 6.25);
    check_accuracy(1.0, buckets, 100.0, 6.25);
}
