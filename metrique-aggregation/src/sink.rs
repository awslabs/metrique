//! Sinks for aggregation

use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::traits::{AggregateStrategy, Merge};

/// Handle for metric that will be automatically merged into the target when dropped
pub struct MergeOnDrop<T, Sink>
where
    T: AggregateStrategy,
    Sink: AggregateSink<T>,
{
    value: Option<T::Source>,
    target: Sink,
}

impl<T, S> Deref for MergeOnDrop<T, S>
where
    T: AggregateStrategy,
    S: AggregateSink<T>,
{
    type Target = T::Source;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for MergeOnDrop<T, S>
where
    T: AggregateStrategy,
    S: AggregateSink<T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, Sink> Drop for MergeOnDrop<T, Sink>
where
    T: AggregateStrategy,
    Sink: AggregateSink<T>,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(value);
        }
    }
}

/// Trait that aggregates items
pub trait AggregateSink<T: AggregateStrategy> {
    /// Merge a given item into the sink
    fn merge(&self, entry: T::Source);
}

/// Sink that aggregates a single type of entry backed by a mutex
///
/// Compared to [`crate::traits::Aggregate`], this type allows appending with `&T` so it supports
/// using merge_on_drop
pub struct MutexAggregator<T: AggregateStrategy> {
    aggregator: Arc<Mutex<<T::Source as Merge>::Merged>>,
}

impl<T: AggregateStrategy> Clone for MutexAggregator<T> {
    fn clone(&self) -> Self {
        Self {
            aggregator: self.aggregator.clone(),
        }
    }
}

impl<T> Default for MutexAggregator<T>
where
    T: AggregateStrategy,
    <T::Source as Merge>::Merged: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T: AggregateStrategy> MutexAggregator<T> {
    /// Creates a new mutex sink
    pub fn new() -> MutexAggregator<T>
    where
        <T::Source as Merge>::Merged: Default,
    {
        Self {
            aggregator: Arc::new(Mutex::new(<T::Source as Merge>::Merged::default())),
        }
    }
}

impl<T: AggregateStrategy> AggregateSink<T> for MutexAggregator<T> {
    fn merge(&self, entry: T::Source) {
        let mut aggregator = self.aggregator.lock().unwrap();
        T::Source::merge(&mut *aggregator, entry);
    }
}

impl<T: AggregateStrategy> CloseValue for MutexAggregator<T>
where
    <T::Source as Merge>::Merged: CloseValue,
{
    type Closed = <<T::Source as Merge>::Merged as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        Arc::try_unwrap(self.aggregator)
            .ok()
            .expect("MutexAggregator must be the only reference when closing")
            .into_inner()
            .unwrap()
            .close()
    }
}
