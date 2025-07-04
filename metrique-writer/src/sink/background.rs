// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_queue::ArrayQueue;
use crossbeam_utils::sync::{Parker, Unparker};
use metrique_writer_core::{BoxEntrySink, EntryIoStream, IoStreamError, sink::FlushWait};

use crate::{Entry, EntrySink, rate_limit::rate_limited};

use super::metrics::{
    DescribedMetric, GlobalRecorderVersion, LocalRecorderVersion, MetricRecorder, MetricsRsType,
    MetricsRsUnit,
};

/// Builder for [`BackgroundQueue`]
pub struct BackgroundQueueBuilder {
    capacity: usize,
    thread_name: String,
    metric_name: Option<String>,
    metric_recorder: Option<Box<dyn MetricRecorder + Send>>,
    flush_interval: Duration,
    shutdown_timeout: Duration,
}

impl Default for BackgroundQueueBuilder {
    fn default() -> Self {
        Self {
            capacity: 64 * 1024,
            thread_name: "metric-background-queue".into(),
            metric_name: None,
            metric_recorder: None,
            flush_interval: Duration::from_secs(1),
            shutdown_timeout: Duration::from_secs(30),
        }
    }
}

/// Contains metadata for the BackgroundQueue metrics emitted by this crate, for implementing
/// your custom describe function.
pub const BACKGROUND_QUEUE_METRICS: &[DescribedMetric] = &[
    DescribedMetric {
        name: "metrique_idle_percent",
        unit: MetricsRsUnit::Percent,
        r#type: MetricsRsType::Histogram,
        description: "Percent of time the background queue is idle",
    },
    DescribedMetric {
        name: "metrique_queue_len",
        unit: MetricsRsUnit::Count,
        r#type: MetricsRsType::Histogram,
        description: "Length of the background queue",
    },
    DescribedMetric {
        name: "metrique_metrics_emitted",
        unit: MetricsRsUnit::Count,
        r#type: MetricsRsType::Counter,
        description: "Number of metrics emitted from this queue",
    },
    DescribedMetric {
        name: "metrique_io_errors",
        unit: MetricsRsUnit::Count,
        r#type: MetricsRsType::Counter,
        description: "Number of IO errors when emitting from this queue",
    },
    DescribedMetric {
        name: "metrique_validation_errors",
        unit: MetricsRsUnit::Count,
        r#type: MetricsRsType::Counter,
        description: "Number of metric validation errors when emitting from this queue",
    },
];

