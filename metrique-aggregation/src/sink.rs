//! Sinks for aggregation

use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::aggregate::{Aggregate, AggregateEntry};

/// Handle for metric that will be automatically merged into the target when dropped
pub struct MergeOnDrop<Entry, Sink>
where
    Entry: AggregateEntry<OwnedSource = Entry>,
    for<'a> Sink: AggregateSink<Entry::Source<'a>>,
{
    value: Option<Entry>,
    target: Sink,
}

/// Handle for metric that will be closed and merged into the target when dropped
pub struct MergeAndCloseOnDrop<Entry, Sink>
where
    Entry: CloseValue + AggregateEntry<OwnedSource = Entry>,
    Entry::Closed: AggregateEntry<OwnedSource = Entry::Closed>,
    for<'a> Sink: AggregateSink<<Entry::Closed as AggregateEntry>::Source<'a>>,
{
    value: Option<Entry>,
    target: Sink,
}

impl<Entry, S> Deref for MergeOnDrop<Entry, S>
where
    Entry: AggregateEntry<OwnedSource = Entry>,
    for<'a> S: AggregateSink<Entry::Source<'a>>,
{
    type Target = Entry;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<Entry, S> DerefMut for MergeOnDrop<Entry, S>
where
    Entry: AggregateEntry<OwnedSource = Entry>,
    for<'a> S: AggregateSink<Entry::Source<'a>>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<Entry, S> Deref for MergeAndCloseOnDrop<Entry, S>
where
    Entry: CloseValue + AggregateEntry<OwnedSource = Entry>,
    Entry::Closed: AggregateEntry<OwnedSource = Entry::Closed>,
    for<'a> S: AggregateSink<<Entry::Closed as AggregateEntry>::Source<'a>>,
{
    type Target = Entry;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<Entry, S> DerefMut for MergeAndCloseOnDrop<Entry, S>
where
    Entry: CloseValue + AggregateEntry<OwnedSource = Entry>,
    Entry::Closed: AggregateEntry<OwnedSource = Entry::Closed>,
    for<'a> S: AggregateSink<<Entry::Closed as AggregateEntry>::Source<'a>>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

/// Extension trait to supporting merging an item into an aggregator on drop
pub trait MergeOnDropExt: AggregateEntry<OwnedSource = Self> + Sized {
    /// Merge an item into a given sink when the guard drops
    fn merge_on_drop<S>(self, sink: &S) -> MergeOnDrop<Self, S>
    where
        for<'a> S: AggregateSink<Self::Source<'a>> + Clone,
    {
        MergeOnDrop {
            value: Some(self),
            target: sink.clone(),
        }
    }
}

/// Extension trait to support merging and closing an item into an aggregator on drop
pub trait MergeAndCloseOnDropExt: AggregateEntry<OwnedSource = Self> + CloseValue + Sized
where
    Self::Closed: AggregateEntry<OwnedSource = Self::Closed>,
{
    /// Merge an item into a given sink when the guard drops, calling close first
    fn merge_and_close_on_drop<S>(self, sink: &S) -> MergeAndCloseOnDrop<Self, S>
    where
        for<'a> S: AggregateSink<<Self::Closed as AggregateEntry>::Source<'a>> + Clone,
    {
        MergeAndCloseOnDrop {
            value: Some(self),
            target: sink.clone(),
        }
    }
}

impl<Entry, Sink> Drop for MergeOnDrop<Entry, Sink>
where
    Entry: AggregateEntry<OwnedSource = Entry>,
    for<'a> Sink: AggregateSink<Entry::Source<'a>>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(Entry::to_ref(&value));
        }
    }
}

impl<Entry, Sink> Drop for MergeAndCloseOnDrop<Entry, Sink>
where
    Entry: CloseValue + AggregateEntry<OwnedSource = Entry>,
    Entry::Closed: AggregateEntry<OwnedSource = Entry::Closed>,
    for<'a> Sink: AggregateSink<<Entry::Closed as AggregateEntry>::Source<'a>>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            let closed = value.close();
            self.target.merge(<Entry::Closed as AggregateEntry>::to_ref(&closed));
        }
    }
}

/// Trait that aggregates items
pub trait AggregateSink<T> {
    /// Merge a given item into the sink
    fn merge(&self, entry: T);
}

/// Aggregation that coordinates access with a Mutex
pub struct MutexSink<T: AggregateEntry>
where
    for<'a> T::Key<'a>: 'static,
{
    aggregator: Arc<Mutex<Option<Aggregate<T>>>>,
    default_value: Arc<dyn Fn() -> T::Aggregated + Send + Sync>,
}

impl<T: AggregateEntry> Clone for MutexSink<T>
where
    for<'a> T::Key<'a>: 'static,
{
    fn clone(&self) -> Self {
        Self {
            aggregator: self.aggregator.clone(),
            default_value: self.default_value.clone(),
        }
    }
}

impl<T: AggregateEntry> MutexSink<T>
where
    for<'a> T::Key<'a>: 'static,
{
    /// Creates a new mutex sink
    pub fn new() -> MutexSink<T>
    where
        T::Aggregated: Default,
    {
        Self {
            aggregator: Default::default(),
            default_value: Arc::new(|| T::Aggregated::default()),
        }
    }
}

impl<'a, T: AggregateEntry> AggregateSink<T::Source<'a>> for MutexSink<T>
where
    for<'b> T::Key<'b>: 'static,
{
    fn merge(&self, entry: T::Source<'a>) {
        let mut aggregator = self.aggregator.lock().unwrap();
        match &mut *aggregator {
            Some(v) => {
                v.add(entry);
            }
            None => {
                let value = (self.default_value)();
                let mut agg = Aggregate::new(value);
                agg.add(entry);
                *aggregator = Some(agg);
            }
        }
    }
}

impl<T: AggregateEntry> CloseValue for MutexSink<T>
where
    for<'a> T::Key<'a>: 'static,
    T::Aggregated: CloseValue,
{
    type Closed = Option<<Aggregate<T> as CloseValue>::Closed>;

    fn close(self) -> Self::Closed {
        self.aggregator.lock().unwrap().take().map(|v| v.close())
    }
}
