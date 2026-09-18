use std::time::Duration;

use metrique::{CloseValue, unit_of_work::metrics};
use metrique_aggregation::{
    aggregate,
    aggregator::KeyedAggregator,
    histogram::{Histogram, SortAndMerge},
    sink::{RollingThresholdSink, TeeSink, non_aggregate},
    traits::{AggregateSink, FlushableSink},
};
use metrique_writer::test_util::test_entry_sink;

#[aggregate(ref)]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(ignore)]
    request_id: u64,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: Duration,
}

impl ApiCallEntry {
    #[allow(deprecated)]
    fn latency_millis(&self) -> u64 {
        u64::try_from(self.latency.as_millis()).unwrap_or(u64::MAX)
    }
}

#[test]
fn aggregates_every_entry_and_approximately_emits_the_next_windows_tail() {
    let aggregated = test_entry_sink();
    let raw = test_entry_sink();

    let aggregator = KeyedAggregator::<ApiCall>::new(aggregated.sink);
    let tail = RollingThresholdSink::new(10, ApiCallEntry::latency_millis, non_aggregate(raw.sink));
    let mut sink = TeeSink::new(aggregator, tail);

    for window in 0..2 {
        for latency_ms in 1..=100 {
            sink.merge(
                ApiCall {
                    endpoint: "GetItem".to_string(),
                    request_id: window * 100 + latency_ms,
                    latency: Duration::from_millis(latency_ms),
                }
                .close(),
            );
        }
        sink.flush();
    }

    let aggregated_entries = aggregated.inspector.entries();
    assert_eq!(aggregated_entries.len(), 2);
    assert!(
        aggregated_entries
            .iter()
            .all(|entry| entry.metrics["latency"].distribution.len() == 100)
    );

    let raw_entries = raw.inspector.entries();
    assert!((10..=16).contains(&raw_entries.len()));
    assert!(
        raw_entries
            .iter()
            .all(|entry| entry.metrics["latency"].as_f64() >= 80.0)
    );
}
