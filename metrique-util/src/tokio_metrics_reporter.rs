// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use crate::State;
use crate::dynamic_inflection::DynamicInflectionEntry;
use metrique::writer::{AttachGlobalEntrySink, BoxEntrySink, EntrySink, EntryWriter, ShutdownFn};
use metrique::{CloseValue, InflectableEntry, NameStyle};
use metrique_core::DynamicNameStyle as MetricNameStyle;
use tokio::runtime::Handle;
use tokio_metrics::{RuntimeMetrics, RuntimeMonitor};

type RtClosed = <RuntimeMetrics as CloseValue>::Closed;

/// Pre-closed Tokio runtime-metrics snapshot, embedded in a [`State`] so
/// each entry can flatten in the latest sample without cloning the
/// underlying data.
///
/// Obtain a `State<EmbeddedTokioMetrics>` by calling
/// [`AttachGlobalEntrySinkTokioMetricsExt::embed_tokio_runtime_metrics`] on
/// your global entry sink. The sampler is aborted when the sink's
/// [`AttachHandle`](metrique::writer::sink::AttachHandle) is dropped.
/// Embed the [`State`] in your entry with `#[metrics(flatten)]`.
///
/// Cloning the [`State`] (per request) and closing the entry are both
/// cheap reference-count operations.
#[derive(Clone)]
pub struct EmbeddedTokioMetrics(Arc<RtClosed>);

impl CloseValue for EmbeddedTokioMetrics {
    type Closed = Self;
    fn close(self) -> Self {
        self
    }
}

impl<NS: NameStyle> InflectableEntry<NS> for EmbeddedTokioMetrics
where
    RtClosed: InflectableEntry<NS>,
{
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        <RtClosed as InflectableEntry<NS>>::write(self.0.as_ref(), w);
    }

    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        <RtClosed as InflectableEntry<NS>>::sample_group(self.0.as_ref())
    }
}

const DEFAULT_METRIC_SAMPLING_INTERVAL: Duration = Duration::from_secs(30);

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

/// Extension methods for plugging Tokio runtime metrics into a global entry sink.
///
/// Two flavors are available, both backed by a single background sampler
/// task whose lifecycle is tied to the sink's
/// [`AttachHandle`](metrique::writer::sink::AttachHandle):
///
/// - [`subscribe_tokio_runtime_metrics`](Self::subscribe_tokio_runtime_metrics)
///   appends each [`RuntimeMetrics`] snapshot to the sink as a standalone
///   entry — best when you want a separate runtime-metrics record stream.
/// - [`embed_tokio_runtime_metrics`](Self::embed_tokio_runtime_metrics)
///   returns a [`State<EmbeddedTokioMetrics>`](EmbeddedTokioMetrics) you
///   embed into your own metric structs via `#[metrics(flatten)]` — best
///   when you want every emitted record to carry the latest runtime
///   sample alongside its own fields.
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
    /// If you'd rather fold the latest runtime sample into your own metric
    /// structs instead of emitting standalone runtime-metric entries, use
    /// [`embed_tokio_runtime_metrics`](Self::embed_tokio_runtime_metrics).
    ///
    /// # Panics
    ///
    /// Must be called from within a Tokio runtime — the reporter is spawned
    /// via [`tokio::spawn`], which panics if there is no active runtime.
    ///
    /// [`RuntimeMetrics`]: tokio_metrics::RuntimeMetrics
    fn subscribe_tokio_runtime_metrics(config: TokioRuntimeMetricsConfig) {
        let sink = BoxEntrySink::lazy(Self::try_sink);
        let name_style = config.name_style;
        let abort = spawn_runtime_metrics_loop(config.interval, move |snapshot| {
            sink.append(DynamicInflectionEntry {
                entry: snapshot.close(),
                name_style,
            });
        });
        Self::register_shutdown_fn(ShutdownFn::new(move || {
            abort.abort();
        }));
    }

    /// Spawn a runtime-metrics sampler that drives a shared [`State`] for
    /// folding into per-request entries via `#[metrics(flatten)]`.
    ///
    /// The sampler is aborted when the
    /// [`AttachHandle`](metrique::writer::sink::AttachHandle) is dropped,
    /// the same way [`subscribe_tokio_runtime_metrics`](Self::subscribe_tokio_runtime_metrics)
    /// is. After shutdown the returned [`State`] still resolves (to the last
    /// sample stored before the abort), but no longer refreshes.
    ///
    /// Unlike `subscribe_tokio_runtime_metrics`, this does not emit
    /// standalone runtime-metric entries — callers fold the returned
    /// [`State`] into their own entries instead.
    fn embed_tokio_runtime_metrics(
        config: TokioRuntimeMetricsConfig,
    ) -> State<EmbeddedTokioMetrics> {
        let initial = EmbeddedTokioMetrics(Arc::new(RuntimeMetrics::default().close()));
        let state = State::new(initial);
        let task_state = state.clone();
        let abort = spawn_runtime_metrics_loop(config.interval, move |snapshot| {
            task_state.store(Arc::new(EmbeddedTokioMetrics(Arc::new(snapshot.close()))));
        });
        Self::register_shutdown_fn(ShutdownFn::new(move || {
            abort.abort();
        }));
        state
    }
}

