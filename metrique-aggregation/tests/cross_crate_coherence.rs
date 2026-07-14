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

use aggregate_dep::{DepMetrics, DepRefMetrics};
use metrique::timers::Timer;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::{Aggregate, KeyedAggregator};
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::traits::{AggregateSinkRef, FlushableSink};
use metrique_writer::test_util::{test_entry_sink, test_metric};
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

// --- `#[aggregate(ref)]` / `MergeRef` cross-crate coherence -----------------------------

/// A local `#[aggregate(ref)]` aggregate. Its generated `MergeRef` impl, alongside
/// `DepRefMetrics`'s `MergeRef` impl (from another crate), must NOT trigger E0119.
///
/// Before the fix `MergeRef` (like `Merge`) was emitted against the projection
/// `<T as CloseValue>::Closed`, so two `#[aggregate(ref)]` structs in different crates
/// failed to compile. This guards the `ref` half of the fix.
#[aggregate(ref)]
#[metrics]
struct LocalRefMetrics {
    #[aggregate(key)]
    endpoint: String,
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

#[test]
fn cross_crate_aggregate_ref_merge_ref_coexists() {
    use metrique::CloseValue;

    // Exercise the local ref aggregate via the `merge_ref` path (AggregateSinkRef).
    // In entry mode, aggregation happens over the *closed* entry, so we close first.
    let local_sink = test_entry_sink();
    let mut local_agg: KeyedAggregator<LocalRefMetrics, _> = KeyedAggregator::new(local_sink.sink);
    let e1 = LocalRefMetrics {
        endpoint: "api".to_string(),
        latency: Duration::from_millis(10),
    }
    .close();
    let e2 = LocalRefMetrics {
        endpoint: "api".to_string(),
        latency: Duration::from_millis(20),
    }
    .close();
    local_agg.merge_ref(&e1);
    local_agg.merge_ref(&e2);
    local_agg.flush();
    let entries = local_sink.inspector.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metrics["latency"].num_observations(), 2);

    // Exercise the dependency-crate ref aggregate the same way. The fact that both
    // `#[aggregate(ref)]` types compile and merge_ref together proves the cross-crate
    // MergeRef coherence fix.
    let dep_sink = test_entry_sink();
    let mut dep_agg: KeyedAggregator<DepRefMetrics, _> = KeyedAggregator::new(dep_sink.sink);
    let d1 = DepRefMetrics {
        endpoint: "svc".to_string(),
        latency: Duration::from_millis(30),
    }
    .close();
    dep_agg.merge_ref(&d1);
    dep_agg.flush();
    let entries = dep_sink.inspector.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metrics["latency"].num_observations(), 1);
}
