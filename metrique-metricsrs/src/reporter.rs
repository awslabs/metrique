// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::pin::{Pin, pin};
use std::time::Duration;

use futures::future::Either;
use futures::future::select;
#[cfg(feature = "background-queue")]
use metrique_writer::sink::BackgroundQueueBuilder;
use metrique_writer::stream::NullEntryIoStream;
use metrique_writer_core::{AnyEntrySink, EntrySink};
use metrique_writer_core::{BoxEntrySink, EntryIoStream};
use tokio::task;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::{MetricRecorder, MetricsRsVersion};

/// A handle to a metric reporter. This struct is mainly used to synchronize shutdown of the metric reporter
/// to ensure all metrics are flushed on shutdown.
///
/// Shutdown only occurs when called explicitly (via the [`shutdown`] function) - dropping the `MetricReporter`
/// will not wait for a flush.
///
/// This may be freely cloned.
///
/// After installation, this will sets up a [`metrics::Recorder`] that will
/// collect metrics and periodically emit them into the output file. The builder
/// can also be used without installation as a local recorder.
///
/// The recorder will work with all metrics reported via the [`metrics::counter!`],
/// [`metrics::gauge!`] and [`metrics::histogram!`] macros.
///
/// # Examples
///
/// **Construct a logger which publishes EMF metrics to a rotating file:**
///
/// ```no_run
/// # use metrics_024 as metrics;
/// use metrique_metricsrs::MetricReporter;
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
///     .metrics_rs_version::<dyn metrics::Recorder>()
///     .metrics_io_stream(Emf::all_validations("MyNS".to_string(),
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
/// **Or via a global sink:**
///
/// ```no_run
/// # use metrics_024 as metrics;
/// use metrique_metricsrs::MetricReporter;
/// use metrique_writer::{Entry, EntryIoStream, FormatExt, EntryIoStreamExt};
/// use metrique_writer::{GlobalEntrySink, AttachGlobalEntrySinkExt};
/// use metrique_writer_format_emf::Emf;
/// use metrique::ServiceMetrics; // or some other GlobalEntrySink
/// use tracing_appender::rolling::{RollingFileAppender, Rotation};
///
/// #[derive(Entry)]
/// #[entry(rename_all = "PascalCase")]
/// struct Globals {
///     service: &'static str,
/// }
///
/// # let log_dir = std::path::PathBuf::from("example");
/// let handle = ServiceMetrics::attach_to_stream(Emf::all_validations("MyNS".to_string(),
///     vec![vec![], vec!["service".to_string()]]).output_to_makewriter(
///           RollingFileAppender::new(Rotation::HOURLY, &log_dir, "metric_log.log")
///     )
///     // you can skip the `merge_globals` call if you don't want to add dimensions
///     .merge_globals(Globals {
///         service: "MyCoolProgram",
///     }));
/// let logger = MetricReporter::builder()
///     .metrics_rs_version::<dyn metrics::Recorder>()
///     // if some other part of your program manages BackgroundQueue shutdown,
///     // you can pass `(ServiceMetrics::sink(), ())` instead
///     // of `(ServiceMetrics::sink(), handle)` - the handle is just
///     // dropped on shutdown to manage shutdown easily.
///     .metrics_sink((ServiceMetrics::sink(), handle))
///     .build_and_install();
///
/// // (You can then use `ServiceMetrics::sink()` for other uses as well, all emissions will go to the same destination)
/// ```
///
/// **Note: It is impossible to construct a `MetricReporter` without a configuring destination for metrics:**:
/// ```compile_fail
/// use metrique_writer::metrics::MetricReporter;
/// let logger = MetricReporter::builder().build();
/// ```
/// This results in a compilation error.
///
/// [`shutdown`]: MetricReporter::shutdown
/// [`metrics::Recorder`]: metrics_024::Recorder
/// [`metrics::counter!`]: metrics_024::counter
/// [`metrics::gauge!`]: metrics_024::gauge
/// [`metrics::histogram!`]: metrics_024::histogram
#[derive(Debug, Clone)]
pub struct MetricReporter {
    tasks: TaskTracker,
    cancellation_token: CancellationToken,
    sink: BoxEntrySink,
}

const DEFAULT_METRICS_PUBLISH_INTERVAL: Duration = Duration::from_secs(60);

