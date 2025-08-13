// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::time::Duration;

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt};
use metrique::writer::{GlobalEntrySink, sink::global_entry_sink};
use metrique::{Counter, OnParentDrop, Slot, SlotGuard};
use tokio::task;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(flatten)]
    background_metrics: Slot<BackgroundMetrics>,
}

#[metrics]
#[derive(Default)]
struct BackgroundMetrics {
    field_1: usize,
    counter: Counter,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("MyApp".into(), vec![vec![]]).output_to(std::io::stderr()),
    );
    let task_tracker = TaskTracker::new();
    assert_eq!(handle_request(&task_tracker).await, 42);
    task_tracker.close();
    task_tracker.wait().await;
}

async fn handle_request(task_tracker: &TaskTracker) -> usize {
    let mut metrics = RequestMetrics {
        operation: "abc",
        background_metrics: Default::default(),
    }
    .append_on_drop(ServiceMetrics::sink());

    // You can create a `FlushGuard` from the root metrics. The root metrics can then be dropped (but the record won't be flushed yet.).
    // This ensures that any metrics from the backgound work will make it into the final entry. The entry itself will not be written to
    // the sink until the `SlotGuard` is dropped.
    //
    // NOTE: this does not impact the **actual** `drop` of the root entry or delay the main task. It solely means that the metric
    //       will not be flushed until the `FlushGuard` is dropped.
    let flush_guard = metrics.flush_guard();

    let background_metrics = metrics
        .background_metrics
        // Or if you don't want to delay flushing, use `OnParentDrop::Ignore`
        .open(OnParentDrop::Wait(flush_guard))
        .unwrap();

    // NOTE: you can also hold a flush guard yourself, it does not NEED to be embedded into a slot to function.

    // But perhaps you are not willing to wait forever. In this case,
    // you can create a ForceFlush to ensure that the root entry itself
    // is flushed in a reasonable amount of time.
    let force_flush = metrics.force_flush_guard();

    task::spawn(do_background_work(background_metrics));

    // You can use `force_flush` to ensure that the parent record is eventually flushed. This will not delay flushing
    // the record if background task completes, it only serves as a stopgap.
    // NOTE: YOU MUST ENSURE THIS FUTURE COMPLETES BEFORE SHUTDOWN
    // FUTURE FEATURE NOTE: Ideally, this future would drop when the metric itself drops. For this, we'd need to do a slightly
    // better job of integrating timeouts directly into metrics. In this case, this future will always complete which will slightly delay
    // total shutdown of the system.
    task_tracker.spawn(async move {
        sleep(Duration::from_millis(5000)).await;
        warn!("timeout occured");
        drop(force_flush);
    });
    // we already know the answer is 42
    42
}

async fn do_background_work(mut metrics: SlotGuard<BackgroundMetrics>) {
    // this will take awhile...
    sleep(Duration::from_secs(1)).await;
    metrics.field_1 += 1;
    info!("background work is complete");
}
