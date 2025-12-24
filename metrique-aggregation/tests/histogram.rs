use assert2::check;
use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{ExponentialAggregationStrategy, Histogram, SharedHistogram};
use metrique_writer::unit::{Byte, Millisecond};
use metrique_writer::value::WithDimensions;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, ExponentialAggregationStrategy>,
    #[metrics(unit = Byte)]
    size: Histogram<u32, ExponentialAggregationStrategy>,
}

#[test]
fn test_histogram() {
    let sink = test_entry_sink();
    let mut metrics = TestMetrics {
        latency: Histogram::new(ExponentialAggregationStrategy::new()),
        size: Histogram::new(ExponentialAggregationStrategy::new()),
    };

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