/// Creates a task that flushes metrics to the background queue every [`METRICS_PUBLISH_INTERVAL`]. The background queue handles buffering and flushes based on a flush timeout.
///
/// The [`BackgroundQueue`] runs it's own thread to consume from the queue and write data to `destination`
///
/// Each call to `background_queue.append(..)` results in one new record being produced.
fn spawn_metric_reporter<V: MetricsRsVersion + ?Sized>(
    tracker: &TaskTracker,
    destination: BoxEntrySink,
    shutdown_handle: ShutdownHandle,
    publish_interval: Duration,
    shutdown_signal: CancellationToken,
    emit_zero_counters: bool,
) -> MetricRecorder<V> {
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
            destination.append(recorder_.readout())
        }
        // Publish one more time to the background queue during the shutdown process.
        destination.append(recorder_.readout());
        // Shutdown the background publisher for metrics and flush all data to disk.
        match shutdown_handle {
            ShutdownHandle::SyncHandle(shutdown) => {
                if let Err(e) = task::spawn_blocking(shutdown).await {
                    // TODO: recovering the panic message here is not trivial.
                    tracing::error!(
                        "A panic occured while shutting down the background queue: {:?}",
                        e
                    );
                } else {
                    tracing::debug!("Background queue shutdown complete.");
                }
            }
            ShutdownHandle::AsyncHandle(shutdown) => shutdown.await,
        };
    });
    recorder
}

/// Marker type to ensure that a metrics destination is always set.
///
/// You cannot construct a MetricReporter builder without provding a metrics destination.
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct YouMustConfigureAMetricsDestination;

/// Marker type to ensure that a metrics.rs version is always set.
///
/// You cannot construct a MetricReporter builder without provding a metrics.rs version.
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct YouMustConfigureAMetricsRsVersion;

/// Builder for [`MetricReporter`]
///
/// [`MetricReporter`] must be constructed within the context of a Tokio runtime.
pub struct MetricReporterBuilder<
    S = YouMustConfigureAMetricsDestination,
    V: ?Sized = YouMustConfigureAMetricsRsVersion,
> {
    // We need to keep this generic to allow opaque [`Write`] implementations to be stored. It is only possible
    // to invoke build when `W: Write` — the builder starts with `YouMustConfigureAMetricsDestination` (which is _not_ `Write`).
    // This prevents customers from intializing the MetricReporter without metrics destination at compile time.
    metrics_stream: S,
    marker: PhantomData<V>,
    box_entry_sink: Option<(BoxEntrySink, ShutdownHandle)>,
    emit_zero_counters: bool,
    metrics_publish_interval: Duration,
}

enum ShutdownHandle {
    SyncHandle(Box<dyn FnOnce() + Send + Sync>),
    AsyncHandle(Pin<Box<dyn Future<Output = ()> + Send + Sync>>),
}

impl<S> fmt::Debug for MetricReporterBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: CR to `BackgroundQueueBuilder` to make it `Debug`
        f.debug_struct("MetricReporterBuilder").finish()
    }
}

impl Default
    for MetricReporterBuilder<
        YouMustConfigureAMetricsDestination,
        YouMustConfigureAMetricsRsVersion,
    >
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S, V: ?Sized> MetricReporterBuilder<S, V> {
    /// Configure a metrics.rs version
    ///
    /// To ensure metrics are collected from the same metrics.rs version your
    /// code publishes to, you must call this function with
    /// `dyn metrics::Recorder`, as in
    /// ```
    /// # use metrics_024 as metrics;
    /// # use metrique_metricsrs::MetricReporterBuilder;
    ///
    /// let builder = MetricReporterBuilder::new().metrics_rs_version::<dyn metrics::Recorder>();
    /// ```
    pub fn metrics_rs_version<V2: MetricsRsVersion + ?Sized>(self) -> MetricReporterBuilder<S, V2> {
        MetricReporterBuilder {
            metrics_stream: self.metrics_stream,
            marker: PhantomData,
            box_entry_sink: self.box_entry_sink,
            metrics_publish_interval: self.metrics_publish_interval,
            emit_zero_counters: self.emit_zero_counters,
        }
    }
}

