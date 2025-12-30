//! Sinks for aggregation

use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

use metrique_core::CloseValue;

use crate::aggregate::{Aggregate, SourceMetric};

/// Handle for metric that will be automatically merged into the target when dropped
pub struct MergeOnDrop<T, Sink: AggregateSink<T>> {
    value: Option<T>,
    target: Sink,
}

/// Handle for metric that will be closed and merged into the target when dropped
pub struct MergeAndCloseOnDrop<T, Sink: AggregateSink<T::Closed>>
where
    T: CloseValue,
{
    value: Option<T>,
    target: Sink,
}

impl<T, S: AggregateSink<T>> Deref for MergeOnDrop<T, S> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S: AggregateSink<T>> DerefMut for MergeOnDrop<T, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

impl<T, S> Deref for MergeAndCloseOnDrop<T, S>
where
    T: CloseValue,
    S: AggregateSink<T::Closed>,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("unreachable: valid until drop")
    }
}

impl<T, S> DerefMut for MergeAndCloseOnDrop<T, S>
where
    T: CloseValue,
    S: AggregateSink<T::Closed>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("unreachable: valid until drop")
    }
}

/// Extension trait to supporting merging an item into an aggregator on drop
pub trait MergeOnDropExt: SourceMetric {
    /// Merge an item into a given sink when the guard drops
    fn merge_on_drop<S>(self, sink: &S) -> MergeOnDrop<Self, S>
    where
        S: AggregateSink<Self> + Clone,
        Self: Sized,
    {
        MergeOnDrop {
            value: Some(self),
            target: sink.clone(),
        }
    }
}

/// Extension trait to support merging and closing an item into an aggregator on drop
pub trait MergeAndCloseOnDropExt: SourceMetric + CloseValue {
    /// Merge an item into a given sink when the guard drops, calling close first
    fn merge_and_close_on_drop<S>(self, sink: &S) -> MergeAndCloseOnDrop<Self, S>
    where
        S: AggregateSink<Self::Closed> + Clone,
        Self: Sized,
    {
        MergeAndCloseOnDrop {
            value: Some(self),
            target: sink.clone(),
        }
    }
}

impl<T, Sink: AggregateSink<T>> Drop for MergeOnDrop<T, Sink> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.target.merge(value);
        }
    }
}

impl<T, Sink> Drop for MergeAndCloseOnDrop<T, Sink>
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

/// Trait that aggregates items
pub trait AggregateSink<T> {
    /// Merge a given item into the sink
    fn merge(&self, entry: T);
}

/// Aggregation that coordinates access with a Mutex
pub struct MutexSink<T: SourceMetric> {
    aggregator: Arc<Mutex<Option<Aggregate<T>>>>,
    default_value: Arc<dyn Fn() -> T::Aggregated + Send + Sync>,
}

impl<T: SourceMetric> Clone for MutexSink<T> {
    fn clone(&self) -> Self {
        Self {
            aggregator: self.aggregator.clone(),
            default_value: self.default_value.clone(),
        }
    }
}

impl<T: SourceMetric> MutexSink<T> {
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

impl<T: SourceMetric> AggregateSink<T> for MutexSink<T> {
    fn merge(&self, entry: T) {
        let mut aggregator = self.aggregator.lock().unwrap();
        match &mut *aggregator {
            Some(v) => {
                v.add(&entry);
            }
            None => {
                let value = (self.default_value)();
                let mut agg = Aggregate::new(value);
                agg.add(&entry);
                *aggregator = Some(agg);
            }
        }
    }
}

impl<T: SourceMetric> CloseValue for MutexSink<T> {
    type Closed = Option<<Aggregate<T> as CloseValue>::Closed>;

    fn close(self) -> Self::Closed {
        self.aggregator.lock().unwrap().take().map(|v| v.close())
    }
}
