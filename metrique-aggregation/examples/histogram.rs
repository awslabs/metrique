use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{SharedHistogram, Histogram, SortAndMerge};
use metrique_writer::unit::{Byte, Millisecond};
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration>,
    #[metrics(unit = Byte)]
    size: Histogram<u32>,

    // for thread safe, use SharedHistogram
    atomics: SharedHistogram<usize>,

    // other strategies are available, e.g. SortAndMerge preserves all
    // data points
    all_values: Histogram<usize, SortAndMerge<64>>,
}

fn main() {
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
    println!("{:?}", entries[0].metrics);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
}
