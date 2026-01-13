//! Sinks for aggregation

use std::ops::{Deref, DerefMut};

use metrique::{InflectableEntry, RootEntry};
use metrique_core::CloseValue;
use metrique_writer::EntrySink;

use crate::traits::{AggregateSink, AggregateSinkRef, AggregateStrategy, FlushableSink, RootSink};

pub mod mutex;
pub mod worker;

pub use mutex::MutexSink;
pub use worker::WorkerSink;

/// Handle for metric that will be automatically merged into the target when dropped (for raw mode)
pub struct MergeOnDrop<T, Sink>
where
    T: AggregateStrategy<Source = T>,
    Sink: RootSink<T>,
{
    value: Option<T>,
    target: Sink,
}

impl<T, S> Deref for MergeOnDrop<T, S>
where
    T: AggregateStrategy<Source = T>,
    S: RootSink<T>,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for MergeOnDrop<T, S>
where
    T: AggregateStrategy<Source = T>,
    S: RootSink<T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, Sink> Drop for MergeOnDrop<T, Sink>
where
    T: AggregateStrategy<Source = T>,
    Sink: RootSink<T>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(value);
        }
    }
}

impl<T, Sink> MergeOnDrop<T, Sink>
where
    T: AggregateStrategy<Source = T>,
    Sink: RootSink<T>,
{
    /// Create a new MergeOnDrop that will merge the value on drop
    pub fn new(value: T, target: Sink) -> Self {
        Self {
            value: Some(value),
            target,
        }
    }
}

/// Trait alias for drop guards to simplify code.
///
/// This trait is not intended to be implemented directly, instead it should be used as a method argument when you want to accept an
/// aggregated metric you have used with `close_and_merge_on_drop`
///
/// ```
/// use metrique::{unit_of_work::metrics, timers::Timer};
/// use metrique_aggregation::{aggregate, sink::DropGuard, value::Sum};
/// use metrique_aggregation::aggregator::KeyedAggregator;
/// use metrique_aggregation::sink::MutexSink;
/// # use metrique::test_util::test_entry_sink;
/// #[aggregate]
/// #[metrics]
/// struct QueueItem {
///     #[aggregate(strategy = Sum)]
///     processing_time: Timer,
/// }
/// async fn process_item(item: &str, entry: impl DropGuard<QueueItem>) {}
///
/// # fn main() {
/// # let base_sink = test_entry_sink().sink;
/// let aggregator = KeyedAggregator::<QueueItem>::new(base_sink);
/// let sink = MutexSink::new(aggregator);
/// let queue_item = QueueItem { processing_time: Timer::start_now() }.close_and_merge(sink);
/// # }
/// ```
pub trait DropGuard<T>: Deref<Target = T> + DerefMut {}

impl<T, U> DropGuard<T> for CloseAndMergeOnDrop<T, U>
where
    T: CloseValue,
    U: RootSink<T::Closed>,
{
}

//pub type Guard<T: CloseValue> = CloseAndMergeOnDrop<T, impl RootSink<<T as CloseValue>::Closed>>;

/// Handle for metric that will be closed and merged into the target when dropped (for entry mode)
pub struct CloseAndMergeOnDrop<T, Sink>
where
    T: CloseValue,
    Sink: RootSink<T::Closed>,
{
    value: Option<T>,
    target: Sink,
}

impl<T, S> Deref for CloseAndMergeOnDrop<T, S>
where
    T: CloseValue,
    S: RootSink<T::Closed>,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for CloseAndMergeOnDrop<T, S>
where
    T: CloseValue,
    S: RootSink<T::Closed>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, Sink> Drop for CloseAndMergeOnDrop<T, Sink>
where
    T: CloseValue,
    Sink: RootSink<T::Closed>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(value.close());
        }
    }
}

impl<T, Sink> CloseAndMergeOnDrop<T, Sink>
where
    T: CloseValue,
    Sink: RootSink<T::Closed>,
{
    /// Create a new CloseAndMergeOnDrop that will close and merge the value on drop
    pub fn new(value: T, target: Sink) -> Self {
        Self {
            value: Some(value),
            target,
        }
    }
}

/// Sends entries to two sinks by reference - useful for aggregating while also sending raw entries
///
/// This requires sink A to implement `AggregateSinkRef<T>` which typically means
/// the source type must implement `MergeRef`.
///
/// - You can chain more impls by nesting SplitSink.
/// - You can write entries to an `EntrySink` (unaggregated) by wrapping an entry sink in [`EntrySinkAsAggregateSink`]
pub struct SplitSink<A, B> {
    sink_a: A,
    sink_b: B,
}

impl<A, B> SplitSink<A, B> {
    /// Create a new split sink
    pub fn new(sink_a: A, sink_b: B) -> Self {
        Self { sink_a, sink_b }
    }
}

impl<T, A, B> AggregateSink<T> for SplitSink<A, B>
where
    A: AggregateSinkRef<T>,
    B: AggregateSink<T>,
{
    fn merge(&mut self, entry: T) {
        self.sink_a.merge_ref(&entry);
        self.sink_b.merge(entry);
    }
}

impl<A, B> FlushableSink for SplitSink<A, B>
where
    A: FlushableSink,
    B: FlushableSink,
{
    fn flush(&mut self) {
        self.sink_a.flush();
        self.sink_b.flush();
    }
}

/// EntrySinkAsAggregateSink allows wraps an `EntrySink` so it can be used as an aggregate destination
///
/// Note: `flush` does NOT call the underlying flush method and is a no-op.
///
/// This is because, you typically _don't_ want to "flush" the raw sink whenever you want to flush out a new aggregate.
pub struct EntrySinkAsAggregateSink<T>(T);

impl<T> EntrySinkAsAggregateSink<T> {
    /// Create a new wrapper from a given sink entry sink
    pub fn new(sink: T) -> Self {
        Self(sink)
    }
}

impl<E, T> AggregateSink<E> for EntrySinkAsAggregateSink<T>
where
    E: InflectableEntry,
    T: EntrySink<RootEntry<E>>,
{
    fn merge(&mut self, entry: E) {
        self.0.append(RootEntry::new(entry));
    }
}

impl<T> FlushableSink for EntrySinkAsAggregateSink<T> {
    fn flush(&mut self) {
        // flushing a raw sink doesn't do anything
    }
}
