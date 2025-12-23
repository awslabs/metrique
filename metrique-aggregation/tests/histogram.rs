use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{AtomicHistogram, Histogram, LinearAggregationStrategy};
use metrique_writer::unit::{Byte, Millisecond};
use metrique_writer::value::WithDimensions;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, LinearAggregationStrategy>,
    #[metrics(unit = Byte)]
    size: Histogram<u32, LinearAggregationStrategy>,
}

#[test]
fn test_histogram() {
    let sink = test_entry_sink();
    let mut metrics = TestMetrics {
        latency: Histogram::new(LinearAggregationStrategy::new(10.0, 10)),
        size: Histogram::new(LinearAggregationStrategy::new(1024.0, 5)),
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
    assert_eq!(entries.len(), 1);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
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
    assert_eq!(entries.len(), 1);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    // Verify values are sorted
    let dist = &latency_metric.distribution;
    assert_eq!(dist.len(), 3);
}

#[test]
fn test_atomic_histogram() {
    use metrique_aggregation::histogram::AtomicLinearAggregationStrategy;

    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: AtomicHistogram<Duration, AtomicLinearAggregationStrategy>,
    }

    let metrics = Metrics {
        latency: AtomicHistogram::new(AtomicLinearAggregationStrategy::new(10.0, 10)),
    };

    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));
    metrics.latency.add_value(Duration::from_millis(25));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    assert_eq!(entries.len(), 1);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");
}

#[test]
fn test_histogram_with_dimensions() {
    let sink = test_entry_sink();

    #[metrics(rename_all = "PascalCase")]
    struct Metrics {
        #[metrics(unit = Millisecond)]
        latency: WithDimensions<Histogram<Duration, LinearAggregationStrategy>, 1>,
    }

    let mut metrics = Metrics {
        latency: WithDimensions::new(
            Histogram::new(LinearAggregationStrategy::new(10.0, 10)),
            "Operation",
            "GetItem",
        ),
    };

    metrics.latency.add_value(Duration::from_millis(5));
    metrics.latency.add_value(Duration::from_millis(15));

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    assert_eq!(entries.len(), 1);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");
}
