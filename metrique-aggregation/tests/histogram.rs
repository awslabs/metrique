use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{Histogram, LinearAggregationStrategy};
use metrique_writer::unit::{Byte, Millisecond};
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

    metrics.latency.add_entry(Duration::from_millis(5));
    metrics.latency.add_entry(Duration::from_millis(15));
    metrics.latency.add_entry(Duration::from_millis(25));
    metrics.latency.add_entry(Duration::from_millis(25));

    metrics.size.add_entry(512u32);
    metrics.size.add_entry(2048u32);
    metrics.size.add_entry(2048u32);

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    assert_eq!(entries.len(), 1);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
}
