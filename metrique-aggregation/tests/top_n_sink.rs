use metrique::{CloseValue, unit_of_work::metrics};
use metrique_aggregation::{
    aggregate,
    aggregator::KeyedAggregator,
    histogram::{Histogram, SortAndMerge},
    sink::{TeeSink, TopNSink, non_aggregate},
    traits::{AggregateSink, FlushableSink},
};
use metrique_writer::test_util::test_entry_sink;
use std::time::Duration;

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
    fn latency(&self) -> Duration {
        #[allow(deprecated)]
        self.latency
    }
}

#[test]
fn aggregates_every_entry_and_emits_only_the_worst_n_raw_entries() {
    let aggregated = test_entry_sink();
    let raw = test_entry_sink();

    let aggregator = KeyedAggregator::<ApiCall>::new(aggregated.sink);
    let worst = TopNSink::new(2, ApiCallEntry::latency, non_aggregate(raw.sink));
    let mut sink = TeeSink::new(aggregator, worst);

    for (request_id, latency_ms) in [(1, 10), (2, 100), (3, 30), (4, 80)] {
        sink.merge(
            ApiCall {
                endpoint: "GetItem".to_string(),
                request_id,
                latency: Duration::from_millis(latency_ms),
            }
            .close(),
        );
    }
    sink.flush();

    let aggregated_entries = aggregated.inspector.entries();
    assert_eq!(aggregated_entries.len(), 1);
    assert_eq!(
        aggregated_entries[0].metrics["latency"].distribution.len(),
        4
    );

    let mut raw_request_ids = raw
        .inspector
        .entries()
        .iter()
        .map(|entry| entry.metrics["request_id"].as_u64())
        .collect::<Vec<_>>();
    raw_request_ids.sort_unstable();
    assert_eq!(raw_request_ids, [2, 4]);

    sink.merge(
        ApiCall {
            endpoint: "GetItem".to_string(),
            request_id: 5,
            latency: Duration::from_millis(5),
        }
        .close(),
    );
    sink.flush();

    let raw_entries = raw.inspector.entries();
    assert_eq!(raw_entries.len(), 3);
    assert_eq!(raw_entries[2].metrics["request_id"], 5);
}
