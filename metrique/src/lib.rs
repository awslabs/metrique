// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
// not bumping the MSRV for collapsible_if
#![allow(clippy::collapsible_if)]

pub mod emf;
pub mod flex;
pub mod instrument;
mod keep_alive;

/// Provides timing utilities for metrics, including timestamps and duration measurements.
///
/// This module contains types for recording timestamps and measuring durations:
/// - `Timestamp`: Records a point in time, typically when an event occurs
/// - `TimestampOnClose`: Records the time when a metric record is closed
/// - `Timer`: Automatically starts timing when created and stops when dropped
/// - `Stopwatch`: Manually controlled timer that must be explicitly started
///
/// # Examples
///
/// Using a Timer:
/// ```
/// # use metrique::timers::Timer;
/// #
/// let mut timer = Timer::start_now();
/// // Do some work...
/// let elapsed = timer.stop();
/// ```
///
/// Using a Timestamp:
/// ```
/// # use metrique::timers::Timestamp;
/// #
/// let timestamp = Timestamp::now();
/// ```
pub mod timers;

/// [`Slot`] lets you split off a section of your metrics to be handled by another task
///
/// It is often cumbersome to maintain a reference to the root metrics entry if your handling work in a separate tokio Task or thread. `Slot` provides primitives to
/// handle that work in the background.
pub mod slot;

use metrique_core::CloseEntry;
use metrique_writer_core::Entry;
use metrique_writer_core::EntryWriter;
use metrique_writer_core::entry::SampleGroupElement;
pub use slot::{FlushGuard, ForceFlushGuard, LazySlot, OnParentDrop, Slot, SlotGuard};

pub use flex::Flex;

use core::ops::Deref;
use core::ops::DerefMut;
use keep_alive::DropAll;
use keep_alive::Guard;
use keep_alive::Parent;
use metrique_writer_core::EntrySink;
use std::sync::Arc;

pub use metrique_core::{CloseValue, CloseValueRef, Counter, InflectableEntry, NameStyle};

/// Unit types and utilities for metrics.
///
/// This module provides various unit types for metrics, such as time units (Second, Millisecond),
/// data size units (Byte, Kilobyte), and rate units (BytePerSecond).
///
/// These units can be attached to metrics using the `#[metrics(unit = ...)]` attribute.
pub mod unit {
    pub use metrique_writer_core::unit::{
        Bit, BitPerSecond, Byte, BytePerSecond, Count, Gigabit, GigabitPerSecond, Gigabyte,
        GigabytePerSecond, Kilobit, KilobitPerSecond, Kilobyte, KilobytePerSecond, Megabit,
        MegabitPerSecond, Megabyte, MegabytePerSecond, Microsecond, Millisecond, None, Percent,
        Second, Terabit, TerabitPerSecond, Terabyte, TerabytePerSecond,
    };
    use metrique_writer_core::{MetricValue, unit::WithUnit};
    /// Internal trait to attach units when closing values
    #[doc(hidden)]
    pub trait AttachUnit: Sized {
        type Output<U>;
        fn make<U>(self) -> Self::Output<U>;
    }

    impl<V: MetricValue> AttachUnit for V {
        type Output<U> = WithUnit<V, U>;

        fn make<U>(self) -> Self::Output<U> {
            WithUnit::from(self)
        }
    }
}

#[doc(hidden)]
pub mod format {
    pub use metrique_writer_core::value::FormattedValue;
}

/// Test utilities for metrique
#[cfg(feature = "test-util")]
pub mod test_util {
    pub use crate::writer::test_util::{
        Inspector, Metric, TestEntry, TestEntrySink, test_entry_sink, test_metric, to_test_entry,
    };
}

/// Unit of work metrics macros and utilities.
///
/// This module provides the `metrics` macro for defining unit of work metrics structs.
/// Unit of work metrics are typically tied to the request/response scope and capture
/// metrics over the course of a request.
///
/// Example:
/// ```
/// # use metrique::unit_of_work::metrics;
/// #
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: &'static str,
///     count: usize,
/// }
/// ```
pub mod unit_of_work {
    pub use metrique_macro::metrics;
}

/// Default sink type for metrics.
///
/// This is a type alias for `metrique_writer_core::sink::BoxEntrySink`, which is a boxed
/// entry sink that can be used to append closed metrics entries.
pub type DefaultSink = metrique_writer_core::sink::BoxEntrySink;