impl MetricReporterBuilder<YouMustConfigureAMetricsDestination, YouMustConfigureAMetricsRsVersion> {
    /// Initialize the builder.
    ///
    /// You must call [Self::metrics_rs_version] and one of the functions that configures a metric
    /// destination to actually use it.
    pub fn new() -> Self {
        Self {
            metrics_stream: YouMustConfigureAMetricsDestination,
            marker: PhantomData,
            box_entry_sink: None,
            metrics_publish_interval: DEFAULT_METRICS_PUBLISH_INTERVAL,
            emit_zero_counters: false,
        }
    }
}

impl<V: ?Sized> MetricReporterBuilder<YouMustConfigureAMetricsDestination, V> {
    /// Write metrics to an [`EntryIoStream`]
    ///
    /// For production, you normally use an EMF emitter backed by a [`tracing_appender::rolling::RollingFileAppender`]. See the
    /// examples on [`MetricReporterBuilder`]. Internally, this will connect your provided stream to a [`BackgroundQueue`].
    ///
    /// For testing, either:
    /// - Do not use a `MetricReporterBuilder`, just use use [`capture_metrics`] to capture metrics without installing a global recorder.
    /// - Use [Self::metrics_sink] to create a reporter that points to a [`TestEntrySink`], to write records to an in memory buffer.
    ///
    /// [`capture_metrics`]: crate::capture::capture_metrics
    /// [`TestEntrySink`]: metrique_writer::test_util::TestEntrySink
    /// [`MetricReporterBuilder`]: crate::reporter::MetricReporterBuilder
    /// [`BackgroundQueue`]: metrique_writer::sink::BackgroundQueue
    pub fn metrics_io_stream<S: EntryIoStream + Send + 'static>(
        self,
        stream: S,
    ) -> MetricReporterBuilder<S, V> {
        MetricReporterBuilder {
            metrics_stream: stream,
            marker: PhantomData,
            box_entry_sink: self.box_entry_sink,
            metrics_publish_interval: self.metrics_publish_interval,
            emit_zero_counters: self.emit_zero_counters,
        }
    }

    /// Write metrics to an [`AnyEntrySink`]
    ///
    /// This API is setup so that you can use it directly in the output of the attach function of a global entry queue
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use metrics_024 as metrics;
    /// use metrique_writer::{
    ///    Entry,
    ///    BoxEntry,
    ///    AnyEntrySink,
    ///    format::FormatExt as _,
    ///    sink::BackgroundQueueBuilder,
    ///    unit::AsCount,
    /// };
    /// use metrique_metricsrs::MetricReporter;
    /// use metrique_writer_format_emf::Emf;
    /// use tracing_appender::rolling::{RollingFileAppender, Rotation};
    /// let sink = BackgroundQueueBuilder::new().build_boxed(
    ///    Emf::all_validations("MyApp".into(), vec![vec![]])
    ///        .output_to_makewriter(
    ///            RollingFileAppender::new(Rotation::HOURLY, "my/logs", "prefix.log")
    ///        )
    ///    );
    /// let reporter = MetricReporter::builder().metrics_sink(sink).metrics_rs_version::<dyn metrics::Recorder>()
    ///     .build_and_install();
    /// ```
    pub fn metrics_sink(
        self,
        sink: (
            impl AnyEntrySink + Send + Sync + 'static,
            impl Any + Send + Sync,
        ),
    ) -> MetricReporterBuilder<NullEntryIoStream, V> {
        let (sink, handle) = sink;
        let shutdown_handle = ShutdownHandle::SyncHandle(Box::new(move || {
            let _ = handle;
        }));
        MetricReporterBuilder {
            metrics_stream: NullEntryIoStream::default(),
            marker: PhantomData,
            box_entry_sink: Some((sink.boxed(), shutdown_handle)),
            emit_zero_counters: self.emit_zero_counters,
            metrics_publish_interval: self.metrics_publish_interval,
        }
    }

    /// Write metrics to a sink that must be shutdown asynchronously
    pub fn metrics_sink_async_shutdown(
        self,
        sink: impl AnyEntrySink + Send + Sync + 'static,
        shutdown: impl Future<Output = ()> + Send + Sync + 'static,
    ) -> MetricReporterBuilder<NullEntryIoStream, V> {
        let shutdown_handle = ShutdownHandle::AsyncHandle(Box::pin(async {
            shutdown.await;
        }));
        MetricReporterBuilder {
            metrics_stream: NullEntryIoStream::default(),
            marker: PhantomData,
            box_entry_sink: Some((sink.boxed(), shutdown_handle)),
            emit_zero_counters: self.emit_zero_counters,
            metrics_publish_interval: self.metrics_publish_interval,
        }
    }

    /// Creates a metric emitter that drops all metrics. Potentially useful for testing.
    pub fn disable_metrics(self) -> MetricReporterBuilder<NullEntryIoStream, V> {
        self.metrics_io_stream(NullEntryIoStream::default())
    }
}