impl BackgroundQueueBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the number of entries that can be stored in the background queue before older entries start being dropped.
    ///
    /// Defaults to `64*1024`. Note that this is the *number* of entries, not their byte size when written.
    ///
    /// Higher capacity values give greater tolerance for transient writer slowdowns but comes at the cost of higher
    /// memory consumption. It also won't help if entries are being appended faster than the writer can consume them on
    /// average.
    ///
    /// Note that we deliberately drop the oldest entries on hitting capacity. We almost always care more about the most
    /// recent metrics as they're more reflective of the system state. See the [`crate`] documentation.
    ///
    /// A [`tracing`] error will be emitted periodically if metrics are being dropped.
    pub fn capacity(mut self, capacity: usize) -> Self {
        assert!(capacity > 0);
        self.capacity = capacity;
        self
    }

    /// Thread name assigned to the background thread that reads from the queue.
    pub fn thread_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(!name.is_empty());
        self.thread_name = name;
        self
    }

    /// If true, the background queue will emit metrics to the callback
    ///
    /// All metrics are emitted with the dimension `queue` equal to the [Self::metric_name] config.
    ///
    /// The following metrics exist:
    /// 1. `metrique_idle_percent` - the percentage of time that the background queue is idle.
    /// 2. `metrique_queue_len` - the measured length of the background queue.
    /// 3. `metrique_metrics_emitted` - the count of metrics emitted.
    /// 4. `metrique_io_errors` - the amount of IO errors encountered emitting metrics.
    /// 5. `metrique_validation_errors` - the amount of validation errors encountered emitting metrics.
    ///
    /// To avoid breakage, this function intentionally does not depend on metrics-rs. To allow for
    /// bridging, you can use the [BackgroundQueueBuilder::metrics_recorder_global] or
    /// [BackgroundQueueBuilder::metrics_recorder_local] functions, which are what most
    /// customers are expected to use.
    ///
    /// This function does not assign units to metrics, since there are often race conditions as the
    /// metric recorder can be set after the background queue. You can use [`describe_sink_metrics`]
    /// or [`BACKGROUND_QUEUE_METRICS`] to do that.
    ///
    /// For the metrics.rs recorder, you can use something like [crate::metrics] to emit these metrics via
    /// a Metrique sink, or of course any other metrics.rs backend.
    pub fn metric_recorder(mut self, recorder: Option<Box<dyn MetricRecorder + Send>>) -> Self {
        self.metric_recorder = recorder;
        self
    }

    /// Send metrics to the global recorder. Pass `dyn metrics::Recorder` as a type parameter
    /// to allow it to autodetect the right metrics.rs version.
    ///
    /// For example (assuming you already have a [`metrics::Recorder`] named `recorder`
    /// and an [`EntryIoStream`] named `stream`).
    /// ```
    /// # use std::sync::{Arc, Mutex};
    /// # use metrique_writer::{AnyEntrySink, Entry, GlobalEntrySink};
    /// # use metrique_writer::sink::{BackgroundQueue, BackgroundQueueBuilder};
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
    /// let _handle = ServiceMetrics::attach(BackgroundQueueBuilder::new()
    ///     .metrics_recorder_global::<dyn metrics::Recorder>()
    ///     .build(stream));
    ///
    /// let metric_base = MyMetrics { value: 0 };
    /// let mut metric = ServiceMetrics::append_on_drop(metric_base);
    /// # drop(metric);
    /// # futures::executor::block_on(ServiceMetrics::sink().flush_async());
    /// # assert_eq!(output.lock().unwrap().values, vec![0]);
    /// # assert!(recorder_clone.snapshotter().snapshot().into_vec().iter().any(|(k, u, d, v)| {
    /// #     k.key().name() == "metrique_metrics_emitted" && *v == DebugValue::Counter(1) &&
    /// #     u.is_some() && d.is_some()
    /// # }))
    /// ```
    #[allow(private_bounds)]
    pub fn metrics_recorder_global<V: GlobalRecorderVersion + ?Sized>(self) -> Self {
        self.metric_recorder(Some(Box::new(V::recorder())))
    }

    /// Send metrics to a local metrics recorder. Pass `dyn metrics::Recorder` as the first type parameter
    /// to allow it to autodetect the right metrics.rs version.
    ///
    /// For example (assuming you already have a [`metrics::Recorder`] named `recorder`
    /// and an [`EntryIoStream`] named `stream`).
    /// ```
    /// # use std::sync::{Arc, Mutex};
    /// # use metrique_writer::{Entry, GlobalEntrySink};
    /// # use metrique_writer::sink::{BackgroundQueue, BackgroundQueueBuilder};
    /// # use metrique_writer::sink::{describe_sink_metrics, global_entry_sink};
    /// # use metrique_writer::{AnyEntrySink, AttachGlobalEntrySink};
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
    /// metrics::with_local_recorder(&recorder, || describe_sink_metrics::<dyn metrics::Recorder>());
    ///
    /// let _handle = ServiceMetrics::attach(BackgroundQueueBuilder::new()
    ///     .metrics_recorder_local::<dyn metrics::Recorder, _>(recorder)
    ///     .build(stream));
    ///
    /// let metric_base = MyMetrics { value: 0 };
    /// let mut metric = ServiceMetrics::append_on_drop(metric_base);
    /// # drop(metric);
    /// # futures::executor::block_on(ServiceMetrics::sink().flush_async());
    /// # assert_eq!(output.lock().unwrap().values, vec![0]);
    /// # assert!(recorder_clone.snapshotter().snapshot().into_vec().iter().any(|(k, u, d, v)| {
    /// #     k.key().name() == "metrique_metrics_emitted" && *v == DebugValue::Counter(1) &&
    /// #     u.is_some() && d.is_some()
    /// # }))
    /// ```
    #[allow(private_bounds)]
    pub fn metrics_recorder_local<V: LocalRecorderVersion<R> + ?Sized, R>(
        self,
        recorder: R,
    ) -> Self {
        self.metric_recorder(Some(Box::new(V::recorder(recorder))))
    }

    /// Dimension used for the tracing span and queue metrics emitted. Defaults to the thread name.
    pub fn metric_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(!name.is_empty());
        self.metric_name = Some(name);
        self
    }

    /// Sets approximately how frequently the writer is flushed.
    ///
    /// Defaults to every second.
    ///
    /// The writer will always be subject to periodic flushing, even if no data is being written. This prevents entries
    /// from sitting in a file buffer for a long time and potentially being counted against the wrong time period.
    ///
    /// Setting a smaller interval will ensure the output closely tracks the entries already appended, but typically
    /// comes at a higher IO cost.
    ///
    /// The interval can't be greater than a minute, as that is very likely to cause entries to be counted against the
    /// wrong time period.
    pub fn flush_interval(mut self, flush_interval: Duration) -> Self {
        assert!(
            Duration::ZERO < flush_interval && flush_interval < Duration::from_secs(60),
            "flush_interval must be in the range (0, 1 minute), not {flush_interval:?}"
        );
        self.flush_interval = flush_interval;
        self
    }

    /// Sets how long the background thread will try to drain remaining metric entries once starting to shut down.
    ///
    /// Defaults to 30 seconds.
    ///
    /// A longer timeout will give the thread more time to drain entries, which is especially helpful when there is a
    /// a high throughput or a low IO throughput. The downside is this will cause service shutdown to take longer. Some
    /// process managers may kill a service that doesn't quicky shut down after receiving a termination signal.
    pub fn shutdown_timeout(mut self, shutdown_timeout: Duration) -> Self {
        assert!(
            shutdown_timeout > Duration::ZERO,
            "shutdown_timeout must not be zero"
        );
        self.shutdown_timeout = shutdown_timeout;
        self
    }

    /// Build a [`BackgroundQueue`] for writing metric entries of type `T` to the given stream.
    ///
    /// Returns both the queue and a [`BackgroundQueueJoinHandle`] that can be used to cleanly flush all remaining
    /// queue entries during service shutdown.
    ///
    /// This is the right mode for when all of the metric entries for the output stream can be described by a single
    /// type. Note that an enum can be used to allow for multiple kinds of entries even within the single type
    /// restriction! If a more flexible queue is needed, use [`BackgroundQueueBuilder::build_any`] instead.
    pub fn build<T: Entry + Send + 'static>(
        self,
        stream: impl EntryIoStream + Send + 'static,
    ) -> (BackgroundQueue<T>, BackgroundQueueJoinHandle) {
        let (inner, handle) = self.do_build(stream);
        (BackgroundQueue(inner), handle)
    }

    /// Build a background [`BoxEntrySink`] for writing metric entries of *any* type that impls [`Entry`].
    ///
    /// This uses dynamic dispatch and will allocate the entries on the heap. If the type of the entries is already
    /// known or can fit inside an enum of cases, prefer [`BackgroundQueueBuilder::build`] instead.
    pub fn build_boxed(
        self,
        stream: impl EntryIoStream + Send + 'static,
    ) -> (BoxEntrySink, BackgroundQueueJoinHandle) {
        let (queue, handle) = self.build(stream);
        (BoxEntrySink::new(queue), handle)
    }

    fn do_build<S: EntryIoStream + Send + 'static, E: Entry + Send + 'static>(
        self,
        stream: S,
    ) -> (Arc<Inner<E>>, BackgroundQueueJoinHandle) {
        let parker = Parker::default();
        let unparker = parker.unparker().clone();
        let (flush_queue_sender, flush_queue_receiver) = std::sync::mpsc::channel();
        let inner = Arc::new(Inner {
            queue: ArrayQueue::new(self.capacity),
            unparker: unparker.clone(),
            flush_queue_sender,
        });
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        let receiver = Receiver {
            metrics_emitted: 0,
            metric_validation_errors: 0,
            metric_io_errors: 0,
            name: self.metric_name.unwrap_or_else(|| self.thread_name.clone()),
            stream,
            recorder: self.metric_recorder,
            inner: Arc::clone(&inner),
            flush_interval: self.flush_interval,
            shutdown_timeout: self.shutdown_timeout,
            shutdown_signal: Arc::clone(&shutdown_signal),
            parker,
        };

        let handle = thread::Builder::new()
            .name(self.thread_name)
            .spawn(move || receiver.run(flush_queue_receiver))
            .unwrap();

        (
            inner,
            BackgroundQueueJoinHandle {
                handle: Some(handle),
                shutdown_signal,
                unparker,
            },
        )
    }
}

