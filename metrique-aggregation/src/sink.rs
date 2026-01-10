//! Sinks for aggregation

use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::traits::{AggregateSink, AggregateStrategy, CloseAggregateEntry, Merge};

/// Handle for metric that will be automatically merged into the target when dropped (for raw mode)
pub struct MergeOnDrop<T, Sink>
where
    T: AggregateStrategy<Source = T>,
    Sink: AggregateSink<T>,
{
    value: Option<T>,
    target: Sink,
}

impl<T, S> Deref for MergeOnDrop<T, S>
where
    T: AggregateStrategy<Source = T>,
    S: AggregateSink<T>,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for MergeOnDrop<T, S>
where
    T: AggregateStrategy<Source = T>,
    S: AggregateSink<T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, Sink> Drop for MergeOnDrop<T, Sink>
where
    T: AggregateStrategy<Source = T>,
    Sink: AggregateSink<T>,
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
    Sink: AggregateSink<T>,
{
    /// Create a new MergeOnDrop that will merge the value on drop
    pub fn new(value: T, target: Sink) -> Self {
        Self {
            value: Some(value),
            target,
        }
    }
}

/// Handle for metric that will be closed and merged into the target when dropped (for entry mode)
pub struct CloseAndMergeOnDrop<T, Sink>
where
    T: CloseValue,
    Sink: AggregateSink<T::Closed>,
{
    value: Option<T>,
    target: Sink,
}

impl<T, S> Deref for CloseAndMergeOnDrop<T, S>
where
    T: CloseValue,
    S: AggregateSink<T::Closed>,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for CloseAndMergeOnDrop<T, S>
where
    T: CloseValue,
    S: AggregateSink<T::Closed>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, Sink> Drop for CloseAndMergeOnDrop<T, Sink>
where
    T: CloseValue,
    Sink: AggregateSink<T::Closed>,
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
    Sink: AggregateSink<T::Closed>,
{
    /// Create a new CloseAndMergeOnDrop that will close and merge the value on drop
    pub fn new(value: T, target: Sink) -> Self {
        Self {
            value: Some(value),
            target,
        }
    }
}

/// Extension trait for creating merge-on-drop guards
pub trait CloseAndMergeOnDropExt: CloseValue + Sized {
    /// Create a guard that will close and merge this value on drop
    fn close_and_merge<Sink: AggregateSink<Self::Closed>>(
        self,
        sink: Sink,
    ) -> CloseAndMergeOnDrop<Self, Sink> {
        CloseAndMergeOnDrop::new(self, sink)
    }
}

impl<T> CloseAndMergeOnDropExt for T
where
    T: CloseAggregateEntry<T>,
    T: AggregateStrategy,
{
}

/// Extension trait for creating merge-on-drop guards (raw mode)
pub trait MergeOnDropExt: Sized + AggregateStrategy<Source = Self> {
    /// Create a guard that will merge this value on drop
    fn merge<Sink: AggregateSink<Self>>(self, sink: Sink) -> MergeOnDrop<Self, Sink> {
        MergeOnDrop::new(self, sink)
    }
}

impl<T> MergeOnDropExt for T where T: AggregateStrategy<Source = Self> {}

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

impl<T: AggregateStrategy> AggregateSink<T::Source> for MutexAggregator<T> {
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
