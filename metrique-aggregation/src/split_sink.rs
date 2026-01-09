//! Split sink that sends to multiple destinations

use crate::traits::{AggregateSink, AggregateSinkRef, FlushableSink};

/// Sends entries to two sinks by reference - useful for aggregating while also sending raw entries
///
/// This requires sink B to implement `AggregateSinkRef<T>` which typically means
/// the source type must implement `MergeRef`.
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
    A: AggregateSink<T>,
    B: AggregateSinkRef<T>,
{
    fn add(&self, entry: T) {
        self.sink_b.add_ref(&entry);
        self.sink_a.add(entry);
    }
}

impl<A, B> FlushableSink for SplitSink<A, B>
where
    A: FlushableSink,
    B: FlushableSink,
{
    fn flush(&self) {
        self.sink_a.flush();
        self.sink_b.flush();
    }
}
