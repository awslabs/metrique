// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use metrique_core::{CloseValue, InflectableEntry};
use metrique_writer_core::global::{AttachGlobalEntrySink, GlobalEntrySink};
use metrique_writer_core::{BoxEntrySink, Entry, EntrySink, EntryWriter};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_metrics::RuntimeMonitor;

const DEFAULT_METRIC_SAMPLING_INTERVAL: Duration = Duration::from_secs(30);

/// Runtime metric field naming style used by the Tokio metrics bridge.
///
/// This is a re-export of [`metrique_core::DynamicNameStyle`].
pub use metrique_core::DynamicNameStyle as MetricNameStyle;

/// Configuration for Tokio runtime metrics bridge subscriptions.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct TokioRuntimeMetricsConfig {
    /// Sampling interval used by the reporter loop.
    interval: Duration,
    /// Name style for emitted metric fields.
    name_style: MetricNameStyle,
}

impl Default for TokioRuntimeMetricsConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_METRIC_SAMPLING_INTERVAL,
            name_style: MetricNameStyle::default(),
        }
    }
}

impl TokioRuntimeMetricsConfig {
    /// Return a config with a custom sampling interval.
    pub fn with_interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// Set the name style for emitted metric fields.
    ///
    /// Defaults to [`MetricNameStyle::Identity`].
    pub fn with_name_style(self, name_style: MetricNameStyle) -> Self {
        Self { name_style, ..self }
    }
}

/// Extension methods for subscribing Tokio runtime metrics to a global entry sink.
///
/// Spawns a background task that periodically samples
/// [`RuntimeMetrics`](tokio_metrics::RuntimeMetrics) and appends each snapshot to the sink.
/// The task is automatically aborted when the [`AttachHandle`] is dropped.
///
/// # `tokio_unstable`
///
/// When the runtime is built with `RUSTFLAGS="--cfg tokio_unstable"` and
/// `enable_metrics_poll_time_histogram` is called on the runtime builder, each
/// snapshot also includes a `poll_time_histogram` entry emitted as a distribution
/// metric with bucket ranges from the runtime handle.
///
/// # Example
///
/// ```rust,ignore
/// use metrique_util::{
///     AttachGlobalEntrySinkTokioMetricsExt, MetricNameStyle, TokioRuntimeMetricsConfig,
/// };
/// use std::time::Duration;
///
/// let _handle = ServiceMetrics::attach_to_stream(emf.output_to(std::io::stderr()));
///
/// let config = TokioRuntimeMetricsConfig::default()
///     .with_interval(Duration::from_secs(30))
///     .with_name_style(MetricNameStyle::PascalCase);
/// ServiceMetrics::subscribe_tokio_runtime_metrics(config);
/// ```
///
/// [`AttachHandle`]: metrique_writer_core::global::AttachHandle
pub trait AttachGlobalEntrySinkTokioMetricsExt: AttachGlobalEntrySink + GlobalEntrySink {
    /// Subscribe to Tokio runtime metrics, adding the subscription to this handle.
    ///
    /// The reporter task is automatically aborted when the [`AttachHandle`] is dropped.
    /// If the handle is [`forgotten`], the reporter runs indefinitely.
    ///
    /// # Panics
    /// Panics if no sink has been attached yet, or if the underlying sink has been
    /// detached (e.g. the `AttachHandle` was dropped or forgotten before this call).
    ///
    /// [`AttachHandle`]: metrique_writer_core::global::AttachHandle
    /// [`forgotten`]: metrique_writer_core::global::AttachHandle::forget
    fn subscribe_tokio_runtime_metrics(config: TokioRuntimeMetricsConfig) {
        // Guard against duplicate subscriptions within the same attach cycle.
        // Reset in the shutdown fn so re-attaching can subscribe again cleanly.
        static SUBSCRIBED: AtomicBool = AtomicBool::new(false);
        if SUBSCRIBED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "subscribe_tokio_runtime_metrics called more than once; \
                 duplicate reporter task will emit duplicate metrics"
            );
        }
        let sink = Self::sink();
        let (worker_abort, monitor) = spawn_tokio_runtime_metrics_task(sink, config);
        Self::register_shutdown_fn(Box::new(move || {
            worker_abort.abort();
            monitor.abort();
            SUBSCRIBED.store(false, Ordering::Relaxed);
        }));
    }
}

impl<T: AttachGlobalEntrySink + GlobalEntrySink> AttachGlobalEntrySinkTokioMetricsExt for T {}

