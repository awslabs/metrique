// SPDX-License-Identifier: Apache-2.0

//! Test that `#[metrics(entry_name = CustomName)]` works correctly.

use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_writer::test_util::test_metric;
use std::time::Duration;

/// Test struct with a custom entry name.
#[metrics(entry_name = MyCustomEntry)]
struct CustomNamedMetrics {
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

/// Verify the generated entry struct has the custom name.
fn _assert_custom_entry_exists(_e: MyCustomEntry) {}

/// Entry-mode aggregate with custom entry name.
#[aggregate]
#[metrics(entry_name = MyAggEntry)]
struct AggWithCustomEntry {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

/// Verify the aggregate entry struct has the custom name.
fn _assert_agg_entry_exists(_e: MyAggEntry) {}

#[metrics]
struct ParentMetrics {
    #[metrics(flatten)]
    agg: Aggregate<AggWithCustomEntry>,
}

#[test]
fn custom_entry_name_works() {
    // Verify custom-named entry struct works with test_metric
    let m = CustomNamedMetrics {
        latency: Duration::from_millis(42),
    };
    let entry = test_metric(m);
    assert_eq!(entry.metrics["latency"].num_observations(), 1);
}

#[test]
fn custom_entry_name_aggregate_works() {
    let mut parent = ParentMetrics {
        agg: Aggregate::default(),
    };
    parent.agg.insert(AggWithCustomEntry {
        latency: Duration::from_millis(100),
    });
    parent.agg.insert(AggWithCustomEntry {
        latency: Duration::from_millis(200),
    });
    let entry = test_metric(parent);
    assert_eq!(entry.metrics["latency"].num_observations(), 2);
}
