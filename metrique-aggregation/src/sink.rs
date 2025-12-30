//! Sinks for aggregation

use std::{
    borrow::Cow,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::aggregate::{Aggregate, AggregateEntry};

/// Handle for metric that will be automatically merged into the target when dropped
pub struct MergeOnDrop<Entry, Sink>
where
    Entry: AggregateEntry,
    Sink: AggregateSink<Entry>,
{
    value: Option<Entry::Source>,
    target: Sink,
}

impl<Entry, S> Deref for MergeOnDrop<Entry, S>
where
    Entry: AggregateEntry,
    S: AggregateSink<Entry>,
{
    type Target = Entry::Source;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<Entry, S> DerefMut for MergeOnDrop<Entry, S>
where
    Entry: AggregateEntry,
    S: AggregateSink<Entry>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

/// Extension trait to supporting merging an item into an aggregator on drop
pub trait MergeOnDropExt: AggregateEntry + Sized {
    /// Merge an item into a given sink when the guard drops
    fn merge_on_drop<S>(self, sink: &S) -> MergeOnDrop<Self, S>
    where
        Self: AggregateEntry<Source = Self>,
        S: AggregateSink<Self> + Clone,
    {
        MergeOnDrop {
            value: Some(self),
            target: sink.clone(),
        }
    }
}

impl<Entry, Sink> Drop for MergeOnDrop<Entry, Sink>
where
    Entry: AggregateEntry,
    Sink: AggregateSink<Entry>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(Cow::Owned(value));
        }
    }
}

/// Trait that aggregates items
pub trait AggregateSink<T: AggregateEntry> {
    /// Merge a given item into the sink
    fn merge<'a>(&self, entry: Cow<'a, T::Source>);
}

/// Aggregation that coordinates access with a Mutex
pub struct MutexSink<T: AggregateEntry> {
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

impl<T: AggregateEntry> AggregateSink<T> for MutexSink<T> {
    fn merge<'a>(&self, entry: Cow<'a, T::Source>) {
        let mut aggregator = self.aggregator.lock().unwrap();
        match &mut *aggregator {
            Some(v) => {
                v.add_ref(entry.as_ref());
            }
            None => {
                let value = (self.default_value)();
                let mut agg = Aggregate::new(value);
                agg.add_ref(entry.as_ref());
                *aggregator = Some(agg);
            }
        }
    }
}

impl<T: AggregateEntry> CloseValue for MutexSink<T>
where
    T::Aggregated: CloseValue,
{
    type Closed = Option<<Aggregate<T> as CloseValue>::Closed>;

    fn close(self) -> Self::Closed {
        self.aggregator.lock().unwrap().take().map(|v| v.close())
    }
}
