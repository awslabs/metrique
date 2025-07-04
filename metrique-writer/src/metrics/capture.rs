// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module contains functions for capturing metrics, generally to be used in tests.

use std::future::Future;

use metrics::Recorder;
use pin_project::pin_project;

use crate::metrics::{MetricAccumulatorEntry, MetricRecorder};

/// Run `f`, capturing the metrics while it runs using a local recorder.
pub fn capture_metrics<T, F: FnOnce() -> T>(f: F) -> (MetricAccumulatorEntry, T) {
    let accumulator: MetricRecorder = MetricRecorder::new();
    let res = metrics::with_local_recorder(&accumulator, f);
    (accumulator.readout(), res)
}

/// Asynchrounously run `f`, capturing the metrics while it runs using a local recorder.
///
/// If `f` spawns subtasks, metrics from the subtasks will *not* be captured.
pub async fn capture_metrics_async<T, F: Future<Output = T>>(f: F) -> (MetricAccumulatorEntry, T) {
    let accumulator = MetricRecorder::new();
    let res = LocalRecorderWrapper::new(accumulator.clone(), f).await;
    (accumulator.readout(), res)
}

/// Wraps a future to install a local recorder during the executor of said future.
#[pin_project]
pub struct LocalRecorderWrapper<R: Recorder, F: Future> {
    recorder: R,
    #[pin]
    future: F,
}

impl<R: Recorder, F: Future> Future for LocalRecorderWrapper<R, F> {
    type Output = F::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();
        metrics::with_local_recorder(this.recorder, || this.future.poll(cx))
    }
}

impl<R: Recorder, F: Future> LocalRecorderWrapper<R, F> {
    /// Create a new `LocalRecorderWrapper`
    pub fn new(recorder: R, future: F) -> Self {
        Self { recorder, future }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_capture_metrics() {
        let (metrics, _) = super::capture_metrics(|| {
            metrics::counter!("foo").increment(9);
            metrics::counter!("foo").increment(3);
            metrics::gauge!("bar").set(5);
            metrics::histogram!("baz").record(100);
            metrics::histogram!("baz").record(101);
            metrics::histogram!("baz").record(1000);
        });
        assert_eq!(metrics.counter_value("foo"), Some(12));
        assert_eq!(metrics.counter_value("nothing"), None);
        assert_eq!(metrics.gauge_value("bar"), Some(5.0));
        assert_eq!(metrics.gauge_value("nothing"), None);
        // FIXME: use fuzzy matching for histogram?
        assert_eq!(metrics.histogram_value("baz"), vec![101, 101, 1007]);
        assert_eq!(metrics.histogram_value("nothing"), Vec::<u32>::new());
    }

    #[tokio::test]
    async fn test_capture_metrics_async() {
        let (metrics, _) = super::capture_metrics_async(async move {
            metrics::counter!("foo").increment(9);
            metrics::counter!("foo").increment(3);
            tokio::task::yield_now().await;
            metrics::gauge!("bar").set(5);
            metrics::histogram!("baz").record(100);
            metrics::histogram!("baz").record(101);
            metrics::histogram!("baz").record(1000);
        })
        .await;
        assert_eq!(metrics.counter_value("foo"), Some(12));
        assert_eq!(metrics.counter_value("nothing"), None);
        assert_eq!(metrics.gauge_value("bar"), Some(5.0));
        assert_eq!(metrics.gauge_value("nothing"), None);
        // FIXME: use fuzzy matching for histogram?
        assert_eq!(metrics.histogram_value("baz"), vec![101, 101, 1007]);
        assert_eq!(metrics.histogram_value("nothing"), Vec::<u32>::new());
    }
}
