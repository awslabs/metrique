// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fmt;
use std::pin::pin;
use std::time::Duration;

use crate::sink::{BackgroundQueue, BackgroundQueueBuilder};
use crate::stream::NullEntryIoStream;
use futures::future::Either;
use futures::future::select;
use metrique_writer_core::EntryIoStream;
use metrique_writer_core::EntrySink;
use tokio::task;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::metrics::MetricAccumulatorEntry;
use crate::metrics::MetricRecorder;

/// A handle to a metric reporter. This struct is mainly used to synchronize shutdown of the metric reporter
/// to ensure all metrics are flushed on shutdown.
///
/// The main documentation for metric reports is at [`MetricReporterBuilder`].
///
/// Shutdown only occurs when called explicitly (via the [`shutdown`] function) - dropping the `MetricReporter`
/// will not wait for a flush.
///
/// [`shutdown`]: MetricReporter::shutdown
///
/// This may be freely cloned.
#[derive(Debug, Clone)]
pub struct MetricReporter {
    tasks: TaskTracker,
    cancellation_token: CancellationToken,
}

const DEFAULT_METRICS_PUBLISH_INTERVAL: Duration = Duration::from_secs(60);

/// Creates a task that flushes metrics to the background queue every [`METRICS_PUBLISH_INTERVAL`]. The background queue handles buffering and flushes based on a flush timeout.
///
/// The [`BackgroundQueue`] runs it's own thread to consume from the queue and write data to `destination`
///
/// Each call to `background_queue.append(..)` results in one new record being produced.
fn spawn_metric_reporter(
    tracker: &TaskTracker,
    destination: impl EntryIoStream + Send + 'static,
    publish_interval: Duration,
    shutdown_signal: CancellationToken,
    background_queue_builder: Option<BackgroundQueueBuilder>,
    emit_zero_counters: bool,
) -> (MetricRecorder, BackgroundQueue<MetricAccumulatorEntry>) {
    let (background_queue, background_queue_shutdown_hook) = background_queue_builder
        .unwrap_or_default()
        .build(destination);
    let background_queue_clone = background_queue.clone();
    let recorder = MetricRecorder::new_with_emit_zero_counters(emit_zero_counters);
    let recorder_ = recorder.clone();
    tracker.spawn(async move {
        let next_metrics_publish = || tokio::time::sleep(publish_interval);
        let shutdown_initiated = || shutdown_signal.cancelled();
        // We want to wait for either:
        // 1. `METRICS_PUBLISH_INTERVAL` to complete
        // 2. The shutdown sequence to start.
        // If the shutdown sequence is starting `Either::Right` is returned and we exit the loop.
        while let Either::Left(_time_interval_ticked) =
            select(pin!(next_metrics_publish()), pin!(shutdown_initiated())).await
        {
            tracing::trace!("publishing metrics to background queue");
            background_queue.append(recorder_.readout())
        }
        // Publish one more time to the background queue during the shutdown process.
        background_queue.append(recorder_.readout());
        // Shutdown the background publisher for metrics and flush all data to disk.
        if let Err(e) = task::spawn_blocking(|| background_queue_shutdown_hook.shut_down()).await {
            // TODO: recovering the panic message here is not trivial.
            tracing::error!(
                "A panic occured while shutting down the background queue: {:?}",
                e
            );
        } else {
            tracing::debug!("Background queue shutdown complete.");
        }
    });
    (recorder, background_queue_clone)
}

/// Marker type to ensure that a metrics destination is always set.
///
/// You cannot construct a MetricReporter builder without provding a metrics destination.
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct YouMustConfigureAMetricsDestination;