/// An [`EntrySink`] implementation for entries of type `T`.
///
/// Entries are appended to a shared queue that's drained by a background thread. See [`BackgroundQueueBuilder::build`].
/// Cloning is cheap and still appends to the same shared queue.
///
/// Emits [`tracing`] errors periodically if a [`IoStreamError`] occurs, but doesn't stop writing.
pub struct BackgroundQueue<T>(Arc<Inner<T>>);

impl<T: Entry + Send + 'static> BackgroundQueue<T> {
    /// Create a new background queue using the [`BackgroundQueueBuilder`] defaults.
    pub fn new(stream: impl EntryIoStream + Send + 'static) -> (Self, BackgroundQueueJoinHandle) {
        BackgroundQueueBuilder::new().build(stream)
    }
}
struct FlushSignal {
    // drop the sender to signal that the message has been flushed
    #[allow(unused)]
    channel: tokio::sync::oneshot::Sender<()>,
}

struct Inner<E> {
    // Note we use crossbeam's ArrayQueue rather than std::sync::mpsc because we want ring buffer behavior. That is, the
    // oldest entries should be dropped when the queue is full.
    queue: ArrayQueue<E>,
    // queue for flush wakers. This is not the fast-path so it does not use a ring buffer
    flush_queue_sender: std::sync::mpsc::Sender<FlushSignal>,
    // The unparker allows appending threads to cheaply wake up the background writing thread
    unparker: Unparker,
}

/// Guard handle that, when dropped, will block until all already appended entries are written to the output stream.
///
/// This ensures that all metric entries are written from the buffered background queue during service shutdown.
pub struct BackgroundQueueJoinHandle {
    handle: Option<thread::JoinHandle<()>>,
    shutdown_signal: Arc<AtomicBool>,
    unparker: Unparker,
}

impl<T> Clone for BackgroundQueue<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Entry + Send + 'static> EntrySink<T> for BackgroundQueue<T> {
    fn append(&self, entry: T) {
        self.0.push(entry)
    }

    fn flush_async(&self) -> FlushWait {
        self.0.flush_async()
    }
}

impl BackgroundQueueJoinHandle {
    /// Drop the handle but also let the background thread keep running until no [`BackgroundQueue`]s exist.
    pub fn forget(mut self) {
        self.handle = None;
    }

    /// Alias for `drop(handle)`. Causes the background thread to try to flush all remaining queued entries and then
    /// stop. Will try to flush for a maximum of 5 minutes before giving up.
    pub fn shut_down(self) {}
}

impl Drop for BackgroundQueueJoinHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.shutdown_signal.store(true, Ordering::Relaxed);
            self.unparker.unpark();
            tracing::info!("awaiting background metrics queue shutdown");
            handle.join().unwrap();
            tracing::info!("background metrics queue shut down");
        }
    }
}

impl<E> Inner<E> {
    fn push(&self, entry: E) {
        // force_push causes the oldest entry to be dropped if the queue is full. We want this since the more recent
        // metrics are more valuable when describing the state of the service!
        if self.queue.force_push(entry).is_some() {
            rate_limited!(
                Duration::from_secs(1),
                tracing::error!(
                    "background metric queue has fallen behind, metrics will be missing"
                )
            );
        }
        // Note that we're not enormously concerned about the ordering guarantees between the queue push and the unpark
        // signal. That's because the writer thread will at most wait for flush_interval before waking itself up.
        self.unparker.unpark();
    }

