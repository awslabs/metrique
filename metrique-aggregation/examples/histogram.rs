use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{Histogram, ExponentialAggregationStrategy};
use metrique_writer::unit::{Byte, Millisecond};
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, ExponentialAggregationStrategy>,
    #[metrics(unit = Byte)]
    size: Histogram<u32, ExponentialAggregationStrategy>,
}

fn main() {
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
    println!("{:?}", entries[0].metrics);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
}