fn spawn_tokio_runtime_metrics_task(
    sink: BoxEntrySink,
    config: TokioRuntimeMetricsConfig,
) -> (tokio::task::AbortHandle, JoinHandle<()>) {
    let interval = config.interval;
    let name_style = config.name_style;
    let worker = tokio::spawn(async move {
        tracing::debug!("tokio runtime metrics reporter started");
        let handle = Handle::current();
        let monitor = RuntimeMonitor::new(&handle);
        for snapshot in monitor.intervals() {
            sink.append(RootedEntry {
                entry: snapshot.close(),
                name_style,
            });
            tokio::time::sleep(interval).await;
        }
        tracing::debug!("tokio runtime metrics reporter stopped");
    });
    let worker_abort = worker.abort_handle();
    let monitor = tokio::spawn(async move {
        match worker.await {
            Ok(()) => {}
            Err(err) if err.is_cancelled() => {
                tracing::debug!("tokio runtime metrics reporter cancelled");
            }
            Err(err) => {
                tracing::error!(?err, "tokio runtime metrics reporter panicked");
            }
        }
    });
    (worker_abort, monitor)
}

/// Wrapper that roots an [`InflectableEntry`] into an [`Entry`], applying the
/// configured [`MetricNameStyle`].
struct RootedEntry<M> {
    entry: M,
    name_style: MetricNameStyle,
}

impl<M> Entry for RootedEntry<M>
where
    M: InflectableEntry<metrique_core::Identity>
        + InflectableEntry<metrique_core::PascalCase>
        + InflectableEntry<metrique_core::SnakeCase>
        + InflectableEntry<metrique_core::KebabCase>,
{
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        use metrique_core::DynamicNameStyle;
        match self.name_style {
            DynamicNameStyle::Identity => {
                InflectableEntry::<metrique_core::Identity>::write(&self.entry, w)
            }
            DynamicNameStyle::PascalCase => {
                InflectableEntry::<metrique_core::PascalCase>::write(&self.entry, w)
            }
            DynamicNameStyle::SnakeCase => {
                InflectableEntry::<metrique_core::SnakeCase>::write(&self.entry, w)
            }
            DynamicNameStyle::KebabCase => {
                InflectableEntry::<metrique_core::KebabCase>::write(&self.entry, w)
            }
            _ => {
                static WARNED_UNKNOWN_NAME_STYLE: AtomicBool = AtomicBool::new(false);
                if !WARNED_UNKNOWN_NAME_STYLE.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        ?self.name_style,
                        "unknown MetricNameStyle variant; falling back to Identity"
                    );
                }
                InflectableEntry::<metrique_core::Identity>::write(&self.entry, w)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use metrique_writer::sink::AttachGlobalEntrySink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

    use super::{
        AttachGlobalEntrySinkTokioMetricsExt, MetricNameStyle, TokioRuntimeMetricsConfig,
    };

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_identity() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        assert!(!entries.is_empty(), "expected entries");

        let entry = &entries[0];
        assert!(
            entry.metrics.contains_key("workers_count"),
            "expected snake_case field names with Identity style, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
        assert!(entry.metrics.contains_key("total_park_count"));
        assert!(entry.metrics.contains_key("elapsed"));

        #[cfg(tokio_unstable)]
        assert!(
            entry.metrics.contains_key("poll_time_histogram"),
            "expected poll_time_histogram under tokio_unstable, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_pascal_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::PascalCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        assert!(!entries.is_empty(), "expected entries");

        let entry = &entries[0];
        assert!(
            entry.metrics.contains_key("WorkersCount"),
            "expected PascalCase field names, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
        assert!(entry.metrics.contains_key("TotalParkCount"));
        assert!(entry.metrics.contains_key("Elapsed"));

        #[cfg(tokio_unstable)]
        assert!(
            entry.metrics.contains_key("PollTimeHistogram"),
            "expected PollTimeHistogram under tokio_unstable, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_snake_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::SnakeCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        assert!(!entries.is_empty(), "expected entries");

        let entry = &entries[0];
        assert!(
            entry.metrics.contains_key("workers_count"),
            "expected snake_case field names, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
        assert!(entry.metrics.contains_key("total_park_count"));
        assert!(entry.metrics.contains_key("elapsed"));

        #[cfg(tokio_unstable)]
        assert!(
            entry.metrics.contains_key("poll_time_histogram"),
            "expected poll_time_histogram under tokio_unstable, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_kebab_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::KebabCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        assert!(!entries.is_empty(), "expected entries");

        let entry = &entries[0];
        assert!(
            entry.metrics.contains_key("workers-count"),
            "expected kebab-case field names, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
        );
        assert!(entry.metrics.contains_key("total-park-count"));
        assert!(entry.metrics.contains_key("elapsed"));

        #[cfg(tokio_unstable)]
        assert!(
            entry.metrics.contains_key("poll-time-histogram"),
            "expected poll-time-histogram under tokio_unstable, got keys: {:?}",
            entry.metrics.keys().collect::<Vec<_>>()
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