    fn flush_async(&self) -> FlushWait {
        let (channel, receiver) = tokio::sync::oneshot::channel();
        self.flush_queue_sender.send(FlushSignal { channel }).ok();
        self.unparker.unpark();
        FlushWait::from_future(async move {
            let _ = receiver.await;
        })
    }
}

// Background thread struct that receives entries from the shared queue.
struct Receiver<S, E> {
    name: String,
    metrics_emitted: u64,
    metric_validation_errors: u64,
    metric_io_errors: u64,
    stream: S,
    inner: Arc<Inner<E>>,
    flush_interval: Duration,
    shutdown_timeout: Duration,
    shutdown_signal: Arc<AtomicBool>,
    recorder: Option<Box<dyn MetricRecorder + Send>>,
    // Utility to notice wakeup events when an appender thread has appended something to the queue.
    parker: Parker,
}

// A struct for tracking the waking of flush wakers
//
// Safety invariant:
// S1. If an entry has been pushed into the queue, and a waker has been afterwards pushed
// into the flush_queue, then the entry will have been popped from the queue and flush
// called before the waker will wake.
// S2. [busy-loop freedom] if will_progress_on_drained_queue is true, then calling handle_waiting_wakers
// with DrainResult::Drained will make progress by "counting against" a waker sent to flush_queue_receiver,
// avoiding busy-loops [unless someone is putting entries into flush_queue_receiver infinitely often].
// Liveness invariant:
// L1. If handle_waiting_wakers is called, and then either will_progress_on_drained_queue returns false, or
// handle_waiting_wakers is called sufficiently many (the bound is `queue_capacity`) times [calling with
// DrainResult::HitDeadline and entry_count == 0 doesn't happen in the real world and does not count], then all wakers
// that were sent before the first call to handle_waiting_wakers have been woken.
//
// WakerTracker does not access the queue directly but rather uses queue_capacity to ensure the invariants,
// instead you have these conditions:
// P1. if status == DrainResult::Drained, then the queue has been empty at least once since the last
// call to handle_waiting_wakers.
// P2. `queue_capacity` is a function that returns the number of entries that once they
// are processed, it's guaranteed that all entries currently in the queue have been
//  processed. We use the queue's capacity, since it is guaranteed that all entries
// currently in the queue have been popped queue after capacity entries have been
// popped (.len() would work here as well, but len of a queue is a non-standard function).

struct WakerTracker {
    waiting_wakers: Vec<FlushSignal>,
    entries_before_wake: usize,
    flush_queue_receiver: std::sync::mpsc::Receiver<FlushSignal>,
}

impl WakerTracker {
    fn new(flush_queue_receiver: std::sync::mpsc::Receiver<FlushSignal>) -> Self {
        WakerTracker {
            waiting_wakers: vec![],
            entries_before_wake: 0,
            flush_queue_receiver,
        }
    }

    /// Handle waiting wakers (for flush signalling) for processing (status, count) entries
    ///
    /// If there are wakers that need to be waken, this will flush the stream and wake them up.
    ///
    //
    // The "liveness" goal I am trying to maintain is to ensure that wakers will be woken
    // finishes after a bounded number of pops, and .capacity() is a bounded
    // number of pops.
    fn handle_waiting_wakers(
        &mut self,
        queue_capacity: impl FnOnce() -> usize,
        flush_stream: impl FnOnce(),
        status: DrainResult,
        entry_count: usize,
    ) {
        if !self.waiting_wakers.is_empty() {
            self.entries_before_wake = self.entries_before_wake.saturating_sub(entry_count);
            // if all entries in the queue have been flushed, or the queue is empty, wake the wakers.
            if self.entries_before_wake == 0 || status == DrainResult::Drained {
                tracing::debug!("flushing metrics stream");
                flush_stream();
                self.entries_before_wake = 0;
                self.waiting_wakers.clear();
            }
        }
        // We can get to this `if` either if there are no wakers initially, or if the wakers
        // have just been woken up.
        if self.waiting_wakers.is_empty() {
            while let Ok(entry) = self.flush_queue_receiver.try_recv() {
                // move all flush wakers from the receiver to the queue
                self.waiting_wakers.push(entry);
            }

            if !self.waiting_wakers.is_empty() {
                self.entries_before_wake = queue_capacity();
            }
        }
    }

    // If this returns true, then calling handle_waiting_wakers with a drained queue will make progress,
    // so it does not make sense to sleep [also, "will make progress" means that this won't cause
    // a busy wait]
    //
    // this function's result can't change concurrently [i.e. except by calling handle_waiting_wakers]
    fn will_progress_on_drained_queue(&mut self) -> bool {
        !self.waiting_wakers.is_empty()
    }
}

