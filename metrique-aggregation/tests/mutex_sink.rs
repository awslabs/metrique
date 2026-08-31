// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use assert2::check;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::MutexSink;
use metrique_aggregation::traits::{FlushableSink, RootSink};
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

fn call(endpoint: &str, latency_ms: u64) -> ApiCallEntry {
    ApiCall {
        endpoint: endpoint.to_string(),
        latency: Duration::from_millis(latency_ms),
    }
    .close()
}

/// A `KeyedAggregator` accumulates on `merge` and only writes to its inner sink when
/// flushed, so a `MutexSink` wrapping one has to be flushable for the entries to reach
/// the sink at all.
#[test]
fn mutex_sink_flush_emits_aggregated_entries() {
    let test_sink = test_entry_sink();
    let aggregator: KeyedAggregator<ApiCall> = KeyedAggregator::new(test_sink.sink);
    let mut sink = MutexSink::new(aggregator);

    sink.merge(call("api1", 10));
    sink.merge(call("api1", 20));
    sink.merge(call("api2", 30));

    // Nothing is emitted until the flush.
    check!(test_sink.inspector.entries().is_empty());

    sink.flush();

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 2, "one entry per key");

    let endpoints: std::collections::BTreeSet<_> = entries
        .iter()
        .map(|e| e.values["endpoint"].clone())
        .collect();
    check!(endpoints == ["api1".to_string(), "api2".to_string()].into());
}

/// Flushing through a clone works, since `MutexSink` is an `Arc` handle.
#[test]
fn mutex_sink_flush_through_a_clone() {
    let test_sink = test_entry_sink();
    let aggregator: KeyedAggregator<ApiCall> = KeyedAggregator::new(test_sink.sink);
    let sink = MutexSink::new(aggregator);

    sink.merge(call("api1", 10));

    let mut other = sink.clone();
    other.flush();

    check!(test_sink.inspector.entries().len() == 1);
}
