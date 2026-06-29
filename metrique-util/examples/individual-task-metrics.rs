// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding the Tokio task metrics of a single request into its own entry.
//!
//! A long-lived worker task is instrumented once with a
//! [`TaskMonitor`](tokio_metrics::TaskMonitor) so that scheduling delay becomes
//! observable. Each request handled by that worker is wrapped with
//! [`TaskTiming`], whose per-request poll/idle/scheduling metrics are flattened
//! onto that request's EMF record.

use std::time::Duration;

use metrique::{
    ServiceMetrics,
    emf::Emf,
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink},
};
use metrique_util::TaskTiming;
use tokio_metrics::TaskMonitor;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    success: bool,
    #[metrics(flatten, prefix = "task_")]
    timing: TaskTiming,
}

async fn handle_request(operation: &'static str) -> bool {
    // A poll, an await point (idle), and another poll.
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    operation != "Fail"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("IndividualTaskMetricsExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    // The worker task is instrumented once. `record_request_scheduling` opts the
    // monitor into publishing scheduling delay so each request's `TaskTiming`
    // can observe it; without it, the scheduling fields would be zero.
    let worker_monitor = TaskMonitor::builder().record_request_scheduling().build();

    // Instrument the worker as a *spawned* task. Task metrics (especially
    // scheduling delay) only reflect reality when the instrumented future is the
    // root of a task the runtime actually schedules — i.e. wrapped at
    // `tokio::spawn` — rather than awaited inline as a sub-future of `main`.
    tokio::spawn(worker_monitor.instrument(async {
        // Each loop iteration emits one EMF record carrying that request's
        // own task metrics, e.g.:
        //   {"Operation":"Read", "Success":1,
        //    "TaskPollCount":3, "TaskTotalPollDuration":0.0001,
        //    "TaskTotalIdleDuration":0.02, "TaskFirstPollDelay":0.0,
        //    "TaskTotalScheduledDuration":0.0001, ...}
        for op in ["Read", "Write", "Read"] {
            let (success, timing) = TaskTiming::instrument(handle_request(op)).await;

            let _m = RequestMetrics {
                operation: op,
                success,
                timing,
            }
            .append_on_drop(ServiceMetrics::sink());
        }
    }))
    .await?;

    Ok(())
}
