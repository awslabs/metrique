//! Test demonstrating SplitSink pattern
//!
//! This test is currently disabled because it requires MergeRef implementations
//! from the #[aggregate] macro, which haven't been generated yet.

#![cfg(feature = "never_enabled")]

use assert2::check;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::keyed_sink::KeyedAggregator;
use metrique_aggregation::split_sink::SplitSink;
use metrique_aggregation::traits::{AggregateSink, FlushableSink};
use metrique_writer::test_util::test_entry_sink;
use std::time::Duration;

#[aggregate]
#[metrics]
pub struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: Duration,
}

/// This test demonstrates the SplitSink pattern - aggregating the same input
/// across multiple sinks using AggregateSinkRef to avoid cloning.
///
/// Currently ignored because the #[aggregate] macro doesn't generate MergeRef impls yet.
#[test]
#[ignore = "Requires MergeRef impl from #[aggregate] macro"]
fn test_split_sink() {
    // Create two output sinks - both will receive aggregated entries
    let aggregated_sink_a = test_entry_sink();
    let aggregated_sink_b = test_entry_sink();

    // Create two aggregators
    let aggregator_a = KeyedAggregator::<ApiCall, _>::new(aggregated_sink_a.sink);
    let aggregator_b = KeyedAggregator::<ApiCall, _>::new(aggregated_sink_b.sink);

    // Combine them with SplitSink - both will aggregate the same data
    let split = SplitSink::new(aggregator_a, aggregator_b);

    // Send some entries
    split.add(
        ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(10),
        }
        .close(),
    );
    split.add(
        ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(20),
        }
        .close(),
    );

    // Flush both sinks
    split.flush();

    // Both sinks should have identical aggregated data
    use metrique_writer::Observation;

    let entries_a = aggregated_sink_a.inspector.entries();
    check!(entries_a.len() == 1);
    check!(
        entries_a[0].metrics["latency"].distribution
            == [
                Observation::Repeated {
                    total: 10.0,
                    occurrences: 1
                },
                Observation::Repeated {
                    total: 20.0,
                    occurrences: 1
                }
            ]
    );

    let entries_b = aggregated_sink_b.inspector.entries();
    check!(entries_b.len() == 1);
    check!(
        entries_b[0].metrics["latency"].distribution
            == [
                Observation::Repeated {
                    total: 10.0,
                    occurrences: 1
                },
                Observation::Repeated {
                    total: 20.0,
                    occurrences: 1
                }
            ]
    );
}
