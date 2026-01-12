//! Split sink that sends to multiple destinations

use metrique::{InflectableEntry, RootEntry};
use metrique_writer::EntrySink;

use crate::traits::{AggregateSink, AggregateSinkRef, FlushableSink};

/// Sends entries to two sinks by reference - useful for aggregating while also sending raw entries
///
/// This requires sink A to implement `AggregateSinkRef<T>` which typically means
/// the source type must implement `MergeRef`.
///
/// - You can chain more impls by nesting SplitSink.
/// - You can write raw entries to a Sink by wrapping an entry sink in [`RawSink`]
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

/// Raw sink allows using an Entry sink as an aggregate destination
///
/// Note: `flush` in a RawSink does NOT call the underlying flush method and is a no-op.
///
/// This is because, you typically _don't_ want to "flush" the raw sink whenever you want to flush out a new aggregate.
pub struct RawSink<T>(T);

impl<T> RawSink<T> {
    /// Create a new RawSink wrapper from a given sink
    pub fn new(sink: T) -> Self {
        Self(sink)
    }
}

impl<E, T> AggregateSink<E> for RawSink<T>
where
    E: InflectableEntry,
    T: EntrySink<RootEntry<E>>,
{
    fn merge(&mut self, entry: E) {
        self.0.append(RootEntry::new(entry));
    }
}

impl<T> FlushableSink for RawSink<T> {
    fn flush(&mut self) {
        // flushing a raw sink doesn't do anything
    }
}