/// A wrapper that appends and closes an entry when dropped.
///
/// This struct holds a metric entry and a sink. When the struct is dropped,
/// it closes the entry and appends it to the sink.
///
/// The [`metrics`] macro generates a type alias to this type
/// named `<metric struct name>Guard`, you should normally mention that instead
/// of mentioning `AppendAndCloseOnDrop` directly.
///
/// This is typically created using the `append_on_drop` method on a metrics struct
/// or through the `append_and_close` function.
///
/// [`metrics`]: crate::unit_of_work::metrics
///
/// # Example
///
/// ```
/// # use metrique::ServiceMetrics;
/// # use metrique::unit_of_work::metrics;
/// # use metrique::writer::GlobalEntrySink;
/// #
/// #[metrics]
/// struct MyMetrics {
///     operation: &'static str,
/// }
///
/// # fn example() {
/// let metrics: MyMetricsGuard /* type alias */ = MyMetrics {
///     operation: "example",
/// }.append_on_drop(ServiceMetrics::sink());
/// // When `metrics` is dropped, it will be closed and appended to the sink
/// # }
/// ```
pub struct AppendAndCloseOnDrop<E: CloseEntry, S: EntrySink<RootMetric<E>>> {
    inner: Parent<AppendAndCloseOnDropInner<E, S>>,
}

