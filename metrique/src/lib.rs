// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
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
        Inspector, Metric, TestEntry, TestEntrySink, test_entry_sink, to_test_entry,
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
/// The `#[metrics]` macro generates a type alias to this type
/// named `<metric struct name>Guard`, you should normally mention that instead
/// of mentioning `AppendAndCloseOnDrop` directly.
///
/// This is typically created using the `append_on_drop` method on a metrics struct
/// or through the `append_and_close` function.
///
/// Example:
/// ```
/// # use metrique::ServiceMetrics;
/// # use metrique::unit_of_work::metrics;
/// # use metrique::writer::GlobalEntrySink;
///
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
pub struct AppendAndCloseOnDrop<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> {
    inner: Parent<AppendAndCloseOnDropInner<E, S>>,
}

impl<
    E: CloseEntry + Send + Sync + 'static,
    S: EntrySink<RootEntry<E::Closed>> + Send + Sync + 'static,
> AppendAndCloseOnDrop<E, S>
{
    /// Create a `flush_guard` to delay flushing the entry to the backing sink
    ///
    /// When you create a [`FlushGuard`], the actual appending of the record to the attached sink will
    /// occur when:
    /// 1. This struct (`AppendAndCloseOnDrop`) is dropped.
    /// 2. FlushGuards have been dropped (or a `force_flush_guard` has been created and dropped).
    ///
    /// Creating a [`FlushGuard`] does not actually _block_ this struct from being dropped. The actual
    /// write to the background sink happens in the thread of the last guard to drop.
    ///
    /// If you want to force the entry to be immediately flushed, you can use [`Self::force_flush_guard`], then
    /// drop the resulting guard. That will prevent any present (and future) `FlushGuard`s from preventing the
    /// main entry from flushing to the sink.
    pub fn flush_guard(&self) -> FlushGuard {
        FlushGuard {
            _drop_guard: self.inner.new_guard(),
        }
    }

    /// Create a [`ForceFlushGuard`]
    ///
    /// A typical usage will be creating this prior to flushing the record and spawning a task to
    /// drop it after some timeout.
    pub fn force_flush_guard(&self) -> ForceFlushGuard {
        ForceFlushGuard {
            _drop_guard: self.inner.force_drop_guard(),
        }
    }

    /// Return a handle
    pub fn handle(self) -> AppendAndCloseOnDropHandle<E, S> {
        AppendAndCloseOnDropHandle {
            inner: std::sync::Arc::new(self),
        }
    }
}

struct AppendAndCloseOnDropInner<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> {
    entry: Option<E>,
    sink: S,
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> Deref for AppendAndCloseOnDrop<E, S> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> DerefMut for AppendAndCloseOnDrop<E, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.deref_mut()
    }
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> Deref for AppendAndCloseOnDropInner<E, S> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.entry.as_ref().unwrap()
    }
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> DerefMut
    for AppendAndCloseOnDropInner<E, S>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entry.as_mut().unwrap()
    }
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> Drop for AppendAndCloseOnDropInner<E, S> {
    fn drop(&mut self) {
        let entry = self.entry.take().expect("only drop calls this");
        let entry = entry.close();
        self.sink.append(RootEntry::new(entry));
    }
}

/// Handle to an AppendAndCloseOnDrop
pub struct AppendAndCloseOnDropHandle<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> {
    inner: Arc<AppendAndCloseOnDrop<E, S>>,
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> Clone for AppendAndCloseOnDropHandle<E, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<E: CloseEntry, S: EntrySink<RootEntry<E::Closed>>> std::ops::Deref
    for AppendAndCloseOnDropHandle<E, S>
{
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

/// Creates an `AppendAndCloseOnDrop` wrapper for a metric entry and sink.
///
/// This function takes a metric entry and a sink, and returns a wrapper that will
/// close the entry and append it to the sink when dropped.
///
/// # Parameters
/// * `base` - The metric entry to close and append
/// * `sink` - The sink to append the closed entry to
///
/// # Returns
/// An `AppendAndCloseOnDrop` wrapper that will close and append the entry when dropped
///
/// # Example
/// ```
/// # use metrique::{append_and_close, unit_of_work::metrics, ServiceMetrics};
/// # use metrique::writer::{GlobalEntrySink, FormatExt};
///
/// #[metrics]
/// struct MyMetrics {
///     operation: &'static str,
/// }
///
/// # fn example() {
/// let metrics = append_and_close(
///     MyMetrics { operation: "example" },
///     ServiceMetrics::sink()
/// );
/// // When `metrics` is dropped, it will be closed and appended to the sink
/// # }
/// ```
pub fn append_and_close<
    C: CloseEntry + Send + Sync + 'static,
    S: EntrySink<RootEntry<C::Closed>> + Send + Sync + 'static,
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

/// "Roots" an [`InflectableEntry`] to turn it into an [`Entry`] that can be passed
/// to an [`EntrySink`].
///
/// [`EntrySink`]: metrique_writer::EntrySink
pub struct RootEntry<M: InflectableEntry> {
    metric: M,
}

impl<M: InflectableEntry> RootEntry<M> {
    /// returns the metric
    pub fn metric(&self) -> &M {
        &self.metric
    }
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
    pub use metrique_writer::{entry, format, merge, sample, sink, stream, value};

    #[cfg(feature = "test-util")]
    #[doc(hidden)] // prefer the metrique::test_util re-export
    pub use metrique_writer::test_util;

    #[doc(hidden)] // prefer the metrique::unit re-export
    pub use metrique_writer::unit;

    // used by macros
    #[doc(hidden)]
    pub use metrique_writer::core;
}
