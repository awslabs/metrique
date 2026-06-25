// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding the Tokio task metrics of a single request into your own entries.
//!
//! [`TaskMonitor`](tokio_metrics::TaskMonitor) aggregates metrics across every
//! future it instruments. [`TaskTiming`] instead captures the metrics of *one*
//! future — a single request running within a larger task — so they can be
//! flattened onto that request's own metric record, the same way
//! [`TokioRuntimeSnapshot`](crate::TokioRuntimeSnapshot) folds in runtime
//! metrics.

use std::future::Future;

use metrique::CloseValue;
use tokio_metrics::{RequestMonitor, RequestTaskMetrics};

/// A metrique field holding the captured Tokio task metrics of a single request.
///
/// Wrap the request's future with [`TaskTiming::instrument`]; awaiting the
/// returned future yields the future's output together with a `TaskTiming`. Fold
/// that into your metric struct with `#[metrics(flatten)]` and, on close, it
/// emits the request's poll, idle, first-poll, and scheduling metrics (see
/// [`RequestTaskMetrics`](tokio_metrics::RequestTaskMetrics) for the full list).
///
/// `instrument` is a one-shot associated function that takes ownership of the
/// future, so a `TaskTiming` always describes exactly one request — there is no
/// way to accidentally reuse it across futures.
///
/// Idle, poll, and first-poll metrics are measured locally from the request's
/// own future, so they stay accurate even when the surrounding task interleaves
/// other work. **Scheduling delay can only be observed by the root future the
/// runtime schedules**, so for `scheduled_count`/`total_scheduled_duration`/
/// `long_delay_count` to be populated, the surrounding task must be instrumented
/// with a monitor built via
/// [`TaskMonitorBuilder::record_request_scheduling`](tokio_metrics::TaskMonitorBuilder::record_request_scheduling).
/// Without that, those fields are zero and the rest still work.
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique_util::TaskTiming;
/// use tokio_metrics::TaskMonitor;
///
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: &'static str,
///     success: bool,
///     // Folds in TaskPollCount, TaskTotalPollDuration, TaskTotalIdleDuration,
///     // TaskTotalScheduledDuration, TaskFirstPollDelay, ...
///     #[metrics(flatten, prefix = "task_")]
///     timing: TaskTiming,
/// }
///
/// # async fn handle_request() -> bool { true }
/// async fn run() {
///     // build the worker monitor with scheduling capture enabled, then
///     // instrument the larger task once
///     let mut builder = TaskMonitor::builder();
///     builder.record_request_scheduling();
///     let task_monitor = builder.build();
///     task_monitor
///         .instrument(async {
///             let (success, timing) = TaskTiming::instrument(handle_request()).await;
///             let _m = RequestMetrics {
///                 operation: "Read",
///                 success,
///                 timing,
///             };
///             // `_m.append_on_drop(sink)` in real code
///         })
///         .await;
/// }
/// ```
#[derive(Clone, Debug)]
pub struct TaskTiming {
    metrics: RequestTaskMetrics,
}

impl TaskTiming {
    /// Instruments the request's future. Awaiting the returned future yields the
    /// future's output paired with the `TaskTiming` to fold into your metrics.
    pub fn instrument<F: Future>(task: F) -> impl Future<Output = (F::Output, TaskTiming)> {
        async move {
            let (output, metrics) = RequestMonitor::new().instrument(task).await;
            (output, TaskTiming { metrics })
        }
    }

    /// The metrics captured for this request.
    pub fn metrics(&self) -> &RequestTaskMetrics {
        &self.metrics
    }
}

impl CloseValue for TaskTiming {
    type Closed = <RequestTaskMetrics as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        self.metrics.close()
    }
}

impl CloseValue for &'_ TaskTiming {
    type Closed = <RequestTaskMetrics as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        self.metrics.clone().close()
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use metrique::unit_of_work::metrics;
    use metrique_writer::test_util::test_metric;
    use tokio_metrics::TaskMonitor;

    use super::TaskTiming;

    #[metrics(rename_all = "PascalCase")]
    struct RequestMetrics {
        operation: &'static str,
        #[metrics(flatten, prefix = "task_")]
        timing: TaskTiming,
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn folds_per_request_metrics_into_entry() {
        let mut builder = TaskMonitor::builder();
        builder.record_request_scheduling();
        let task_monitor = builder.build();
        task_monitor
            .instrument(async {
                let (_, timing) = TaskTiming::instrument(async {
                    tokio::task::yield_now().await; // extra poll
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await; // idle
                })
                .await;

                let entry = test_metric(RequestMetrics {
                    operation: "Read",
                    timing,
                });

                check!(entry.values["Operation"] == "Read");
                check!(entry.metrics["TaskPollCount"] >= 2);
                check!(entry.metrics["TaskTotalIdleDuration"] >= 1.0);
            })
            .await;
    }
}
