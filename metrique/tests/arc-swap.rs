// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "arc-swap")]

use std::sync::Arc;

use arc_swap::ArcSwap;
use metrique::unit_of_work::metrics;
use metrique::writer::sink::VecEntrySink;
use metrique::writer::test_util;

#[metrics(subfield_owned, rename_all = "PascalCase")]
#[derive(Clone)]
struct ServerState {
    region: String,
    cell: String,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(flatten)]
    server_state: &'static ArcSwap<ServerState>,
    duck_count: usize,
}

#[test]
fn arc_swap_subfield_flattened() {
    let vec_sink = VecEntrySink::new();
    let server_state: &'static ArcSwap<ServerState> =
        Box::leak(Box::new(ArcSwap::from_pointee(ServerState {
            region: "us-east-1".into(),
            cell: "5".into(),
        })));

    let mut metrics = RequestMetrics {
        operation: "GetItem",
        server_state,
        duck_count: 0,
    }
    .append_on_drop(vec_sink.clone());
    metrics.duck_count = 42;
    drop(metrics);

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry.values["Operation"], "GetItem");
    assert_eq!(entry.values["Region"], "us-east-1");
    assert_eq!(entry.values["Cell"], "5");
    assert_eq!(entry.metrics["DuckCount"], 42);
}

/// Simulates concurrent requests straddling a config reload.
///
/// - req1 is created before the swap and closed after it (should see new state)
/// - req2 is created and closed before the swap (should see old state)
/// - req3 is created and closed after the swap (should see new state)
#[test]
fn arc_swap_concurrent_requests_across_config_reload() {
    let vec_sink = VecEntrySink::new();
    let server_state: &'static ArcSwap<ServerState> =
        Box::leak(Box::new(ArcSwap::from_pointee(ServerState {
            region: "us-east-1".into(),
            cell: "1".into(),
        })));

    // req1: in-flight during the swap, closed after
    let req1 = RequestMetrics {
        operation: "GetItem",
        server_state,
        duck_count: 1,
    }
    .append_on_drop(vec_sink.clone());

    // req2: fully completes before the swap
    let req2 = RequestMetrics {
        operation: "PutItem",
        server_state,
        duck_count: 2,
    }
    .append_on_drop(vec_sink.clone());
    drop(req2);

    // Config reload
    server_state.store(Arc::new(ServerState {
        region: "eu-west-1".into(),
        cell: "2".into(),
    }));

    // req3: created and closed after the swap
    let req3 = RequestMetrics {
        operation: "DeleteItem",
        server_state,
        duck_count: 3,
    }
    .append_on_drop(vec_sink.clone());
    drop(req3);

    // req1 finally completes after the swap
    drop(req1);

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 3);

    // req2 closed before swap: old state
    let e2 = test_util::to_test_entry(&entries[0]);
    assert_eq!(e2.values["Operation"], "PutItem");
    assert_eq!(e2.values["Region"], "us-east-1");
    assert_eq!(e2.values["Cell"], "1");

    // req3 closed after swap: new state
    let e3 = test_util::to_test_entry(&entries[1]);
    assert_eq!(e3.values["Operation"], "DeleteItem");
    assert_eq!(e3.values["Region"], "eu-west-1");
    assert_eq!(e3.values["Cell"], "2");

    // req1 was in-flight, closed after swap: new state
    let e1 = test_util::to_test_entry(&entries[2]);
    assert_eq!(e1.values["Operation"], "GetItem");
    assert_eq!(e1.values["Region"], "eu-west-1");
    assert_eq!(e1.values["Cell"], "2");
}

/// Spawns tasks that hold metric guards across a config swap.
/// Tasks started before the swap see the new state at close time.
#[tokio::test]
async fn arc_swap_spawned_tasks_across_config_reload() {
    let vec_sink = VecEntrySink::new();
    let server_state: &'static ArcSwap<ServerState> =
        Box::leak(Box::new(ArcSwap::from_pointee(ServerState {
            region: "us-east-1".into(),
            cell: "1".into(),
        })));

    let (pre_swap_tx, pre_swap_rx) = tokio::sync::oneshot::channel::<()>();
    let (swap_done_tx, swap_done_rx) = tokio::sync::oneshot::channel::<()>();

    // Task 1: created before swap, holds its guard until after the swap completes.
    let sink = vec_sink.clone();
    let task1 = tokio::spawn(async move {
        let metrics = RequestMetrics {
            operation: "GetItem",
            server_state,
            duck_count: 1,
        }
        .append_on_drop(sink);

        // Signal that we're holding the guard, then wait for the swap.
        pre_swap_tx.send(()).unwrap();
        swap_done_rx.await.unwrap();

        // Guard drops here, after the swap.
        drop(metrics);
    });

    // Wait for task1 to be holding its guard.
    pre_swap_rx.await.unwrap();

    // Task 2: created and completed before the swap.
    let sink = vec_sink.clone();
    let task2 = tokio::spawn(async move {
        let metrics = RequestMetrics {
            operation: "PutItem",
            server_state,
            duck_count: 2,
        }
        .append_on_drop(sink);
        drop(metrics);
    });
    task2.await.unwrap();

    // Config reload while task1 is still in-flight.
    server_state.store(Arc::new(ServerState {
        region: "eu-west-1".into(),
        cell: "2".into(),
    }));
    swap_done_tx.send(()).unwrap();

    // Task 3: created after the swap.
    let sink = vec_sink.clone();
    let task3 = tokio::spawn(async move {
        let metrics = RequestMetrics {
            operation: "DeleteItem",
            server_state,
            duck_count: 3,
        }
        .append_on_drop(sink);
        drop(metrics);
    });

    task1.await.unwrap();
    task3.await.unwrap();

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 3);

    // task2 closed before swap: old state
    let e2 = test_util::to_test_entry(&entries[0]);
    assert_eq!(e2.values["Operation"], "PutItem");
    assert_eq!(e2.values["Region"], "us-east-1");
    assert_eq!(e2.values["Cell"], "1");

    // task1 closed after swap: new state
    let e1 = test_util::to_test_entry(&entries[1]);
    assert_eq!(e1.values["Operation"], "GetItem");
    assert_eq!(e1.values["Region"], "eu-west-1");
    assert_eq!(e1.values["Cell"], "2");

    // task3 created and closed after swap: new state
    let e3 = test_util::to_test_entry(&entries[2]);
    assert_eq!(e3.values["Operation"], "DeleteItem");
    assert_eq!(e3.values["Region"], "eu-west-1");
    assert_eq!(e3.values["Cell"], "2");
}