impl<E: CloseEntry + Send + Sync + 'static, S: EntrySink<RootMetric<E>> + Send + Sync + 'static>
    AppendAndCloseOnDrop<E, S>
{
    /// Create a `flush_guard` to delay flushing the entry to the backing sink
    ///
    /// When you create a [`FlushGuard`], the actual appending of the record to the attached sink will
    /// occur after both:
    /// 1. This struct ([`AppendAndCloseOnDrop`]) is dropped (if [AppendAndCloseOnDrop::handle] is used,
    ///    then after all instances of the [`AppendAndCloseOnDropHandle`] have been dropped).
    /// 2. Either all [`FlushGuard`]s have been dropped, or *any* of the [`ForceFlushGuard`]s has been
    ///    dropped.
    ///
    /// This is most useful when the metrics struct contains a [`SharedChild`] or a [`Slot`], to allow for
    /// delaying the metric's flush until the tasks using the slot have recorded their metrics. Note
    /// that a [`Slot`] can hold a [`FlushGuard`] using [`OnParentDrop::Wait`].
    ///
    /// Creating a [`FlushGuard`] does not actually _block_ this struct from being dropped. The actual
    /// write to the background sink happens in the thread of the last guard to drop.
    ///
    /// If you want to force the entry to be immediately flushed, you can use [`Self::force_flush_guard`], then
    /// drop the resulting guard. That will prevent any present (and future) `FlushGuard`s from preventing the
    /// main entry from flushing to the sink.
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    /// use metrique::{Counter, OnParentDrop, ServiceMetrics, Slot, SlotGuard};
    /// use metrique::unit_of_work::metrics;
    /// use metrique::writer::GlobalEntrySink;
    ///
    /// #[metrics(rename_all = "PascalCase")]
    /// struct RequestMetrics {
    ///     operation: &'static str,
    ///     #[metrics(flatten)]
    ///     background_metrics: Slot<BackgroundMetrics>,
    /// }
    ///
    /// #[metrics(subfield)]
    /// #[derive(Default)]
    /// struct BackgroundMetrics {
    ///     field_1: usize,
    ///     counter: Counter,
    /// }
    ///
    /// async fn handle_request() {
    ///     let mut metrics = RequestMetrics {
    ///         operation: "abc",
    ///         background_metrics: Default::default(),
    ///     }
    ///     .append_on_drop(ServiceMetrics::sink());
    ///
    ///     let flush_guard = metrics.flush_guard();
    ///     // the flush_guard will delay the metric emission until dropped
    ///     // use OnParentDrop::Wait to wait until the `SlotGuard` is flushed.
    ///     let background_metrics = metrics
    ///         .background_metrics
    ///         .open(OnParentDrop::Wait(flush_guard))
    ///         .unwrap();
    ///
    ///     tokio::task::spawn(do_background_work(background_metrics));
    ///     // metric will be emitted after `do_background_work` completes
    /// }
    ///
    /// async fn do_background_work(mut metrics: SlotGuard<BackgroundMetrics>) {
    ///     // do some slow operation
    ///     tokio::time::sleep(Duration::from_secs(1)).await;
    ///     // `SlotGuard` derefs to the slot contents
    ///     metrics.field_1 += 1;
    /// }
    /// ```
    pub fn flush_guard(&self) -> FlushGuard {
        FlushGuard {
            _drop_guard: self.inner.new_guard(),
        }
    }

    /// <div class="warning">
    /// `ForceFlushGuard` is currently in an experimental state, and does not seem to
    /// have many real-world use-cases in its current state.
    ///
    /// If you are interested in getting it improved to fit your use-case, file an
    /// issue.
    /// </div>
    ///
    /// Create a [`ForceFlushGuard`]
    ///
    /// When a [`ForceFlushGuard`] (it is possible to create multiple of them) along with all
    /// "direct" references to the [`AppendAndCloseOnDrop`] have been dropped, the metric will
    /// be emitted.
    ///
    /// A typical usage is handing a [`Slot`] to a background task, and dropping the
    /// [`ForceFlushGuard`] after a timeout to ensure the rest of the metric record will
    /// be emitted even if the background task takes a very long time to finish - in that case,
    /// the metrics from the background task written after the timeout will not
    /// be emitted, but the rest the metric entry will be emitted.
    pub fn force_flush_guard(&self) -> ForceFlushGuard {
        ForceFlushGuard::new(self.inner.force_drop_guard())
    }

    /// Return a cloneable handle to the contents. The handle allows for cloneable,
    /// shared access to the contents.
    ///
    /// The [`metrics`] macro generates a type alias to the return type of this function
    /// named `<my metrics struct>Handle`.
    ///
    /// [`metrics`]: crate::unit_of_work::metrics
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use metrique::{Counter, ServiceMetrics};
    /// use metrique::unit_of_work::metrics;
    /// use metrique::writer::GlobalEntrySink;
    /// use tokio::task::JoinSet;
    ///
    /// #[metrics(rename_all = "PascalCase")]
    /// struct RequestMetrics {
    ///     operation: &'static str,
    ///     counter: Counter,
    /// }
    ///
    /// async fn handle_request() {
    ///     let mut metrics = RequestMetrics {
    ///         operation: "abc",
    ///         counter: Default::default(),
    ///     }
    ///     .append_on_drop(ServiceMetrics::sink());
    ///
    ///     let handle = metrics.handle();
    ///     let join_set: JoinSet<_> = (0..10).map(
    ///         |_| do_parallel_work(handle.clone())
    ///     ).collect();
    ///     join_set.join_all().await;
    ///
    ///     // metric will be emitted here
    /// }
    ///
    /// async fn do_parallel_work(mut metrics: /* type alias */ RequestMetricsHandle) {
    ///     // do some slow operation
    ///     tokio::time::sleep(Duration::from_secs(1)).await;
    ///     // `handle` derefs to the metric contents
    ///     metrics.counter.increment();
    /// }
    /// ```
    pub fn handle(self) -> AppendAndCloseOnDropHandle<E, S> {
        AppendAndCloseOnDropHandle {
            inner: std::sync::Arc::new(self),
        }
    }
}

struct AppendAndCloseOnDropInner<E: CloseEntry, S: EntrySink<RootMetric<E>>> {
    entry: Option<E>,
    sink: S,
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> Deref for AppendAndCloseOnDrop<E, S> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}
//
impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> DerefMut for AppendAndCloseOnDrop<E, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.deref_mut()
    }
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> Deref for AppendAndCloseOnDropInner<E, S> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.entry.as_ref().unwrap()
    }
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> DerefMut for AppendAndCloseOnDropInner<E, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entry.as_mut().unwrap()
    }
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> Drop for AppendAndCloseOnDropInner<E, S> {
    fn drop(&mut self) {
        let entry = self.entry.take().expect("only drop calls this");
        let entry = entry.close();
        self.sink.append(RootEntry::new(entry));
    }
}