/// Builder for [`MetricReporter`]
///
/// [`MetricReporter`] must be constructed within the context of a Tokio runtime.
///
/// After installation, this builder set ups a [`metrics::Recorder`] that will
/// collect metrics and periodically emit them into the output file. The builder
/// can also be used without installation as a local recorder.
///
/// The recorder will work with all metrics reported via the `metrics::counter!`,
/// `metrics::gauge!` and `metrics::histogram!` macros, both directly-called
/// and metrics generated using the [`metrics`](`crate::metrics`) macro in this crate.
///
/// # Examples
///
/// **Construct a logger which publishes EMF metrics to a rotating file**
/// ```no_run
/// use metrique_writer::metrics::MetricReporter;
/// use metrique_writer::{Entry, EntryIoStream, FormatExt, EntryIoStreamExt};
/// use metrique_writer_format_emf::Emf;
/// use tracing_appender::rolling::{RollingFileAppender, Rotation};
///
/// #[derive(Entry)]
/// #[entry(rename_all = "PascalCase")]
/// struct Globals {
///     service: &'static str,
/// }
///
/// # let log_dir = std::path::PathBuf::from("example");
/// let logger = MetricReporter::builder()
///     .metrics_sink(Emf::all_validations("MyNS".to_string(),
///                   vec![vec![], vec!["service".to_string()]]).output_to_makewriter(
///                         RollingFileAppender::new(Rotation::HOURLY, &log_dir, "metric_log.log")
///                   )
///            // you can skip the `merge_globals` call if you don't want to add dimensions
///            .merge_globals(Globals {
///                service: "MyCoolProgram",
///            })
///     )
///     .build_and_install();
/// ```
///
/// **Note: It is impossible to construct a `MetricReporter` without a configuring destination for metrics:**:
/// ```compile_fail
/// use metrique_writer::metrics::MetricReporter;
/// let logger = MetricReporter::builder().build();
/// ```
/// This results in a compilation error.
pub struct MetricReporterBuilder<S = YouMustConfigureAMetricsDestination> {
    // We need to keep this generic to allow opaque [`Write`] implementations to be stored. It is only possible
    // to invoke build when `W: Write` — the builder starts with `YouMustConfigureAMetricsDestination` (which is _not_ `Write`).
    // This prevents customers from intializing the MetricReporter without metrics destination at compile time.
    metrics_sink: S,
    background_queue_builder: Option<BackgroundQueueBuilder>,
    emit_zero_counters: bool,
    metrics_publish_interval: Duration,
}

impl<S> fmt::Debug for MetricReporterBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: CR to `BackgroundQueueBuilder` to make it `Debug`
        f.debug_struct("MetricReporterBuilder").finish()
    }
}

impl Default for MetricReporterBuilder<YouMustConfigureAMetricsDestination> {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricReporterBuilder<YouMustConfigureAMetricsDestination> {
    /// Initialize the builder.
    pub fn new() -> Self {
        Self {
            metrics_sink: YouMustConfigureAMetricsDestination,
            background_queue_builder: None,
            metrics_publish_interval: DEFAULT_METRICS_PUBLISH_INTERVAL,
            emit_zero_counters: false,
        }
    }

    /// Sets the destination for emitted metrics
    ///
    /// For production, you normally use an EMF emitter backed by a [`tracing_appender::rolling::RollingFileAppender`]. See the
    /// examples on [`MetricReporterBuilder`].
    ///
    /// For testing, either:
    /// - Do not use a `MetricReporterBuilder`, just use use [`capture_metrics`] to capture metrics without installing a global recorder.
    /// - Provide a metric writer that points to a [`TestSink`], to write records to an in memory buffer.
    ///
    /// [`capture_metrics`]: crate::capture::capture_metrics
    /// [`TestSink`]: crate::util::TestSink
    /// [`MetricReporterBuilder`]: crate::reporter::MetricReporterBuilder
    pub fn metrics_sink<S: EntryIoStream + Send + 'static>(
        self,
        sink: S,
    ) -> MetricReporterBuilder<S> {
        MetricReporterBuilder {
            metrics_sink: sink,
            background_queue_builder: self.background_queue_builder,
            metrics_publish_interval: self.metrics_publish_interval,
            emit_zero_counters: self.emit_zero_counters,
        }
    }

    /// Creates a metric emitter that drops all metrics. Potentially useful for testing.
    pub fn disable_metrics(self) -> MetricReporterBuilder<NullEntryIoStream> {
        self.metrics_sink(NullEntryIoStream::default())
    }
}

impl<S> MetricReporterBuilder<S> {
    /// If true, counter metrics that have a value of 0 will be emitted. If false (default), they will be skipped.
    /// This does not affect gauges or histograms, which are always emitted.
    pub fn emit_zero_counters(mut self, emit_zero_counters: bool) -> Self {
        self.emit_zero_counters = emit_zero_counters;
        self
    }

