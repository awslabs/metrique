// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{marker::PhantomData, sync::Arc, time::Instant};

use metrique_writer_core::entry::BoxEntry;

use crate::{
    AnyEntrySink, BoxEntrySink, Entry, EntrySink,
    stream::{EntryIoStream, IoStreamError},
};

use super::{
    FlushWait,
    metrics::{
        DescribedMetric, GlobalRecorderVersion, MetricRecorder, MetricsRsType, MetricsRsUnit,
    },
};

/// Builder for [`FlushImmediately`] and [`AnyFlushImmediately`].
#[derive(Default)]
pub struct FlushImmediatelyBuilder {
    metric_name: Option<String>,
    metric_recorder: Option<Box<dyn MetricRecorder>>,
}

impl FlushImmediatelyBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Dimension used for the tracing span and sink metrics emitted.
    pub fn metric_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(!name.is_empty());
        self.metric_name = Some(name);
        self
    }

    /// If provided, metrics to the callback when entries are written.
    #[deprecated = "this function can't be called by users since `MetricRecorder` implementations are private, \
        call metrics_recorder_global or metrics_recorder_local instead"]
    pub fn metric_recorder(mut self, recorder: Option<Box<dyn MetricRecorder>>) -> Self {
        self.metric_recorder = recorder;
        self
    }

    /// Send metrics to the global recorder. Pass `dyn metrics::Recorder` as a type parameter
    /// to allow it to autodetect the right metrics.rs version.
    ///
    /// All metrics are emitted with the dimension `sink` equal to the [Self::metric_name] config.
    ///
    /// The following metrics exist:
    /// 1. `metrique_flush_time_ms` - histogram of flush operation times in milliseconds
    ///
    /// For example (assuming you already have a [`metrics::Recorder`] named `recorder`
    /// and an [`EntryIoStream`] named `stream`).
    ///
    /// [`metrics::Recorder`]: metrics_024::Recorder
    /// ```
    /// # use metrics_024 as metrics;
    /// # use metrics_util_020 as metrics_util;
    /// # use std::sync::{Arc, Mutex};
    /// # use metrique_writer::{Entry, GlobalEntrySink};
    /// # use metrique_writer::sink::{AnyEntrySink, FlushImmediately, FlushImmediatelyBuilder};
    /// # use metrique_writer::sink::{describe_sink_metrics, global_entry_sink};
    /// # use metrique_writer::AttachGlobalEntrySink;
    /// # use metrics_util::debugging::DebugValue;
    /// # let recorder = Arc::new(metrics_util::debugging::DebuggingRecorder::new());
    /// # let recorder_clone = recorder.clone();
    /// # let output: Arc<Mutex<metrique_writer_core::test_stream::TestStream>> = Default::default();
    /// # let stream = Arc::clone(&output);
    ///
    /// global_entry_sink! { ServiceMetrics }
    ///
    /// #[derive(Entry)]
    /// struct MyMetrics {
    ///      value: usize
    /// }
    ///
    /// metrics::set_global_recorder(recorder).unwrap();
    /// describe_sink_metrics::<dyn metrics::Recorder>();
    ///
    /// let sink = FlushImmediatelyBuilder::new()
    ///     .metrics_recorder_global::<dyn metrics::Recorder>()
    ///     .build::<MyMetrics, _>(stream);
    ///
    /// ```
    #[cfg(feature = "metrics-rs-024")]
    #[allow(private_bounds)]
    pub fn metrics_recorder_global<V: super::metrics::GlobalRecorderVersion + ?Sized>(
        mut self,
    ) -> Self {
        self.metric_recorder = Some(Box::new(V::recorder()));
        self
    }

    /// Send metrics to a local metrics recorder. Pass `dyn metrics::Recorder` as the first type parameter
    /// to allow it to autodetect the right metrics.rs version.
    ///
    /// All metrics are emitted with the dimension `sink` equal to the [Self::metric_name] config.
    ///
    /// The following metrics exist:
    /// 1. `metrique_flush_time_ms` - histogram of flush operation times in milliseconds
    ///
    /// For example (assuming you already have a [`metrics::Recorder`] named `recorder`
    /// and an [`EntryIoStream`] named `stream`).
    ///
    /// [`metrics::Recorder`]: metrics_024::Recorder
    /// ```
    /// # use metrics_024 as metrics;
    /// # use metrics_util_020 as metrics_util;
    /// # use std::sync::{Arc, Mutex};
    /// # use metrique_writer::{Entry, GlobalEntrySink};
    /// # use metrique_writer::sink::{AnyEntrySink, FlushImmediately, FlushImmediatelyBuilder};
    /// # use metrique_writer::sink::{describe_immediate_flush_metrics, global_entry_sink};
    /// # use metrique_writer::AttachGlobalEntrySink;
    /// # use metrics_util::debugging::DebugValue;
    /// # let recorder = Arc::new(metrics_util::debugging::DebuggingRecorder::new());
    /// # let recorder_clone = recorder.clone();
    /// # let output: Arc<Mutex<metrique_writer_core::test_stream::TestStream>> = Default::default();
    /// # let stream = Arc::clone(&output);
    ///
    /// global_entry_sink! { ServiceMetrics }
    ///
    /// #[derive(Entry)]
    /// struct MyMetrics {
    ///      value: usize
    /// }
    ///
    /// // describe the metrics that the sink is going to emit so the recorder is aware of them
    /// metrics::with_local_recorder(&recorder, || describe_immediate_flush_metrics::<dyn metrics::Recorder>());
    /// let sink = FlushImmediatelyBuilder::new()
    ///     .metrics_recorder_local::<dyn metrics::Recorder, _>(recorder)
    ///     .build::<MyMetrics, _>(stream);
    /// ```
    #[cfg(feature = "metrics-rs-024")]
    pub fn metrics_recorder_local<V: super::metrics::LocalRecorderVersion<R> + ?Sized, R>(
        mut self,
        recorder: R,
    ) -> Self {
        self.metric_recorder = Some(Box::new(V::recorder(recorder)));
        self
    }

    /// Build a [`FlushImmediately`] for writing metric entries of type `T` to the given stream.
    pub fn build<T: Entry, S: EntryIoStream>(self, stream: S) -> FlushImmediately<T, S> {
        FlushImmediately {
            stream: Arc::new(std::sync::Mutex::new(SinkState {
                stream,
                name: self
                    .metric_name
                    .unwrap_or_else(|| "immediate-flush".to_string()),
                recorder: self.metric_recorder,
            })),
            _phantom: PhantomData,
        }
    }

    /// Build a boxed version of [`FlushImmediately`] for writing metric entries of *any* type that impls [`Entry`].
    ///
    /// This uses dynamic dispatch and will allocate the entries on the heap. If the type of the entries is already
    /// known or can fit inside an enum of cases, prefer [`FlushImmediatelyBuilder::build`] instead.
    pub fn build_boxed(self, stream: impl EntryIoStream + Send + Sync + 'static) -> BoxEntrySink {
        let sink = self.build::<BoxEntry, _>(stream);
        BoxEntrySink::new(sink)
    }

    /// Build an [`AnyFlushImmediately`] for writing metric entries of any type that impls [`Entry`].
    pub fn build_any<S: EntryIoStream>(self, stream: S) -> AnyFlushImmediately<S> {
        AnyFlushImmediately {
            stream: Arc::new(std::sync::Mutex::new(SinkState {
                stream,
                name: self
                    .metric_name
                    .unwrap_or_else(|| "immediate-flush".to_string()),
                recorder: self.metric_recorder,
            })),
        }
    }
}