/// Handle to an [`AppendAndCloseOnDrop`], returned by [`AppendAndCloseOnDrop::handle`].
///
/// This is basically an `Arc<AppendAndCloseOnDrop>`, allowing shared and clone access to the contents.
///
/// The [`metrics`] macro generates a type alias to this type
/// named `<metric struct name>Handle`, you should normally mention that instead
/// of mentioning `AppendAndCloseOnDropHandle` directly.
///
/// [`metrics`]: crate::unit_of_work::metrics
pub struct AppendAndCloseOnDropHandle<E: CloseEntry, S: EntrySink<RootMetric<E>>> {
    inner: Arc<AppendAndCloseOnDrop<E, S>>,
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> Clone for AppendAndCloseOnDropHandle<E, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<E: CloseEntry, S: EntrySink<RootMetric<E>>> std::ops::Deref
    for AppendAndCloseOnDropHandle<E, S>
{
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

/// Creates an [`AppendAndCloseOnDrop`] wrapper for a metric entry and sink.
///
/// This function takes a metric entry and a sink, and returns a wrapper that will
/// close the entry and append it to the sink when dropped.
///
/// # Parameters
/// * `base` - The metric entry to close and append
/// * `sink` - The sink to append the closed entry to
///
/// # Returns
///
/// An [`AppendAndCloseOnDrop`] wrapper that will close and append the entry when dropped.
///
/// The [`metrics`] macro generates a type alias to [`AppendAndCloseOnDrop`] named
/// `<my metrics struct>Guard`. When using the macro, it is recommended to refer
/// to the return type using that alias.
///
/// [`metrics`]: crate::unit_of_work::metrics
///
/// # Example
/// ```
/// # use metrique::{append_and_close, unit_of_work::metrics, ServiceMetrics};
/// # use metrique::writer::{GlobalEntrySink, FormatExt};
/// #
/// #[metrics]
/// struct MyMetrics {
///     operation: &'static str,
///     counter: u32,
/// }
///
///
/// fn some_function(metrics: &mut /* type alias */ MyMetricsGuard) {
///     metrics.counter += 1;
/// }
///
/// # fn example() {
/// let mut metrics = append_and_close(
///     MyMetrics { operation: "example", counter: 0 },
///     ServiceMetrics::sink()
/// );
/// some_function(&mut metrics);
/// // When `metrics` is dropped, it will be closed and appended to the sink
/// # }
/// ```
pub fn append_and_close<
    C: CloseEntry + Send + Sync + 'static,
    S: EntrySink<RootMetric<C>> + Send + Sync + 'static,
>(
    base: C,
    sink: S,
) -> AppendAndCloseOnDrop<C, S> {
    AppendAndCloseOnDrop {
        inner: Parent::new(AppendAndCloseOnDropInner {
            entry: Some(base),
            sink,
        }),
    }
}

/// A wrapper around `Arc<T>` that writes inner metrics on close if there is exactly
/// one reference open (meaning the parent's reference). This allows you to clone around
/// owned handles to the child metrics struct without dealing with lifetimes and references.
///
/// If there are ANY pending background tasks with clones of this struct, if the parent entry closes, contained
/// metrics fields will NOT be included at all even if a subset of the tasks finish.
///
/// This behavior is similar to [`Slot`], except that [`Slot`] provides mutable references at the cost of
/// a oneshot channel, so is optimized for cases where you don't want to use (more expensive) concurrent metric fields
/// that can be written to with &self.
///
/// Additionally, [`Slot`] supports letting the parent entry to delay flushing (in the background) until child entries close,
/// To accomplish this, use [`SlotGuard::delay_flush()`].
pub struct SharedChild<T>(Arc<T>);
impl<T> SharedChild<T> {
    /// Construct a [`SharedChild`] with values already initialized,
    /// useful if you have some fields that can't be written to with &self
    pub fn new(value: T) -> Self {
        Self(Arc::from(value))
    }
}

impl<T> Clone for SharedChild<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Default> Default for SharedChild<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Deref for SharedChild<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValue> CloseValue for SharedChild<T> {
    type Closed = Option<T::Closed>;

    fn close(self) -> Self::Closed {
        Arc::into_inner(self.0).map(|t| t.close())
    }
}

/// Type alias to a [`RootEntry`] that wraps around a metric entry. This
/// is used to turn a metric into a concrete metric entry that can be sent
/// to an [`EntrySink`]. This is normally the type entry sinks are
/// instantiated for.
///
/// See the [`RootEntry`] docs for more details.
///
/// # Example
///
/// This creates a [`BackgroundQueue`] that is specialized for the entry
/// type from `MyEntry`
///
/// [`BackgroundQueue`]: crate::writer::sink::BackgroundQueue
///
/// ```
/// use metrique::{CloseValue, RootMetric};
/// use metrique::emf::Emf;
/// use metrique::writer::{EntrySink, FormatExt};
/// use metrique::writer::sink::BackgroundQueue;
/// use metrique::unit_of_work::metrics;
///
/// #[metrics]
/// #[derive(Default)]
/// struct MyEntry {
///     value: u32
/// }
///
/// type MyRootEntry = RootMetric<MyEntry>;
///
/// let (queue, handle) = BackgroundQueue::<MyRootEntry>::new(
///     Emf::builder("Ns".to_string(), vec![vec![]])
///         .build()
///         .output_to(std::io::stdout())
/// );
///
/// handle_request(&queue);
///
/// fn handle_request(queue: &BackgroundQueue<MyRootEntry>) {
///     let mut metric = MyEntry::default();
///     metric.value += 1;
///     // or you can `metric.append_on_drop(queue.clone())`, but that clones an `Arc`
///     // which has slightly negative performance impact
///     queue.append(MyRootEntry::new(metric.close()));
/// }
/// ```
pub type RootMetric<E> = RootEntry<<E as CloseValue>::Closed>;

/// "Roots" an [`InflectableEntry`] to turn it into an [`Entry`] that can be passed
/// to an [`EntrySink`].
///
/// The names in an [`InflectableEntry`] such as the struct created by [closing over] a
/// [`metrics`] value can be inflected at compile time, for example by adding a
/// prefix. To send the metrics to an entry sink, the metrics need to be rooted
/// by using a [`RootEntry`].
///
/// When using [`append_and_close`], the metrics are rooted for you, but it is also possible
/// to root a metric entry in your own code.
///
/// The [`RootMetric`] alias exists to help avoid writing
/// `RootEntry<<E as CloseValue>::Closed>` explicitly.
///
/// # Example
///
/// ```
/// use metrique::{CloseValue, ServiceMetrics, RootEntry};
/// use metrique::unit_of_work::metrics;
/// use metrique::writer::{EntrySink, GlobalEntrySink};
///
/// #[metrics]
/// struct MyMetrics {
///     operation: &'static str,
/// }
///
/// # fn example() {
/// let metrics = MyMetrics {
///     operation: "example",
/// };
/// // same as calling `drop(metrics.append_on_drop(ServiceMetrics::sink()));`
/// ServiceMetrics::sink().append(RootEntry::new(metrics.close()));
/// # }
/// ```
///
/// [closing over]: crate::CloseEntry
/// [`EntrySink`]: metrique_writer::EntrySink
/// [`metrics`]: crate::unit_of_work::metrics
pub struct RootEntry<M: InflectableEntry> {
    metric: M,
}

impl<M: InflectableEntry> RootEntry<M> {
    /// create a new [`RootEntry`]
    pub fn new(metric: M) -> Self {
        Self { metric }
    }
}

impl<M: InflectableEntry> Entry for RootEntry<M> {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        self.metric.write(w);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.metric.sample_group()
    }
}

#[cfg(feature = "service-metrics")]
pub use metrique_service_metrics::ServiceMetrics;

#[cfg(feature = "metrics-rs-bridge")]
pub use metrique_metricsrs as metrics_rs;

pub use metrique_core::concat;

/// Re-exports of [metrique_writer]
pub mod writer {
    pub use metrique_writer::GlobalEntrySink;
    pub use metrique_writer::{AnyEntrySink, BoxEntrySink, EntrySink};
    pub use metrique_writer::{BoxEntry, EntryConfig, EntryWriter, core::Entry};
    pub use metrique_writer::{Convert, Unit};
    pub use metrique_writer::{EntryIoStream, IoStreamError};
    pub use metrique_writer::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
    pub use metrique_writer::{ValidationError, ValidationErrorBuilder};

    // Use the variant of the macro that has `metrique::` prefixes.
    pub use metrique_writer_macro::MetriqueEntry as Entry;

    pub use metrique_writer::AttachGlobalEntrySinkExt;
    pub use metrique_writer::{AttachGlobalEntrySink, EntryIoStreamExt, FormatExt};
    pub use metrique_writer::{entry, format, sample, sink, stream, value};

    #[cfg(feature = "test-util")]
    #[doc(hidden)] // prefer the metrique::test_util re-export
    pub use metrique_writer::test_util;

    #[doc(hidden)] // prefer the metrique::unit re-export
    pub use metrique_writer::unit;

    // used by macros
    #[doc(hidden)]
    pub use metrique_writer::core;
}