    /// Override the [`BackgroundQueueBuilder`] used to write metrics
    ///
    /// [`BackgroundQueueBuilder`]: crate::sink::BackgroundQueueBuilder
    pub fn background_queue_builder(
        mut self,
        background_queue_builder: BackgroundQueueBuilder,
    ) -> Self {
        self.background_queue_builder = Some(background_queue_builder);
        self
    }

    /// Change the publishing interval for metrics
    ///
    /// This value defaults to 60 seconds.
    pub fn metrics_publish_interval(mut self, duration: Duration) -> Self {
        self.metrics_publish_interval = duration;
        self
    }
}

impl<S: EntryIoStream + Send + 'static> MetricReporterBuilder<S> {
    /// Builds a MetricReporter and installs its recorder as the global recorder.
    ///
    /// Use the returned MetricReporter to synchronize shutdown.
    #[track_caller]
    pub fn build_and_install(self) -> MetricReporter {
        let (res, accumulator, _) = MetricReporter::new(self);
        metrics::set_global_recorder(accumulator).expect("failed to set global recorder");
        res
    }

    /// Builds a MetricReporter and returns it along with the associated reference to the recorder
    /// which can be manually used. Metrics recorded to the recorder will be reported
    /// both periodically and when shutting down the MetricReporter.
    pub fn build_without_installing(self) -> (MetricReporter, MetricRecorder) {
        let (reporter, recorder, _) = MetricReporter::new(self);
        (reporter, recorder)
    }
}

impl MetricReporter {
    /// Creates a new MetricReporter.
    ///
    /// `access_background_queue` is a hook for unit tests to take a copy of the
    /// background queue to allow them to flush it when needed.
    fn new(
        builder: MetricReporterBuilder<impl EntryIoStream + Send + 'static>,
    ) -> (
        Self,
        MetricRecorder,
        BackgroundQueue<MetricAccumulatorEntry>,
    ) {
        let tracker = TaskTracker::new();
        let cancellation = CancellationToken::new();

        let metrics_token = cancellation.clone();

        let (recorder, queue) = spawn_metric_reporter(
            &tracker,
            builder.metrics_sink,
            builder.metrics_publish_interval,
            metrics_token,
            builder.background_queue_builder,
            builder.emit_zero_counters,
        );
        tracker.close();

        (
            Self {
                tasks: tracker,
                cancellation_token: cancellation.clone(),
            },
            recorder,
            queue,
        )
    }

    /// Shuts down the MetricReporter, requesting a metrics flush and waiting for it to complete.
    pub async fn shutdown(&self) {
        self.cancellation_token.cancel();
        self.tasks.wait().await
    }

    /// Creates a [builder](crate::MetricReporterBuilder) for [`MetricReporter`]
    pub fn builder() -> MetricReporterBuilder {
        MetricReporterBuilder::new()
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use metrique_writer_core::{
        EntrySink,
        test_stream::{DummyFormat, TestSink},
    };
    use test_case::test_case;

    use crate::{
        FormatExt,
        metrics::{MetricReporter, MetricReporterBuilder},
    };

    // not using BackgroundQueue here since it uses the real-time clock.
    #[test_case(true; "emit_zero_counters")]
    #[test_case(false; "no_emit_zero_counters")]
    #[tokio::test(start_paused = true)]
    async fn test_spawn_metric_recorder(emit_zero_counters: bool) {
        let sink = TestSink::default();
        let writer = DummyFormat.output_to(sink.clone());
        let builder = MetricReporterBuilder::new()
            .emit_zero_counters(emit_zero_counters)
            .metrics_publish_interval(Duration::from_secs(60))
            .metrics_sink(writer);
        let (reporter, recorder, bg_queue) = MetricReporter::new(builder);
        metrics::with_local_recorder(&recorder, || {
            metrics::counter!("counter_1").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        bg_queue.flush_async().await;
        let d = sink.take_string();
        assert!(d.contains(r#"("counter_1", "[Unsigned(1)] None []")"#));
        metrics::with_local_recorder(&recorder, || {
            metrics::counter!("counter_1").increment(0);
            metrics::counter!("counter_2").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        bg_queue.flush_async().await;
        let d = sink.take_string();
        if emit_zero_counters {
            assert!(d.contains(r#"("counter_1", "[Unsigned(0)] None []")"#));
        } else {
            assert!(!d.contains("counter_1"));
        }
        assert!(d.contains(r#"("counter_2", "[Unsigned(1)] None []")"#));
        reporter.shutdown().await;
    }
}