struct SinkState<S> {
    stream: S,
    name: String,
    recorder: Option<Box<dyn MetricRecorder>>,
}

impl<S: EntryIoStream> SinkState<S> {
    fn append<E: Entry>(&mut self, entry: &E) {
        match self.stream.next(entry) {
            Ok(()) => {}
            Err(IoStreamError::Validation(err)) => {
                tracing::error!(?err, "metric entry couldn't be formatted correctly");
            }
            Err(IoStreamError::Io(err)) => {
                tracing::error!(?err, "couldn't append to metric stream");
            }
        }

        // Flush after each write to ensure entries are written immediately
        self.flush();
    }

    fn flush(&mut self) {
        let start = Instant::now();

        if let Err(err) = self.stream.flush() {
            tracing::warn!(?err, "couldn't flush metric stream");
        }

        // Record flush time metric if recorder is configured
        if let Some(recorder) = &self.recorder {
            let flush_time_ms = start.elapsed().as_millis() as u32;
            recorder.record_histogram("metrique_flush_time_ms", &self.name, flush_time_ms);
        }
    }
}

/// A sink that immediately writes entries to the output stream without buffering.
///
/// Unlike [`BackgroundQueue`](super::BackgroundQueue), this sink doesn't use a background thread
/// or buffer entries. Instead, it immediately writes each entry to the output stream when
/// [`append`](EntrySink::append) is called. This makes it suitable for environments where
/// background threads are not desirable, such as AWS Lambda functions or other short-lived
/// execution environments.
///
/// # Performance Considerations
///
/// This sink is designed for simplicity and reliability, not high performance. Each call to
/// [`append`](EntrySink::append) will block while the entry is written to the output stream.
/// For high-throughput applications, consider using [`BackgroundQueue`](super::BackgroundQueue) instead.
///
/// # Example
///
/// ```
/// use metrique_writer::{
///     Entry, EntrySink,
///     format::{Format, FormatExt},
///     sink::FlushImmediately,
/// };
/// use metrique_writer_format_emf::Emf;
/// use std::io;
///
/// #[derive(Entry)]
/// struct MyMetrics {
///     value: u64,
/// }
///
/// // Create a `FlushImmediately` that writes to stdout (use locking to avoid tearing)
/// let sink = FlushImmediately::new(Emf::all_validations(
///     "MyApp".into(), vec![vec![]]
/// ).output_to_makewriter(|| io::stdout().lock()));
///
/// // Append metrics - this will write immediately to stdout
/// sink.append(MyMetrics { value: 42 });
/// ```
#[derive(Clone)]
pub struct FlushImmediately<T, S> {
    stream: Arc<std::sync::Mutex<SinkState<S>>>,
    _phantom: PhantomData<fn(T)>,
}