impl<T: AttachGlobalEntrySink + 'static> AttachGlobalEntrySinkTokioMetricsExt for T {}

/// Spawn the [`RuntimeMonitor`]-driven sampling loop. Consumes the first
/// sample synchronously and hands it to `on_sample` before returning, so
/// callers (and any `State` they're populating) see real runtime data
/// immediately rather than the all-zero [`RuntimeMetrics::default`].
/// Subsequent samples are handled on a spawned task, with a sibling task
/// logging unexpected panics. Returns the worker's
/// [`AbortHandle`](tokio::task::AbortHandle).
fn spawn_runtime_metrics_loop<F>(interval: Duration, mut on_sample: F) -> tokio::task::AbortHandle
where
    F: FnMut(RuntimeMetrics) + Send + 'static,
{
    let handle = Handle::current();
    let monitor = RuntimeMonitor::new(&handle);
    let mut intervals = monitor.intervals();
    if let Some(first) = intervals.next() {
        on_sample(first);
    }

    let worker = tokio::spawn(async move {
        tracing::debug!("tokio runtime metrics reporter started");
        for snapshot in intervals {
            tokio::time::sleep(interval).await;
            on_sample(snapshot);
        }
        tracing::debug!("tokio runtime metrics reporter stopped");
    });
    let abort = worker.abort_handle();

    tokio::spawn(async move {
        if let Err(err) = worker.await
            && !err.is_cancelled()
        {
            tracing::error!("tokio runtime metrics reporter panicked: {err}");
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
    async fn embedded_folds_latest_sample_into_entry() {
        use metrique::unit_of_work::metrics;
        use metrique_writer::test_util::test_metric;

        use super::EmbeddedTokioMetrics;

        #[metrics(rename_all = "PascalCase")]
        struct RequestMetrics {
            operation: &'static str,
            #[metrics(flatten)]
            runtime: crate::State<EmbeddedTokioMetrics>,
        }

        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        let runtime = Sink::embed_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        // No sleep — the first sample is consumed synchronously by
        // `embed_tokio_runtime_metrics` before it returns, so the state is
        // already populated.
        let entry = test_metric(RequestMetrics {
            operation: "Read",
            runtime: runtime.clone(),
        });

        check!(entry.values["Operation"] == "Read");
        check!(entry.metrics["WorkersCount"] == 1);
    }

    #[tokio::test(start_paused = true)]
    async fn embed_aborted_on_handle_drop() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let handle = Sink::attach((sink, ()));

        let runtime = Sink::embed_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        // Let the sampler tick at least once, then abort it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(handle);
        tokio::time::sleep(Duration::from_millis(300)).await;

        // After abort, fresh snapshots taken over a span longer than the
        // would-be sampling interval must resolve to the same Arc — proving
        // no new sample was stored.
        let a = runtime.clone().snapshot();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let b = runtime.clone().snapshot();
        check!(std::sync::Arc::ptr_eq(&a, &b));
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
