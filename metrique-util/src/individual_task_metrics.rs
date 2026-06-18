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
use std::time::Duration;

use metrique::CloseValue;
use tokio_metrics::{InstrumentedRequest, RequestMonitor, RequestTaskMetrics};

/// A metrique field capturing the Tokio task metrics of a single request.
///
/// Wrap the request's future with [`instrument`](TaskTiming::instrument), await
/// it, then fold the `TaskTiming` into your metric struct with
/// `#[metrics(flatten)]`. On close it emits the request's poll, idle,
/// first-poll, and scheduling metrics (see
/// [`RequestTaskMetrics`](tokio_metrics::RequestTaskMetrics) for the full field
/// list).
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
///             let timing = TaskTiming::new();
///             let success = timing.instrument(handle_request()).await;
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
#[derive(Clone, Debug, Default)]
pub struct TaskTiming {
    monitor: RequestMonitor,
}

impl TaskTiming {
    /// Creates a new `TaskTiming` for a single request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `TaskTiming` whose local poll metrics use a custom slow-poll
    /// threshold (see
    /// [`TaskMonitor::with_slow_poll_threshold`](tokio_metrics::TaskMonitor::with_slow_poll_threshold)).
    pub fn with_slow_poll_threshold(threshold: Duration) -> Self {
        Self {
            monitor: RequestMonitor::with_slow_poll_threshold(threshold),
        }
    }

    /// Instruments the request's future. Await the returned future, then fold
    /// this `TaskTiming` into your metric struct.
    pub fn instrument<F: Future>(&self, task: F) -> InstrumentedRequest<F> {
        self.monitor.instrument(task)
    }

    /// Returns the metrics captured for this request so far.
    pub fn metrics(&self) -> RequestTaskMetrics {
        self.monitor.metrics()
    }
}

impl CloseValue for TaskTiming {
    type Closed = <RequestTaskMetrics as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        self.monitor.metrics().close()
    }
}

impl CloseValue for &'_ TaskTiming {
    type Closed = <RequestTaskMetrics as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        self.monitor.metrics().close()
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
                let timing = TaskTiming::new();
                timing
                    .instrument(async {
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
