// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "state")]

use std::sync::Arc;

use metrique::unit_of_work::metrics;
use metrique::writer::sink::VecEntrySink;
use metrique::writer::test_util;
use metrique_util::State;

#[derive(Clone, Debug, Default)]
#[metrics(subfield)]
struct AppConfig {
    feature_xyz_enabled: bool,
    traffic_policy: TrafficPolicy,
}

#[derive(Clone, Copy, Debug, Default)]
#[metrics(value(string))]
enum TrafficPolicy {
    #[default]
    Default,
    Canary,
}

#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
    operation: &'static str,
    #[metrics(flatten)]
    config: State<AppConfig>,
    duck_count: usize,
}

#[test]
fn state_flattened() {
    let vec_sink = VecEntrySink::new();

    let mut metrics = MyMetrics {
        operation: "PutItem",
        config: State::new(AppConfig {
            feature_xyz_enabled: false,
            traffic_policy: TrafficPolicy::Default,
        }),
        duck_count: 0,
    }
    .append_on_drop(vec_sink.clone());
    metrics.duck_count = 7;
    drop(metrics);

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry.values["Operation"], "PutItem");
    assert_eq!(entry.metrics["FeatureXyzEnabled"], 0);
    assert_eq!(entry.metrics["DuckCount"], 7);
}

/// First load() captures the value. Later writes don't affect it.
#[test]
fn state_snapshot_on_first_load() {
    let vec_sink = VecEntrySink::new();
    let state = State::new(AppConfig {
        feature_xyz_enabled: false,
        traffic_policy: TrafficPolicy::Default,
    });

    let metrics = MyMetrics {
        operation: "GetItem",
        config: state.clone(),
        duck_count: 1,
    }
    .append_on_drop(vec_sink.clone());

    // Read config in business logic (captures the snapshot).
    let _config = metrics.config.snapshot();

    // Update after the snapshot was captured.
    state.store(Arc::new(AppConfig {
        feature_xyz_enabled: true,
        traffic_policy: TrafficPolicy::Canary,
    }));

    drop(metrics);

    let entries = vec_sink.drain();
    let entry = test_util::to_test_entry(&entries[0]);
    // Metric sees the old value (captured on first load).
    assert_eq!(entry.metrics["FeatureXyzEnabled"], 0);
}

/// Simulates concurrent requests straddling a config reload.
/// Each request clones the State and loads at different times.
#[test]
fn state_across_config_reload() {
    let vec_sink = VecEntrySink::new();
    let state = State::new(AppConfig {
        feature_xyz_enabled: false,
        traffic_policy: TrafficPolicy::Default,
    });

    // req1: clone and load before the swap, closed after
    let req1 = MyMetrics {
        operation: "GetItem",
        config: state.clone(),
        duck_count: 1,
    }
    .append_on_drop(vec_sink.clone());
    let _config = req1.config.snapshot();

    // req2: clone, load, and close before the swap
    let req2 = MyMetrics {
        operation: "PutItem",
        config: state.clone(),
        duck_count: 2,
    }
    .append_on_drop(vec_sink.clone());
    let _config = req2.config.snapshot();
    drop(req2);

    // Config reload
    state.store(Arc::new(AppConfig {
        feature_xyz_enabled: true,
        traffic_policy: TrafficPolicy::Canary,
    }));

    // req3: clone and load after the swap
    let req3 = MyMetrics {
        operation: "DeleteItem",
        config: state.clone(),
        duck_count: 3,
    }
    .append_on_drop(vec_sink.clone());
    let _config = req3.config.snapshot();
    drop(req3);

    // req1 closes after the swap, but its snapshot is from before
    drop(req1);

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 3);

    // req2: old state
    let e2 = test_util::to_test_entry(&entries[0]);
    assert_eq!(e2.values["Operation"], "PutItem");
    assert_eq!(e2.metrics["FeatureXyzEnabled"], 0);

    // req3: new state
    let e3 = test_util::to_test_entry(&entries[1]);
    assert_eq!(e3.values["Operation"], "DeleteItem");
    assert_eq!(e3.metrics["FeatureXyzEnabled"], 1);

    // req1: old state (loaded before swap, even though closed after)
    let e1 = test_util::to_test_entry(&entries[2]);
    assert_eq!(e1.values["Operation"], "GetItem");
    assert_eq!(e1.metrics["FeatureXyzEnabled"], 0);
}

/// Spawns tasks that load config at different times relative to a swap.
#[tokio::test]
async fn state_spawned_tasks_across_config_reload() {
    let vec_sink = VecEntrySink::new();
    let state: &'static State<AppConfig> = Box::leak(Box::new(State::new(AppConfig {
        feature_xyz_enabled: false,
        traffic_policy: TrafficPolicy::Default,
    })));

    let (pre_swap_tx, pre_swap_rx) = tokio::sync::oneshot::channel::<()>();
    let (swap_done_tx, swap_done_rx) = tokio::sync::oneshot::channel::<()>();

    // Task 1: loads before swap, holds guard until after swap completes.
    let sink = vec_sink.clone();
    let task1 = tokio::spawn(async move {
        let metrics = MyMetrics {
            operation: "GetItem",
            config: state.clone(),
            duck_count: 1,
        }
        .append_on_drop(sink);
        let _config = metrics.config.snapshot();

        pre_swap_tx.send(()).unwrap();
        swap_done_rx.await.unwrap();
        drop(metrics);
    });

    pre_swap_rx.await.unwrap();

    // Task 2: loads and completes before the swap.
    let sink = vec_sink.clone();
    let task2 = tokio::spawn(async move {
        let metrics = MyMetrics {
            operation: "PutItem",
            config: state.clone(),
            duck_count: 2,
        }
        .append_on_drop(sink);
        let _config = metrics.config.snapshot();
        drop(metrics);
    });
    task2.await.unwrap();

    // Config reload while task1 is still in-flight.
    state.store(Arc::new(AppConfig {
        feature_xyz_enabled: true,
        traffic_policy: TrafficPolicy::Canary,
    }));

    swap_done_tx.send(()).unwrap();

    // Task 3: loads after the swap.
    let sink = vec_sink.clone();
    let task3 = tokio::spawn(async move {
        let metrics = MyMetrics {
            operation: "DeleteItem",
            config: state.clone(),
            duck_count: 3,
        }
        .append_on_drop(sink);
        let _config = metrics.config.snapshot();
        drop(metrics);
    });

    task1.await.unwrap();
    task3.await.unwrap();

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 3);

    // task2: old state
    let e2 = test_util::to_test_entry(&entries[0]);
    assert_eq!(e2.values["Operation"], "PutItem");
    assert_eq!(e2.metrics["FeatureXyzEnabled"], 0);

    // task1: old state (loaded before swap)
    let e1 = test_util::to_test_entry(&entries[1]);
    assert_eq!(e1.values["Operation"], "GetItem");
    assert_eq!(e1.metrics["FeatureXyzEnabled"], 0);

    // task3: new state
    let e3 = test_util::to_test_entry(&entries[2]);
    assert_eq!(e3.values["Operation"], "DeleteItem");
    assert_eq!(e3.metrics["FeatureXyzEnabled"], 1);
}