impl<T: Entry, S: EntryIoStream> FlushImmediately<T, S> {
    /// Create a `Entry` destination that immediately writes entries to the given stream.
    pub fn new(stream: S) -> Self {
        FlushImmediatelyBuilder::new().build(stream)
    }
}

impl<S: EntryIoStream + Send + Sync + 'static> FlushImmediately<BoxEntry, S> {
    /// Create a new [`FlushImmediately`] that outputs to a given stream,
    /// boxed within a [`BoxEntrySink`].
    pub fn new_boxed(stream: S) -> BoxEntrySink {
        FlushImmediatelyBuilder::new().build_boxed(stream)
    }
}

impl FlushImmediately<(), ()> {
    /// Create a new builder for configuring `FlushImmediately`
    pub fn builder() -> FlushImmediatelyBuilder {
        FlushImmediatelyBuilder::new()
    }
}

impl<T: Entry, S: EntryIoStream> EntrySink<T> for FlushImmediately<T, S> {
    fn append(&self, entry: T) {
        let mut state = self.stream.lock().unwrap();
        state.append(&entry);
    }

    fn flush_async(&self) -> FlushWait {
        // Since we flush after each append, this is a no-op
        FlushWait::ready()
    }
}

/// A [`FlushImmediately`] that can accept entries of any type that implements [`Entry`].
///
/// This uses dynamic dispatch and will allocate the entries on the heap. If the type of the entries is already
/// known, prefer [`FlushImmediately`] instead.
#[derive(Clone)]
pub struct AnyFlushImmediately<S> {
    stream: Arc<std::sync::Mutex<SinkState<S>>>,
}

impl<S: EntryIoStream> AnyFlushImmediately<S> {
    /// Create a new [`FlushImmediately`] that immediately writes entries to the given stream.
    pub fn new(stream: S) -> Self {
        FlushImmediatelyBuilder::new().build_any(stream)
    }

    /// Create a new builder for configuring [`FlushImmediately`].
    pub fn builder() -> FlushImmediatelyBuilder {
        FlushImmediatelyBuilder::new()
    }
}

impl<S: EntryIoStream> AnyEntrySink for AnyFlushImmediately<S> {
    fn append_any(&self, entry: impl Entry + Send + 'static) {
        let mut state = self.stream.lock().unwrap();
        state.append(&entry);
    }

    fn flush_async(&self) -> FlushWait {
        // Since we flush after each append, this is a no-op
        FlushWait::ready()
    }
}

pub const IMMEDIATE_FLUSH_METRICS: &[DescribedMetric] = &[DescribedMetric {
    name: "metrique_flush_time",
    unit: MetricsRsUnit::Millisecond,
    r#type: MetricsRsType::Histogram,
    description: "Percent of time the background sink is idle",
}];

