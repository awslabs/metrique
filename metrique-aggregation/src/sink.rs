//! Sinks for aggregation

use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::traits::{AggregateSink, AggregateStrategy, Merge, MergeRef, RootSink};

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

/// Sink that aggregates a single type of entry backed by a mutex
///
/// Compared to [`crate::traits::Aggregate`], this type allows appending with `&T` so it supports
/// using merge_on_drop
pub struct MutexSink<Inner> {
    inner: Arc<Mutex<Inner>>,
}

impl<Inner> Clone for MutexSink<Inner> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<Inner: Default> Default for MutexSink<Inner> {
    fn default() -> Self {
        Self::new(Inner::default())
    }
}

impl<Inner> MutexSink<Inner> {
    /// Creates a new mutex sink wrapping the inner aggregator
    pub fn new(inner: Inner) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T, Inner> RootSink<T> for MutexSink<Inner>
where
    Inner: AggregateSink<T>,
{
    fn merge(&self, entry: T) {
        self.inner.lock().unwrap().merge(entry);
    }
}

impl<Inner> CloseValue for MutexSink<Inner>
where
    Inner: CloseValue,
{
    type Closed = Inner::Closed;

    fn close(self) -> Self::Closed {
        Arc::try_unwrap(self.inner)
            .ok()
            .expect("MutexSink must be the only reference when closing")
            .into_inner()
            .unwrap()
            .close()
    }
}

/// Simple aggregator that merges entries into a single accumulated value
pub struct Aggregator<T: AggregateStrategy> {
    accumulated: <T::Source as Merge>::Merged,
    _phantom: PhantomData<T>,
}

impl<T> Default for Aggregator<T>
where
    T: AggregateStrategy,
    <T::Source as Merge>::Merged: Default,
{
    fn default() -> Self {
        Self {
            accumulated: Default::default(),
            _phantom: PhantomData,
        }
    }
}

impl<T> Aggregator<T>
where
    T: AggregateStrategy,
    <T::Source as Merge>::Merged: Default,
{
    /// Create a new aggregator
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T> AggregateSink<T::Source> for Aggregator<T>
where
    T: AggregateStrategy,
{
    fn merge(&mut self, entry: T::Source) {
        T::Source::merge(&mut self.accumulated, entry);
    }
}

impl<T> crate::traits::AggregateSinkRef<T::Source> for Aggregator<T>
where
    T: AggregateStrategy,
    T::Source: MergeRef,
{
    fn merge_ref(&mut self, entry: &T::Source) {
        T::Source::merge_ref(&mut self.accumulated, entry);
    }
}

impl<T> CloseValue for Aggregator<T>
where
    T: AggregateStrategy,
    <T::Source as Merge>::Merged: CloseValue,
{
    type Closed = <<T::Source as Merge>::Merged as CloseValue>::Closed;

    fn close(self) -> Self::Closed {
        self.accumulated.close()
    }
}
