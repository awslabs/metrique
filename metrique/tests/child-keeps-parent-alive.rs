// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::{assert_eq, time::Duration};
use std::sync::Arc;

use metrique::writer::sink::VecEntrySink;
use metrique::writer::test_util;
use metrique::{Counter, OnParentDrop, Slot, unit_of_work::metrics};
use tokio::{task, time::sleep};

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct ParentMetrics {
    duration: Option<Duration>,
    #[metrics(flatten)]
    child: Slot<ChildMetrics>,
}

#[metrics]
#[derive(Default)]
struct ChildMetrics {
    a: usize,
    b: usize,

    counter: Counter,
}

#[tokio::test]
async fn flush_guards() {
    let q = VecEntrySink::new();
    let mut metrics = ParentMetrics::default().append_on_drop(q.clone());

    let mut child = metrics.child.open(OnParentDrop::Discard).unwrap();
    child.delay_flush(metrics.flush_guard());

    // you can still mutate metrics
    metrics.duration = Some(Duration::from_secs(5));

    // prematurely drop the root metrics
    drop(metrics);

    // record hasn't been flushed yet
    assert_eq!(q.drain().len(), 0);

    child.a = 5000;

    // You can also `Arc` the child to share it across tasks
    // at which point the final task dropping flushes the parent
    let child = Arc::from(child);
    let mut tasks = vec![];
    for _i in 0..10 {
        let child = child.clone();
        tasks.push(task::spawn(async move {
            child.counter.increment();
        }));
    }
    drop(child);

    for task in tasks {
        let _ = task.await;
    }

    let result = q.drain();
    assert_eq!(result.len(), 1);
    let entry = test_util::to_test_entry(&result[0]);
    // Verify that it has the latest value
    assert_eq!(entry.metrics["A"].as_u64(), 5000);
    assert_eq!(entry.metrics["Counter"].as_u64(), 10);
}

#[tokio::test]
async fn force_flush() {
    let q = VecEntrySink::new();
    let mut metrics = ParentMetrics::default().append_on_drop(q.clone());

    let flush_guard = metrics.flush_guard();
    let mut child = metrics.child.open(OnParentDrop::Wait(flush_guard)).unwrap();

    // you can still mutate metrics
    metrics.duration = Some(Duration::from_secs(5));

    child.a = 5000;
    // this task won't complete
    task::spawn(async move {
        sleep(Duration::from_secs(100000)).await;
        child.b = 10;
    });

    // our "request" is done, set a timeout for actually flushing
    let force_flush = metrics.force_flush_guard();
    let result = q.drain();
    assert_eq!(result.len(), 0);
    drop(metrics);
    drop(force_flush);

    // the data from the child never came back
    let result = q.drain();
    assert_eq!(result.len(), 1);
    let entry = test_util::to_test_entry(&result[0]);

    // in this case, writes are not written back to the parent
    //assert!(matches!(result[0].child, None));
    assert!(entry.metrics.get("Child").is_none());
    assert_eq!(entry.metrics["Duration"].as_u64(), 5000);
}
