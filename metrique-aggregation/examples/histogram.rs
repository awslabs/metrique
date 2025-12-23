use metrique::{test_util::test_entry_sink, unit_of_work::metrics};
use metrique_aggregation::histogram::{Histogram, LinearBucketer};
use metrique_writer::unit::{Byte, Millisecond};

#[metrics(rename_all = "PascalCase")]
struct TestMetrics {
    latency: Histogram<u32, Millisecond, LinearBucketer>,
    size: Histogram<u32, Byte, LinearBucketer>,
}

fn main() {
    let sink = test_entry_sink();
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

    metrics.latency.add_entry(5u32);
    metrics.latency.add_entry(15u32);
    metrics.latency.add_entry(25u32);
    metrics.latency.add_entry(25u32);

    metrics.size.add_entry(512u32);
    metrics.size.add_entry(2048u32);
    metrics.size.add_entry(2048u32);

    metrics.append_on_drop(sink.sink);

    let entries = sink.inspector.entries();
    println!("{:?}", entries[0].metrics);

    let latency_metric = &entries[0].metrics["Latency"];
    assert_eq!(latency_metric.unit.to_string(), "Milliseconds");

    let size_metric = &entries[0].metrics["Size"];
    assert_eq!(size_metric.unit.to_string(), "Bytes");
}
