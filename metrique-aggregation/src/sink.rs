//! Sinks for aggregation

use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::traits::{Aggregate, AggregateEntry};

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
            self.target.merge(value);
        }
    }
}

/// Trait that aggregates items
pub trait AggregateSink<T: AggregateEntry> {
    /// Merge a given item into the sink
    fn merge(&self, entry: T::Source);
}

/// Sink that aggregates a single type of entry backed by a mutex
///
/// Compared to [`Aggregate`], this type allows appending with `&T` so it supports
/// using [`MergeOnDropExt::merge_on_drop`]
pub struct MutexAggregator<T: AggregateEntry> {
    aggregator: Arc<Mutex<Option<T::Aggregated>>>,
}

impl<T: AggregateEntry> Clone for MutexAggregator<T> {
    fn clone(&self) -> Self {
        Self {
            aggregator: self.aggregator.clone(),
        }
    }
}

impl<T: AggregateEntry> MutexAggregator<T> {
    /// Creates a new mutex sink
    pub fn new() -> MutexAggregator<T>
    where
        T::Key: Default,
    {
        Self::with_key(&Default::default())
    }

    /// Creates a sync from a given key. NOTE: this sink does not aggregate by key
    ///
    /// If you create a sink with a non-trivial key, it is your
    /// responsibility to not mix streams.
    pub fn with_key(key: &T::Key) -> MutexAggregator<T> {
        Self {
            aggregator: Arc::new(Mutex::new(Some(T::new_aggregated(key)))),
        }
    }
}

impl<'k, T: AggregateEntry> AggregateSink<T> for MutexAggregator<T>
where
    T::Source: Clone,
{
    fn merge(&self, entry: T::Source) {
        let mut aggregator = self.aggregator.lock().unwrap();
        match &mut *aggregator {
            Some(v) => {
                T::merge_entry(v, entry);
            }
            None => {
                unreachable!("it is always initialized with a value")
            }
        }
    }
}

impl<T: AggregateEntry> CloseValue for MutexAggregator<T>
where
    T::Aggregated: CloseValue,
{
    type Closed = Option<<Aggregate<T> as CloseValue>::Closed>;

    fn close(self) -> Self::Closed {
        self.aggregator.lock().unwrap().take().map(|v| v.close())
    }
}