impl<S: EntryIoStream, E: Entry> Receiver<S, E> {
    fn run(mut self, flush_queue_receiver: std::sync::mpsc::Receiver<FlushSignal>) {
        let span =
            tracing::span!(tracing::Level::TRACE, "metrics background queue", sink=?self.name);
        let _enter = span.enter();
        let mut waker_tracker = WakerTracker::new(flush_queue_receiver);
        let inner = self.inner.clone();

        loop {
            let next_flush = Instant::now() + self.flush_interval;
            let loop_start: Instant = Instant::now();
            let mut idle_duration = Duration::ZERO;
            loop {
                let (status, entry_count) = self.drain_until_deadline(next_flush);

                waker_tracker.handle_waiting_wakers(
                    || inner.queue.capacity(),
                    || self.flush_stream(),
                    status,
                    entry_count,
                );

                if status == DrainResult::HitDeadline {
                    break; // Hit deadline, flush stream
                }

                if self.shutdown_signal.load(Ordering::Relaxed) {
                    break; // shut down, break out of loop to have a chance to flush stream
                }

                // if the waker tracker can make progress observing an empty queue, let it
                if !waker_tracker.will_progress_on_drained_queue() {
                    let park_start = Instant::now();
                    self.parker.park_deadline(next_flush);
                    if self.recorder.is_some() {
                        idle_duration += park_start.elapsed();
                    }
                }

                // If we did make it to the next flush deadline, flush, else someone woke us up and we'll continue
                // writing.
                if Instant::now() >= next_flush {
                    break;
                }
            }

            self.flush_stream();
            if let Some(recorder) = &self.recorder {
                let queue_len = self.inner.queue.len().try_into().unwrap_or(u32::MAX);
                let total_duration = loop_start.elapsed();
                let idle_percent: u32 = idle_duration
                    .as_micros()
                    .saturating_mul(100)
                    .checked_div(total_duration.as_micros())
                    .unwrap_or(100)
                    .try_into()
                    .unwrap_or(100);
                recorder.record_histogram("metrique_idle_percent", &self.name, idle_percent);
                recorder.record_histogram("metrique_queue_len", &self.name, queue_len);
            }
            if self.shutdown_signal.load(Ordering::Relaxed) {
                tracing::info!("caught shutdown signal, shutting down background metrics queue");
                return self.shut_down();
            }
            if Arc::get_mut(&mut self.inner).is_some() {
                tracing::info!("no appenders left, shutting down background metrics queue");
                return self.shut_down();
            }
        }
        // Wakers will wake up when we exit from this function
    }

    fn drain_until_deadline(&mut self, deadline: Instant) -> (DrainResult, usize) {
        // Most write() activites consume < 1us. We don't need to recheck the timeline after every write to still keep
        // a reasonably accurate flush interval. Instead, we'll check the clock every 32 entries if we're still seeing
        // entries remaining in the queue.
        let mut count = 0;
        while let Some(entry) = self.inner.queue.pop() {
            self.consume(entry);

            count += 1;
            if count % 32 == 0 && Instant::now() >= deadline {
                return (DrainResult::HitDeadline, count);
            }
        }

        (DrainResult::Drained, count)
    }

    fn consume(&mut self, entry: E) {
        match self.stream.next(&entry) {
            Ok(()) => {
                self.metrics_emitted += 1;
            }
            Err(IoStreamError::Validation(err)) => {
                self.metric_validation_errors += 1;
                rate_limited!(
                    Duration::from_secs(1),
                    tracing::error!(?err, "metric entry couldn't be formatted correctly")
                )
            }
            Err(IoStreamError::Io(err)) => {
                self.metric_io_errors += 1;
                rate_limited!(
                    Duration::from_secs(1),
                    tracing::error!(?err, "couldn't append to metric stream")
                )
            }
        }
    }

    fn flush_stream(&mut self) {
        if let Err(err) = self.stream.flush() {
            self.metric_io_errors += 1;
            rate_limited!(
                Duration::from_secs(1),
                tracing::warn!(?err, "couldn't flush metric stream")
            )
        }

        if let Some(recorder) = &self.recorder {
            // intentionally use the metric macros here, so if a new global recorder is
            // installed after the background queue is created, [most] metrics won't be lost
            //
            // this is a bit racy because the first flush can always be lost, but life's life
            // [yes, this allocates, but it's only done once every X seconds, when flushing]
            recorder.increment_counter(
                "metrique_metrics_emitted",
                &self.name,
                std::mem::take(&mut self.metrics_emitted),
            );
            recorder.increment_counter(
                "metrique_io_errors",
                &self.name,
                std::mem::take(&mut self.metric_io_errors),
            );
            recorder.increment_counter(
                "metrique_validation_errors",
                &self.name,
                std::mem::take(&mut self.metric_validation_errors),
            );
        }
    }

    fn shut_down(mut self) {
        let deadline = Instant::now() + self.shutdown_timeout;
        let (status, _count) = self.drain_until_deadline(deadline);
        if status == DrainResult::HitDeadline {
            tracing::warn!("unable to drain metrics queue while shutting down");
        }
        self.flush_stream();
        drop(self.stream); // Close the file before we report we're done!
        tracing::info!("background metric log writing has shut down");
    }
}

