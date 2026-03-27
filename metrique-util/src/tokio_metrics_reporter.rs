// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use metrique_writer_core::global::{AttachGlobalEntrySink, GlobalEntrySink};
use metrique_writer_core::{BoxEntrySink, EntrySink};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_metrics::RuntimeMonitor;

const DEFAULT_METRIC_SAMPLING_INTERVAL: Duration = Duration::from_secs(30);

/// Configuration for Tokio runtime metrics bridge subscriptions.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TokioRuntimeMetricsConfig {
    /// Sampling interval used by the reporter loop.
    interval: Duration,
}

impl Default for TokioRuntimeMetricsConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_METRIC_SAMPLING_INTERVAL,
        }
    }
}

impl TokioRuntimeMetricsConfig {
    /// Return a config with a custom sampling interval.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
}

/// Extension methods for subscribing Tokio runtime metrics to a global entry sink.
pub trait AttachGlobalEntrySinkTokioMetricsExt: AttachGlobalEntrySink + GlobalEntrySink {
    /// Subscribe to Tokio runtime metrics, adding the subscription to this handle.
    ///
    /// The reporter task is automatically aborted when the [`AttachHandle`] is dropped.
    ///
    /// # Panics
    /// Panics if the underlying sink has been detached (e.g. the `AttachHandle` was
    /// dropped elsewhere before this call).
    ///
    /// [`AttachHandle`]: metrique_writer_core::global::AttachHandle
    fn subscribe_tokio_runtime_metrics(config: TokioRuntimeMetricsConfig) {
        let sink = Self::sink();
        let task = spawn_tokio_runtime_metrics_task(sink, config);
        Self::register_shutdown_fn(Box::new(move || {
            task.abort();
        }));
    }
}

impl<T: AttachGlobalEntrySink + GlobalEntrySink> AttachGlobalEntrySinkTokioMetricsExt for T {}

fn spawn_tokio_runtime_metrics_task(
    sink: BoxEntrySink,
    config: TokioRuntimeMetricsConfig,
) -> JoinHandle<()> {
    let interval = config.interval;
    tokio::spawn(async move {
        tracing::debug!("tokio runtime metrics reporter started");
        let handle = Handle::current();
        let monitor = RuntimeMonitor::new(&handle);
        for snapshot in monitor.intervals() {
            // Take histogram counts before moving snapshot into append.
            // Bucket ranges come from the runtime handle at format time.
            #[cfg(tokio_unstable)]
            let (snapshot, histogram_counts) = {
                let mut snapshot = snapshot;
                let counts = std::mem::take(&mut snapshot.poll_time_histogram);
                (snapshot, counts)
            };
            sink.append(snapshot);
            #[cfg(tokio_unstable)]
            emit_poll_time_histogram(&sink, histogram_counts, handle.metrics());
            tokio::time::sleep(interval).await;
        }
        tracing::debug!("tokio runtime metrics reporter stopped");
    })
}

/// Emit `poll_time_histogram` bucket counts as a metrique distribution metric,
/// pairing each bucket's count with its range from the runtime handle.
#[cfg(tokio_unstable)]
fn emit_poll_time_histogram(
    sink: &BoxEntrySink,
    counts: Vec<u64>,
    rt: tokio::runtime::RuntimeMetrics,
) {
    use metrique_writer_core::value::MetricFlags;
    use metrique_writer_core::{Entry, EntryWriter, Observation, Unit, unit::NegativeScale};

    // Emitted as a separate entry alongside RuntimeMetrics because
    // `poll_time_histogram` uses #[entry(ignore)] on RuntimeMetrics — the raw
    // Vec<u64> counts need bucket ranges from the runtime handle to be
    // meaningful, which Entry::write() doesn't have access to.
    struct PollTimeHistogramEntry {
        counts: Vec<u64>,
        rt: tokio::runtime::RuntimeMetrics,
    }

    impl Entry for PollTimeHistogramEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("poll_time_histogram", self);
        }
    }

    impl metrique_writer_core::Value for PollTimeHistogramEntry {
        fn write(&self, writer: impl metrique_writer_core::ValueWriter) {
            writer.metric(
                self.counts
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c > 0)
                    .map(|(i, &count)| {
                        let range = self.rt.poll_time_histogram_bucket_range(i);
                        let midpoint_us =
                            (range.start.as_micros() + range.end.as_micros()) as f64 / 2.0;
                        Observation::Repeated {
                            total: midpoint_us * count as f64,
                            occurrences: count,
                        }
                    }),
                Unit::Second(NegativeScale::Micro),
                [],
                MetricFlags::empty(),
            );
        }
    }

    sink.append(PollTimeHistogramEntry { counts, rt });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use metrique_writer::sink::AttachGlobalEntrySink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

    use super::{AttachGlobalEntrySinkTokioMetricsExt, TokioRuntimeMetricsConfig};

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        // Advance past a few intervals so the reporter loop emits entries.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !inspector.entries().is_empty(),
            "expected tokio runtime metrics entries"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_aborted_on_handle_drop() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        // Let some entries accumulate.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let count_before = inspector.entries().len();
        assert!(count_before > 0);

        // Drop the attach handle — this should abort the reporter task.
        drop(handle);

        // Advance time further; no new entries should appear.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(inspector.entries().len(), count_before);
    }
}
