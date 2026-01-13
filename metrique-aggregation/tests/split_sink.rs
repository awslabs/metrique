//! Test demonstrating SplitSink pattern

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{EntrySinkAsAggregateSink, SplitSink};
use metrique_aggregation::traits::{AggregateStrategy, Key};
use metrique_aggregation::{KeyedAggregator, WorkerSink};
use metrique_writer::test_util::test_entry_sink;
use std::borrow::Cow;
use std::time::Duration;

#[aggregate(ref)]
#[metrics]
pub struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: Duration,
}

/// Custom strategy that groups by endpoint and duration threshold
/// Reuses the Merge impl from ApiCall, only changes the Key
struct ByEndpointAndThreshold;

#[derive(Clone, Hash, PartialEq, Eq)]
#[metrics]
struct ThresholdKey<'a> {
    endpoint: Cow<'a, String>,
    over_1s: bool,
}

struct ThresholdKeyExtractor;

impl Key<ApiCallEntry> for ThresholdKeyExtractor {
    type Key<'a> = ThresholdKey<'a>;

    fn from_source(source: &ApiCallEntry) -> Self::Key<'_> {
        #[allow(deprecated)]
        ThresholdKey {
            endpoint: Cow::Borrowed(&source.endpoint),
            over_1s: source.latency >= Duration::from_secs(1),
        }
    }

    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
        ThresholdKey {
            endpoint: Cow::Owned(key.endpoint.clone().into_owned()),
            over_1s: key.over_1s,
        }
    }

    fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool {
        owned == borrowed
    }
}

impl AggregateStrategy for ByEndpointAndThreshold {
    type Source = ApiCallEntry;
    type Key = ThresholdKeyExtractor;
}

/// This test demonstrates the SplitSink pattern - aggregating the same input
/// across multiple sinks using MergeRef to avoid cloning.
///
/// Sink A: Groups by endpoint, tracks latency histogram
/// Sink B: Groups by endpoint + duration threshold (< 1s vs >= 1s), counts requests
#[tokio::test]
async fn test_split_sink() {
    // Create two output sinks
    let aggregated_sink_a = test_entry_sink();
    let aggregated_sink_b = test_entry_sink();
    let raw_sink = test_entry_sink();

    // Aggregator A: standard ApiCall aggregation (by endpoint, histogram)
    let aggregator_a = KeyedAggregator::<ApiCall>::new(aggregated_sink_a.sink);

    // Aggregator B: custom strategy (by endpoint + threshold, same histogram)
    let aggregator_b = KeyedAggregator::<ByEndpointAndThreshold>::new(aggregated_sink_b.sink);

    // Combine them with SplitSink
    let split = SplitSink::new(
        aggregator_a,
        SplitSink::new(aggregator_b, EntrySinkAsAggregateSink::new(raw_sink.sink)),
    );
    let sink = WorkerSink::new(split, Duration::from_secs(10));

    ApiCall {
        endpoint: "api1".to_string(),
        latency: Duration::from_millis(500),
    }
    .close_and_merge(sink.clone());

    ApiCall {
        endpoint: "api1".to_string(),
        latency: Duration::from_millis(1500),
    }
    .close_and_merge(sink.clone());

    ApiCall {
        endpoint: "api1".to_string(),
        latency: Duration::from_millis(800),
    }
    .close_and_merge(sink.clone());

    ApiCall {
        endpoint: "api2".to_string(),
        latency: Duration::from_millis(2000),
    }
    .close_and_merge(sink.clone());

    // Flush both sinks
    sink.flush().await;

    // Sink A: grouped by endpoint only
    let entries_a = aggregated_sink_a.inspector.entries();
    check!(entries_a.len() == 2); // api1 and api2

    let api1_entry = entries_a
        .iter()
        .find(|e| e.values["endpoint"] == "api1")
        .unwrap();
    check!(api1_entry.metrics["latency"].distribution.len() == 3);

    let api2_entry = entries_a
        .iter()
        .find(|e| e.values["endpoint"] == "api2")
        .unwrap();
    check!(api2_entry.metrics["latency"].distribution.len() == 1);

    // Sink B: grouped by endpoint + threshold
    let entries_b = aggregated_sink_b.inspector.entries();
    check!(entries_b.len() == 3); // api1 under, api1 over, api2 over

    let api1_under = entries_b
        .iter()
        .find(|e| e.values["endpoint"] == "api1" && e.metrics["over_1s"] == false)
        .unwrap();
    check!(api1_under.metrics["latency"].distribution.len() == 2); // 500ms and 800ms

    let api1_over = entries_b
        .iter()
        .find(|e| e.values["endpoint"] == "api1" && e.metrics["over_1s"] == true)
        .unwrap();
    check!(api1_over.metrics["latency"].distribution.len() == 1); // 1500ms

    let api2_over = entries_b
        .iter()
        .find(|e| e.values["endpoint"] == "api2" && e.metrics["over_1s"] == true)
        .unwrap();
    check!(api2_over.metrics["latency"].distribution.len() == 1); // 2000ms

    // Check raw sink
    check!(raw_sink.inspector.entries().len() == 4);
}