impl<S> MetricReporterBuilder<S> {
    /// If true, counter metrics that have a value of 0 will be emitted. If false (default), they will be skipped.
    /// This does not affect gauges or histograms, which are always emitted.
    pub fn emit_zero_counters(mut self, emit_zero_counters: bool) -> Self {
        self.emit_zero_counters = emit_zero_counters;
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

impl<S: EntryIoStream + Send + 'static, V: MetricsRsVersion + ?Sized> MetricReporterBuilder<S, V> {
    /// Builds a MetricReporter and installs its recorder as the global recorder.
    ///
    /// Use the returned MetricReporter to synchronize shutdown.
    #[track_caller]
    pub fn build_and_install(self) -> MetricReporter {
        let (res, accumulator) = MetricReporter::new(self);
        V::set_global_recorder(accumulator);
        res
    }

    /// Builds a MetricReporter and returns it along with the associated reference to the recorder
    /// which can be manually used. Metrics recorded to the recorder will be reported
    /// both periodically and when shutting down the MetricReporter.
    pub fn build_without_installing(self) -> (MetricReporter, MetricRecorder<V>) {
        let (reporter, recorder) = MetricReporter::new(self);
        (reporter, recorder)
    }
}

impl MetricReporter {
    /// Creates a new MetricReporter.
    ///
    /// `access_background_queue` is a hook for unit tests to take a copy of the
    /// background queue to allow them to flush it when needed.
    fn new<V: MetricsRsVersion + ?Sized>(
        builder: MetricReporterBuilder<impl EntryIoStream + Send + 'static, V>,
    ) -> (Self, MetricRecorder<V>) {
        let tracker = TaskTracker::new();
        let cancellation = CancellationToken::new();

        let metrics_token = cancellation.clone();
        let (sink, handle) = match builder.box_entry_sink {
            Some((sink, handle)) => (sink, handle),
            #[cfg(not(feature = "background-queue"))]
            None => panic!("setting a queue is required without background-queue enabled"),
            #[cfg(feature = "background-queue")]
            None => {
                let (sink, handle) =
                    BackgroundQueueBuilder::default().build_boxed(builder.metrics_stream);
                (
                    sink,
                    ShutdownHandle::SyncHandle(Box::new(move || {
                        tracing::debug!("shutting down the background queue");
                        handle.shut_down();
                    })),
                )
            }
        };

        let recorder = spawn_metric_reporter(
            &tracker,
            sink.clone(),
            handle,
            builder.metrics_publish_interval,
            metrics_token,
            builder.emit_zero_counters,
        );
        tracker.close();

        (
            Self {
                tasks: tracker,
                cancellation_token: cancellation.clone(),
                sink,
            },
            recorder,
        )
    }

    /// Shuts down the MetricReporter, requesting a metrics flush and waiting for it to complete.
    pub async fn shutdown(&self) {
        self.cancellation_token.cancel();
        self.tasks.wait().await
    }

    /// Flush all outstanding metrics to the configured storage
    pub async fn flush(&self) {
        AnyEntrySink::flush_async(&self.sink).await
    }

