use metrique::unit::Microsecond;
use metrique::unit_of_work::metrics;
use metrique_writer::test_util::test_metric;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(quantize(bits = 8))]
    latency_nanos: u64,

    #[metrics(quantize(bits = 4, rounding = floor))]
    payload_bytes: u64,

    #[metrics(quantize(bits = 4, rounding = ceil))]
    queue_depth: u64,

    #[metrics(unit = Microsecond, quantize(bits = 8, rounding = floor))]
    downstream_time: Duration,

    request_count: u64,
}

#[test]
fn attribute_quantizes_only_annotated_fields() {
    let entry = test_metric(RequestMetrics {
        latency_nanos: 1_234_567,
        payload_bytes: 1000,
        queue_depth: 1000,
        downstream_time: Duration::from_millis(3),
        request_count: 1000,
    });

    assert_eq!(entry.metrics["LatencyNanos"], 1_232_896); // 8 bits, midpoint default
    assert_eq!(entry.metrics["PayloadBytes"], 960); // 4 bits, floor
    assert_eq!(entry.metrics["QueueDepth"], 1024); // 4 bits, ceil
    assert_eq!(entry.metrics["DownstreamTime"], 2992); // 3000us at 8 bits, floor
    assert_eq!(entry.metrics["RequestCount"], 1000); // untouched
}