/// Does describe_metrics for this global recorder, which makes your units visible.
/// Call it with a recorder type, to allow it to autodetect your metrics.rs version
///
/// This function should be called once per metric recorder, since some metric
/// recorders are not idempotent in describe. The recorders in [metrique_metricsrs] are
/// however idempotent with describes, so when using that feel free to call this function
/// multiple times.
///
/// [metrique_metricsrs]: https://docs.rs/metrique_metricsrs
///
/// ```no_run
/// # use metrics_024 as metrics;
/// metrique_writer::sink::describe_immediate_flush_metrics::<dyn metrics::Recorder>();
/// ```
#[allow(private_bounds)]
pub fn describe_immediate_flush_metrics<V: GlobalRecorderVersion + ?Sized>() {
    V::describe(IMMEDIATE_FLUSH_METRICS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValidationError;
    use metrique_writer_core::test_stream::{TestEntry, TestStream};
    use std::sync::{Arc, Mutex};

    #[test]
    fn writes_entries_immediately() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = FlushImmediately::<TestEntry, _>::new(Arc::clone(&output));

        sink.append(TestEntry(1));
        assert_eq!(output.lock().unwrap().values, vec![1]);

        sink.append(TestEntry(2));
        assert_eq!(output.lock().unwrap().values, vec![1, 2]);
    }

    #[test]
    fn flushes_after_each_write() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = FlushImmediately::<TestEntry, _>::new(Arc::clone(&output));

        sink.append(TestEntry(1));
        assert_eq!(output.lock().unwrap().flushes, 1);

        sink.append(TestEntry(2));
        assert_eq!(output.lock().unwrap().flushes, 2);
    }

    #[test]
    fn handles_validation_errors() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        output.lock().unwrap().error = Some(IoStreamError::Validation(ValidationError::invalid(
            "test error",
        )));
        let sink = FlushImmediately::<TestEntry, _>::new(Arc::clone(&output));

        // Should not panic
        sink.append(TestEntry(1));

        // Error should be consumed
        assert!(output.lock().unwrap().error.is_none());
    }

    #[test]
    fn handles_io_errors() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        output.lock().unwrap().error = Some(IoStreamError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "test error",
        )));
        let sink = FlushImmediately::<TestEntry, _>::new(Arc::clone(&output));

        // Should not panic
        sink.append(TestEntry(1));

        // Error should be consumed
        assert!(output.lock().unwrap().error.is_none());
    }

    #[test]
    fn any_flush_immediately_works() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = AnyFlushImmediately::new(Arc::clone(&output));

        sink.append_any(TestEntry(1));
        assert_eq!(output.lock().unwrap().values, vec![1]);

        sink.append_any(TestEntry(2));
        assert_eq!(output.lock().unwrap().values, vec![1, 2]);
        assert_eq!(output.lock().unwrap().flushes, 2);
    }

    #[test]
    fn flush_async_returns_ready_future() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = FlushImmediately::<TestEntry, _>::new(Arc::clone(&output));

        let flush_future = sink.flush_async();

        // Use block_on to verify the future is already ready
        futures::executor::block_on(flush_future);
    }

    #[test]
    fn builder_works() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = FlushImmediatelyBuilder::new()
            .metric_name("test-sink")
            .build::<TestEntry, _>(Arc::clone(&output));

        sink.append(TestEntry(1));
        assert_eq!(output.lock().unwrap().values, vec![1]);
    }

    #[test]
    fn builder_any_works() {
        let output: Arc<Mutex<TestStream>> = Default::default();
        let sink = FlushImmediatelyBuilder::new()
            .metric_name("test-sink")
            .build_any(Arc::clone(&output));

        sink.append_any(TestEntry(1));
        assert_eq!(output.lock().unwrap().values, vec![1]);
    }

    #[cfg(feature = "metrics-rs-024")]
    #[test]
    fn metrics_recorder_works() {
        use metrics_024::Recorder;
        use metrics_util_020::debugging::DebuggingRecorder;

        let output: Arc<Mutex<TestStream>> = Default::default();
        let debug_recorder = DebuggingRecorder::new();
        let snapshot = debug_recorder.snapshotter();

        let sink = FlushImmediatelyBuilder::new()
            .metric_name("test-sink")
            .metrics_recorder_local::<dyn Recorder, _>(debug_recorder)
            .build::<TestEntry, _>(Arc::clone(&output));

        sink.append(TestEntry(1));
        sink.append(TestEntry(2));
        let snapshot = snapshot.snapshot().into_vec();
        let (name, _, _, metrics_util_020::debugging::DebugValue::Histogram(value)) = &snapshot[0]
        else {
            panic!("unexpected metrics: {snapshot:#?}")
        };
        assert_eq!(value.len(), 2);
        assert_eq!(name.key().name(), "metrique_flush_time_ms");
    }
}