    /// Creates a [builder](crate::MetricReporterBuilder) for [`MetricReporter`]
    pub fn builder() -> MetricReporterBuilder {
        MetricReporterBuilder::new()
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use metrique_writer_core::test_stream::{DummyFormat, TestSink};
    use rstest::rstest;

    use crate::{MetricReporter, MetricReporterBuilder};
    use metrique_writer::{
        FormatExt,
        test_util::{TestEntrySink, test_entry_sink},
    };

    // not using BackgroundQueue here since it uses the real-time clock.
    #[rstest]
    #[case(true)]
    #[case(false)]
    #[tokio::test(start_paused = true)]
    async fn test_spawn_metric_recorder(#[case] emit_zero_counters: bool) {
        let sink = TestSink::default();
        let writer = DummyFormat.output_to(sink.clone());
        let builder = MetricReporterBuilder::new()
            .emit_zero_counters(emit_zero_counters)
            .metrics_publish_interval(Duration::from_secs(60))
            .metrics_io_stream(writer)
            .metrics_rs_version::<dyn metrics_024::Recorder>();
        let (reporter, recorder) = MetricReporter::new(builder);
        metrics_024::with_local_recorder(&recorder, || {
            metrics_024::counter!("counter_1").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        reporter.flush().await;
        let d = sink.take_string();
        assert!(d.contains(r#"("counter_1", "[Unsigned(1)] None []")"#));
        metrics_024::with_local_recorder(&recorder, || {
            metrics_024::counter!("counter_1").increment(0);
            metrics_024::counter!("counter_2").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        reporter.flush().await;
        let d = sink.take_string();
        if emit_zero_counters {
            assert!(d.contains(r#"("counter_1", "[Unsigned(0)] None []")"#));
        } else {
            assert!(!d.contains("counter_1"));
        }
        assert!(d.contains(r#"("counter_2", "[Unsigned(1)] None []")"#));
        reporter.shutdown().await;
    }

    struct TestHandle {
        shutdown_called: Arc<AtomicBool>,
        async_shutdown_called: Arc<AtomicBool>,
    }

    impl TestHandle {
        fn new() -> (Self, Arc<AtomicBool>, Arc<AtomicBool>) {
            let drop_shutdown: Arc<AtomicBool> = Default::default();
            let async_shutdown: Arc<AtomicBool> = Default::default();
            (
                Self {
                    shutdown_called: drop_shutdown.clone(),
                    async_shutdown_called: async_shutdown.clone(),
                },
                drop_shutdown,
                async_shutdown,
            )
        }

        pub async fn shutdown(&mut self) {
            self.async_shutdown_called
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl Drop for TestHandle {
        fn drop(&mut self) {
            eprintln!("dropped");
            self.shutdown_called
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[tokio::test(start_paused = true)]
    /// Tests metrics when using `metrics_sink`
    async fn test_metrics_sink() {
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let (shutdown_hook, shutdown, async_shutdown) = TestHandle::new();
        let builder = MetricReporterBuilder::new()
            .metrics_publish_interval(Duration::from_secs(60))
            .metrics_sink((sink, shutdown_hook))
            .metrics_rs_version::<dyn metrics_024::Recorder>();
        let (reporter, recorder) = MetricReporter::new(builder);
        metrics_024::with_local_recorder(&recorder, || {
            metrics_024::counter!("counter_1").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        reporter.flush().await;
        let entries = inspector.entries();
        assert_eq!(inspector.entries().len(), 1);
        assert_eq!(entries[0].metrics["counter_1"], 1);
        metrics_024::with_local_recorder(&recorder, || {
            metrics_024::counter!("counter_1").increment(0);
            metrics_024::counter!("counter_2").increment(1);
        });
        tokio::time::sleep(Duration::from_secs(65)).await;
        reporter.flush().await;
        let entries = inspector.entries();
        assert_eq!(inspector.entries().len(), 2);
        assert_eq!(entries[1].metrics["counter_2"], 1);
        // counter_1 is 0
        assert_eq!(entries[1].metrics.contains_key("counter_1"), false);
        reporter.shutdown().await;
        assert_eq!(shutdown.load(std::sync::atomic::Ordering::Relaxed), true);
        assert_eq!(
            async_shutdown.load(std::sync::atomic::Ordering::Relaxed),
            false
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_with_async_shutdown() {
        let TestEntrySink { sink, .. } = test_entry_sink();
        let (mut shutdown_hook, shutdown, async_shutdown) = TestHandle::new();
        let builder = MetricReporterBuilder::new()
            .metrics_publish_interval(Duration::from_secs(60))
            .metrics_sink_async_shutdown(sink, async move { shutdown_hook.shutdown().await })
            .metrics_rs_version::<dyn metrics_024::Recorder>();
        let (reporter, _recorder) = MetricReporter::new(builder);
        assert_eq!(shutdown.load(std::sync::atomic::Ordering::Relaxed), false);
        assert_eq!(
            async_shutdown.load(std::sync::atomic::Ordering::Relaxed),
            false
        );
        reporter.shutdown().await;
        // this isn't done explicitly, just a side effect of dropping the handle.
        assert_eq!(shutdown.load(std::sync::atomic::Ordering::Relaxed), true);
        assert_eq!(
            async_shutdown.load(std::sync::atomic::Ordering::Relaxed),
            true
        );
    }
}
