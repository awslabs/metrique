// SPDX-License-Identifier: Apache-2.0

//! Cross-crate coherence regression test for entry-mode `#[aggregate]`.
//!
//! Before the fix, having two entry-mode `#[aggregate]` structs in different crates
//! in the same build graph caused E0119 (conflicting Merge impls). The root cause was
//! that the Merge impl was emitted against the projection `<T as CloseValue>::Closed`
//! rather than the concrete entry struct name.
//!
//! This test defines its own entry-mode aggregate AND uses one from `aggregate_dep`,
//! proving both can coexist without coherence errors.

use aggregate_dep::DepMetrics;
use metrique::timers::Timer;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_writer::test_util::test_metric;
use std::time::Duration;

/// A local entry-mode aggregate. Having this alongside `DepMetrics` (from a
/// different crate) must NOT trigger E0119.
#[aggregate]
#[metrics]
struct LocalMetrics {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Timer,
}

/// Parent struct that flattens both aggregates to prove they work together.
#[metrics]
struct CombinedMetrics {
    #[metrics(flatten, prefix = "local_")]
    local_agg: Aggregate<LocalMetrics>,
    #[metrics(flatten, prefix = "dep_")]
    dep_agg: Aggregate<DepMetrics>,
}

#[test]
fn cross_crate_entry_mode_aggregates_coexist() {
    let mut metrics = CombinedMetrics {
        local_agg: Aggregate::default(),
        dep_agg: Aggregate::default(),
    };

    // Insert into local aggregate
    let mut t1 = LocalMetrics {
        latency: Timer::start_now(),
    };
    t1.latency.stop();
    metrics.local_agg.insert(t1);

    let mut t2 = LocalMetrics {
        latency: Timer::start_now(),
    };
    t2.latency.stop();
    metrics.local_agg.insert(t2);

    // Insert into dep aggregate
    let mut d1 = DepMetrics {
        latency: Timer::start_now(),
    };
    d1.latency.stop();
    metrics.dep_agg.insert(d1);

    let mut d2 = DepMetrics {
        latency: Timer::start_now(),
    };
    d2.latency.stop();
    metrics.dep_agg.insert(d2);

    // Verify both aggregate correctly when emitted together
    let entry = test_metric(metrics);
    // Each aggregate should have 2 observations
    assert_eq!(entry.metrics["local_latency"].num_observations(), 2);
    assert_eq!(entry.metrics["dep_latency"].num_observations(), 2);
}

#[test]
fn cross_crate_aggregates_independently() {
    // Verify local aggregate works on its own
    let mut local_parent = LocalParent {
        agg: Aggregate::default(),
    };
    let mut t = LocalMetrics {
        latency: Timer::start_now(),
    };
    t.latency.stop();
    local_parent.agg.insert(t);
    let entry = test_metric(local_parent);
    assert_eq!(entry.metrics["latency"].num_observations(), 1);

    // Verify dep aggregate works on its own
    let mut dep_parent = DepParent {
        agg: Aggregate::default(),
    };
    let mut d = DepMetrics {
        latency: Timer::start_now(),
    };
    d.latency.stop();
    dep_parent.agg.insert(d);
    let entry = test_metric(dep_parent);
    assert_eq!(entry.metrics["latency"].num_observations(), 1);
}

#[metrics]
struct LocalParent {
    #[metrics(flatten)]
    agg: Aggregate<LocalMetrics>,
}

#[metrics]
struct DepParent {
    #[metrics(flatten)]
    agg: Aggregate<DepMetrics>,
}
