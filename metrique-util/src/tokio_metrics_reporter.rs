// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use crate::dynamic_inflection::DynamicInflectionEntry;
use metrique::CloseValue;
use metrique::writer::{AttachGlobalEntrySink, BoxEntrySink, EntrySink, ShutdownFn};
use tokio::runtime::Handle;
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
/// [`RuntimeMetrics`] and appends each snapshot to the sink.
/// The task is automatically aborted when the [`AttachHandle`](metrique::writer::sink::AttachHandle) is dropped.
///
/// ## `tokio_unstable`
///
/// This works with and without `tokio_unstable`. Without it, snapshots include
/// the stable runtime metrics: worker counts, park/steal counts, queue depths,
/// busy durations, and more. See [`RuntimeMetrics`] for the full field list.
///
/// Building with `RUSTFLAGS="--cfg tokio_unstable"` adds additional fields
/// such as `mean_poll_duration`, `num_remote_schedules`,
/// `budget_forced_yield_count`, and `poll_time_histogram`. The histogram
/// requires calling `enable_metrics_poll_time_histogram` on the runtime builder.
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
/// [`RuntimeMetrics`]: tokio_metrics::RuntimeMetrics
pub trait AttachGlobalEntrySinkTokioMetricsExt: AttachGlobalEntrySink + 'static {
    /// Subscribe to Tokio runtime metrics, adding the subscription to this handle.
    ///
    /// Spawns a background task that periodically samples [`RuntimeMetrics`] and
    /// appends each snapshot to the sink. Additional fields are available when
    /// building with `tokio_unstable`, see the
    /// [trait-level docs](AttachGlobalEntrySinkTokioMetricsExt)
    /// for details.
    ///
    /// The reporter task is automatically aborted when the [`AttachHandle`](metrique::writer::sink::AttachHandle) is dropped.
    /// If the handle is [`forgotten`](metrique::writer::sink::AttachHandle::forget), the reporter runs indefinitely.
    ///
    /// If no sink has been attached yet, entries are silently discarded until one
    /// is attached.
    ///
    /// [`RuntimeMetrics`]: tokio_metrics::RuntimeMetrics
    fn subscribe_tokio_runtime_metrics(config: TokioRuntimeMetricsConfig) {
        let sink = BoxEntrySink::lazy(Self::try_sink);
        let abort = spawn_tokio_runtime_metrics_task(sink, config);
        Self::register_shutdown_fn(ShutdownFn::new(move || {
            abort.abort();
        }));
    }
}

impl<T: AttachGlobalEntrySink + 'static> AttachGlobalEntrySinkTokioMetricsExt for T {}

fn spawn_tokio_runtime_metrics_task(
    sink: BoxEntrySink,
    config: TokioRuntimeMetricsConfig,
) -> tokio::task::AbortHandle {
    let interval = config.interval;
    let name_style = config.name_style;
    let worker = tokio::spawn(async move {
        tracing::debug!("tokio runtime metrics reporter started");
        let handle = Handle::current();
        let monitor = RuntimeMonitor::new(&handle);
        for snapshot in monitor.intervals() {
            sink.append(DynamicInflectionEntry {
                entry: snapshot.close(),
                name_style,
            });
            tokio::time::sleep(interval).await;
        }
        tracing::debug!("tokio runtime metrics reporter stopped");
    });
    let abort = worker.abort_handle();

    // Spawn a monitor to log panics
    tokio::spawn(async move {
        if let Err(err) = worker.await {
            if !err.is_cancelled() {
                tracing::error!("tokio runtime metrics reporter panicked: {err}");
            }
        }
    });
    abort
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use assert2::check;
    use metrique_writer::sink::AttachGlobalEntrySink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

    use super::{AttachGlobalEntrySinkTokioMetricsExt, MetricNameStyle, TokioRuntimeMetricsConfig};

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
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["workers_count"] == 1);
        check!(entry.metrics["elapsed"] > 0.0);
        check!(entry.metrics["total_park_count"] > 0);

        #[cfg(tokio_unstable)]
        check!(entry.metrics["poll_time_histogram"].num_observations() > 0);
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
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["WorkersCount"] == 1);
        check!(entry.metrics["Elapsed"] > 0.0);
        check!(entry.metrics["TotalParkCount"] > 0);

        #[cfg(tokio_unstable)]
        check!(entry.metrics["PollTimeHistogram"].num_observations() > 0);
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
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["workers_count"] == 1);
        check!(entry.metrics["elapsed"] > 0.0);
        check!(entry.metrics["total_park_count"] > 0);

        #[cfg(tokio_unstable)]
        check!(entry.metrics["poll_time_histogram"].num_observations() > 0);
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
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["workers-count"] == 1);
        check!(entry.metrics["elapsed"] > 0.0);
        check!(entry.metrics["total-park-count"] > 0);

        #[cfg(tokio_unstable)]
        check!(entry.metrics["poll-time-histogram"].num_observations() > 0);
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
        check!(count_before > 0);

        // Dropping the handle should abort the reporter task.
        drop(handle);

        // Advance time further, no new entries should be appended.
        tokio::time::sleep(Duration::from_millis(200)).await;
        check!(inspector.entries().len() == count_before);
    }
}