/// Does describe_metrics for this global recorder, which makes your units visible.
/// Call it with a recorder type, to allow it to autodetect your metrics.rs version
///
/// This function should be called once per metric recorder, since some metric
/// recorders are not idempotent in describe. Rust-MetricExperimental is however
/// idempotent with describes, so when using that feel free to call this function multiple times.
///
/// ```no_run
/// metrique_writer::sink::describe_sink_metrics::<dyn metrics::Recorder>();
/// ```
#[allow(private_bounds)]
pub fn describe_sink_metrics<V: GlobalRecorderVersion + ?Sized>() {
    V::describe(BACKGROUND_QUEUE_METRICS);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrainResult {
    Drained,     // no entries left in the queue
    HitDeadline, // some entries left, but we're now past the deadline
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::Mutex,
        task::{Poll, Wake},
    };

    use crate::{EntrySink, ValidationError};
    use metrique_writer_core::test_stream::{TestEntry, TestStream};

    use super::*;

    // unfortunately, this needs to be a macro because we can't write a fn
    // generic over both BackgroundQueue and the boxed BackgroundQueue
    macro_rules! test_all_queues {
        (|$builder:ident| $with_builder:expr, |$output:ident, $queue:ident, $handle:ident| $test:expr) => {
            let $builder = BackgroundQueueBuilder::new().flush_interval(Duration::from_micros(1));
            let builder = $with_builder;
            let $output: Arc<Mutex<TestStream>> = Default::default();
            let ($queue, $handle) = builder.build(Arc::clone(&$output));
            $test;

            let $builder = BackgroundQueueBuilder::new().flush_interval(Duration::from_micros(1));
            let builder = $with_builder;
            let $output: Arc<Mutex<TestStream>> = Default::default();
            let ($queue, $handle) = builder.build_boxed(Arc::clone(&$output));
            $test;
        };
    }

    #[test]
    fn writes_all_entries_in_fifo_for_single_thread() {
        test_all_queues! {
            |builder| builder.capacity(1_000),
            |output, queue, handle| {
                for i in 0..1_000 {
                    queue.append(TestEntry(i));
                }
                handle.shut_down();
                assert_eq!(output.lock().unwrap().values, (0..1_000).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn drops_older_entries_when_full() {
        test_all_queues! {
            |builder| builder.capacity(10),
            |output, queue, handle| {
                // hold lock so writer can't make progress
                {
                    let _locked = output.lock().unwrap();
                    for i in 0..20 {
                        queue.append(TestEntry(i));
                    }
                }
                // lock released, should drain now
                handle.shut_down();

                // note we can't directly check output == 10..20 because the background queue can pick up one entry in
                // the range 0..10 before getting blocked on the mutex. It must contain all of 10..20, though.
                let output = output.lock().unwrap();
                assert!((10..=11).contains(&output.values.len()));
                assert!((10..20).all(|i| output.values.contains(&i)));
            }
        }
    }

    #[test]
    fn writes_all_entries_from_multiple_threads() {
        test_all_queues! {
            |builder| builder.capacity(1_000),
            |output, queue, handle| {
                std::thread::scope(|scope| {
                    for t in 0..100 {
                        let queue = queue.clone();
                        scope.spawn(move || {
                            for i in 0..10 {
                                queue.append(TestEntry(t*10 + i));
                            }
                        });
                    }
                });
                handle.shut_down();
                let values = &mut output.lock().unwrap().values;
                values.sort();
                assert_eq!(*values, (0..1_000).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn allows_stream_errors() {
        test_all_queues! {
            |builder| builder.capacity(1_000),
            |output, queue, handle| {
                output.lock().unwrap().error = Some(IoStreamError::Validation(ValidationError::invalid("some problem")));
                for i in 0..1_000 {
                    queue.append(TestEntry(i));
                }
                handle.shut_down();
                let output = output.lock().unwrap();
                assert_eq!(output.values, (0..1_000).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn shut_down_stops_new_entries_from_being_appended() {
        test_all_queues! {
            |builder| builder.capacity(1_000),
            |output, queue, handle| {
                for i in 0..500 {
                    queue.append(TestEntry(i));
                }
                handle.shut_down();
                for i in 500..1_000 {
                    queue.append(TestEntry(i));
                }

                let output = output.lock().unwrap();
                assert_eq!(output.values, (0..500).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn forget_doesnt_stop_new_entries_from_being_appended() {
        test_all_queues! {
            |builder| builder.capacity(1_000),
            |output, queue, handle| {
                for i in 0..500 {
                    queue.append(TestEntry(i));
                }
                handle.forget();
                for i in 500..1_000 {
                    queue.append(TestEntry(i));
                }

                // may now need to wait for a while since we don't have the shut_down() sync point
                let start = Instant::now();
                loop {
                    if output.lock().unwrap().values == (0..1_000).collect::<Vec<_>>() {
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(1));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("didn't finish writing");
                    }
                }
            }
        }
    }

    #[test]
    fn flushes_periodically_even_when_not_writing() {
        test_all_queues! {
            |builder| builder.capacity(100),
            |output, queue, _handle| {
                // flushes after some data written
                queue.append(TestEntry(0));
                let flushes = output.lock().unwrap().flushes;
                let start = Instant::now();
                loop {
                    if output.lock().unwrap().flushes > flushes {
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(1));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }
                }

                // flushes even when nothing written recently
                let flushes = output.lock().unwrap().flushes;
                let start = Instant::now();
                loop {
                    if output.lock().unwrap().flushes > flushes {
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(1));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }
                }
            }
        }
    }

    #[test]
    fn flushes_periodically_when_writing() {
        test_all_queues! {
            |builder| builder.capacity(100),
            |output, queue, _handle| {
                let start = Instant::now();
                // flushes after some data written
                let _fuel_guard = output.lock().unwrap().set_up_fuel(0);

                loop {
                    queue.append(TestEntry(0));
                    queue.append(TestEntry(1));
                    output.lock().unwrap().fuel.as_ref().unwrap().fetch_add(1, Ordering::SeqCst);

                    if output.lock().unwrap().flushes > 10 {
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(1));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("only flushed {} times", output.lock().unwrap().flushes);
                    }
                }
            }
        }
    }

    // Implement a simple waker to avoid taking a dependency on tokio rt
    #[derive(Default)]
    struct SimpleWaker(AtomicBool);

    impl SimpleWaker {
        fn is_awake(&self) -> bool {
            self.0.swap(false, Ordering::SeqCst)
        }
    }

    impl Wake for SimpleWaker {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn flush_simple() {
        // this test also tests the metric recorder, since I'll rather not duplicate it
        #[cfg(feature = "metrics_rs_024")]
        let mut recorder = None;
        test_all_queues! {
            |builder| (|builder: BackgroundQueueBuilder| {
                #[allow(unused_mut)]
                let mut builder = builder.capacity(10);
                #[cfg(feature = "metrics_rs_024")]
                {
                    recorder = Some(Arc::new(metrics_util::debugging::DebuggingRecorder::new()));
                    metrics::with_local_recorder(recorder.as_ref().unwrap(), || describe_sink_metrics::<dyn metrics::Recorder>());
                    builder = builder.metrics_recorder_local::<dyn metrics::Recorder, _>(recorder.clone().unwrap()).metric_name("my_queue");
                }
                builder
            })(builder),
            |output, queue, handle| {
                // flushes after some data written
                let _fuel_guard = output.lock().unwrap().set_up_fuel(0);
                queue.append(TestEntry(1));
                queue.append(TestEntry(2));
                queue.append(TestEntry(10));
                // no values can be flushed since no fuel was provided
                assert_eq!(output.lock().unwrap().values_flushed, 0);
                let mut flush = EntrySink::<TestEntry>::flush_async(&queue);
                let waker = Arc::new(SimpleWaker::default());
                let waker2 = waker.clone().into();
                let mut cx = std::task::Context::from_waker(&waker2);
                waker.wake_by_ref();
                let start = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_micros(1));
                    output.lock().unwrap().fuel.as_ref().unwrap().fetch_add(1, Ordering::SeqCst);

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }

                    if waker.is_awake() {
                        if let Poll::Ready(()) = Pin::new(&mut flush).poll(&mut cx) {
                            break
                        }
                    }
                }
                assert!(output.lock().unwrap().flushes > 0);
                assert!(output.lock().unwrap().values_flushed > 0);
                assert_eq!(output.lock().unwrap().values, vec![1u64, 2, 10]);
                handle.shut_down();
                #[cfg(feature = "metrics_rs_024")]
                {
                    use metrics_util::MetricKind::*;
                    use metrics_util::debugging::DebugValue;
                    use metrics::Unit;
                    let snapshot = recorder.as_ref().unwrap().snapshotter().snapshot().into_hashmap();
                    let key = |kind, name: &'static str| {
                        metrics_util::CompositeKey::new(kind, metrics::Key::from_static_parts(name, const { &[metrics::Label::from_static_parts("sink", "my_queue")] }))
                    };
                    assert_eq!(
                        snapshot[&key(Counter, "metrique_metrics_emitted")],
                        (Some(Unit::Count), Some(metrics::SharedString::from("Number of metrics emitted from this queue")), DebugValue::Counter(3)),
                    );
                    assert_eq!(
                        snapshot[&key(Counter, "metrique_io_errors")],
                        (Some(Unit::Count), Some(metrics::SharedString::from("Number of IO errors when emitting from this queue")), DebugValue::Counter(0)),
                    );
                    assert_eq!(
                        snapshot[&key(Counter, "metrique_validation_errors")],
                        (Some(Unit::Count), Some(metrics::SharedString::from("Number of metric validation errors when emitting from this queue")), DebugValue::Counter(0)),
                    );
                    let (idle_percent_unit, idle_percent_desc, idle_percent_hist) = &snapshot[&key(Histogram, "metrique_idle_percent")];
                    assert_eq!(*idle_percent_unit, Some(Unit::Percent));
                    assert!(idle_percent_desc.is_some());
                    match idle_percent_hist {
                        DebugValue::Histogram(hist) => {
                            assert!(hist.len() > 0);
                            assert!(hist.iter().all(|v| v.0 >= 0.0 && v.0 <= 100.0));
                        }
                        bad => panic!("bad value {:?}", bad),
                    }

                    let (queue_length_unit, queue_length_desc, queue_length_hist) = &snapshot[&key(Histogram, "metrique_queue_len")];
                    assert_eq!(*queue_length_unit, Some(Unit::Count));
                    assert!(queue_length_desc.is_some());
                    match queue_length_hist {
                        DebugValue::Histogram(hist) => {
                            assert!(hist.len() > 0);
                            assert!(hist.iter().all(|v| v.0 >= 0.0));
                        }
                        bad => panic!("bad value {:?}", bad),
                    }
                }
            }
        }
    }

    #[test]
    fn flush_never_empty() {
        #[cfg(feature = "metrics_rs_024")]
        let mut recorder = None;
        const QUEUE_SIZE: usize = 100;
        test_all_queues! {
            |builder| (|builder: BackgroundQueueBuilder| {
                #[allow(unused_mut)]
                let mut builder = builder.capacity(QUEUE_SIZE);
                #[cfg(feature = "metrics_rs_024")]
                {
                    recorder = Some(Arc::new(metrics_util::debugging::DebuggingRecorder::new()));
                    builder = builder.metrics_recorder_local::<dyn metrics::Recorder, _>(recorder.clone().unwrap()).metric_name("my_queue");
                }
                builder
            })(builder),
            |output, queue, handle| {
                // flushes after some data written
                let fuel_guard = output.lock().unwrap().set_up_fuel(0);
                queue.append(TestEntry(1));
                queue.append(TestEntry(2));
                queue.append(TestEntry(3));
                queue.append(TestEntry(4));
                queue.append(TestEntry(5));
                // no values can be flushed since no fuel was provided
                assert_eq!(output.lock().unwrap().values_flushed, 0);
                let mut flush = EntrySink::<TestEntry>::flush_async(&queue);
                let waker = Arc::new(SimpleWaker::default());
                let waker2 = waker.clone().into();
                let mut cx = std::task::Context::from_waker(&waker2);
                waker.wake_by_ref();
                let start = Instant::now();
                let mut i = 0;
                let mut other_flush = None;
                let check_metrics = || {
                    #[cfg(feature = "metrics_rs_024")]
                    {
                        use metrics_util::MetricKind::Histogram;
                        // in theory the queue reaches the size of QUEUE_SIZE in all cases, but I am a little bit afraid
                        // of race conditions that make it not monitorined at its largest causing flakiness.
                        let mut have_queue_size_at_least_half = false;
                        let snapshot = recorder.as_ref().unwrap().snapshotter().snapshot().into_hashmap();
                        let key = |kind, name: &'static str| {
                            metrics_util::CompositeKey::new(kind, metrics::Key::from_static_parts(name, const { &[metrics::Label::from_static_parts("sink", "my_queue")] }))
                        };
                        // [avoid flakiness due to missing metrics because of tests]
                        let (_, _, metrics_util::debugging::DebugValue::Histogram(queue_len)) = &snapshot[&key(Histogram, "metrique_queue_len")] else {
                            panic!("bad queue len")
                        };
                        if queue_len.iter().any(|q| q.0 > ((QUEUE_SIZE as f64) / 2.0)) {
                            have_queue_size_at_least_half = true;
                        }
                        for entry in queue_len {
                            if !(entry.0 <= QUEUE_SIZE as f64) {
                                panic!("queue len contains over-long entry {entry}");
                            }
                        }
                        have_queue_size_at_least_half
                    }
                    #[cfg(not(feature = "metrics_rs_024"))]
                    true
                };
                loop {
                    std::thread::sleep(Duration::from_micros(100));

                    // add more entries than fuel to ensure the queue never empties, but flush
                    // must finish.
                    //
                    // but, don't put more than 10 entries in the queue unless the initial
                    // inputs have been written to avoid overflowing the queue.
                    if i < 10 || output.lock().unwrap().values.len() >= 5 {
                        output.lock().unwrap().fuel.as_ref().unwrap().fetch_add(1, Ordering::SeqCst);
                        queue.append(TestEntry(6));
                        queue.append(TestEntry(7));
                        i += 1;
                        if i == 2 {
                            other_flush = Some(EntrySink::<TestEntry>::flush_async(&queue));
                        }
                    }

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }

                    if waker.is_awake() {
                        if let Poll::Ready(()) = Pin::new(&mut flush).poll(&mut cx) {
                            break
                        }
                    }
                }
                assert!(output.lock().unwrap().flushes > 0);
                assert!(output.lock().unwrap().values_flushed > 0);
                assert_eq!(&output.lock().unwrap().values[..5], [1u64, 2, 3, 4, 5]);

                let mut other_flush = other_flush.unwrap();
                waker.wake_by_ref();
                loop {
                    std::thread::sleep(Duration::from_micros(100));

                    // add more entries than fuel to ensure the queue never empties, but flush
                    // must finish.
                    output.lock().unwrap().fuel.as_ref().unwrap().fetch_add(1, Ordering::SeqCst);
                    queue.append(TestEntry(4));
                    queue.append(TestEntry(5));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }

                    if waker.is_awake() {
                        if let Poll::Ready(()) = Pin::new(&mut other_flush).poll(&mut cx) {
                            break
                        }
                    }
                }

                if !check_metrics() {
                    panic!("queue was never full somehow");
                }

                // add fuel to enable orderly shutdown
                drop(fuel_guard);
                handle.shut_down();
            }
        }
    }

    #[test]
    fn flush_without_waiting() {
        test_all_queues! {
            // weird closure to get the macro to work
            |builder| (|builder: BackgroundQueueBuilder| {
                let mut builder = builder.capacity(10);
                builder.flush_interval = Duration::from_secs(100_000);
                builder
            })(builder),
            |output, queue, handle| {
                queue.append(TestEntry(1));
                // this sleep will make the flush normally run while the background queue is parked, which
                // is where a bug previously existed.
                // since the goal is to trigger a race rather than to avoid triggering it, I'm fine with a sleep.
                std::thread::sleep(std::time::Duration::from_millis(10));
                let mut flush = EntrySink::<TestEntry>::flush_async(&queue);
                let waker = Arc::new(SimpleWaker::default());
                let waker2 = waker.clone().into();
                let mut cx = std::task::Context::from_waker(&waker2);
                waker.wake_by_ref();
                let start = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_micros(1));

                    if start.elapsed() > Duration::from_secs(60) {
                        panic!("never flushed");
                    }

                    if waker.is_awake() {
                        if let Poll::Ready(()) = Pin::new(&mut flush).poll(&mut cx) {
                            break
                        }
                    }
                }
                assert!(output.lock().unwrap().flushes > 0);
                assert_eq!(output.lock().unwrap().values, vec![1u64]);

                handle.shut_down();
            }
        }
    }
}
