//! Mutex-based sink for thread-safe aggregation

use std::sync::{Arc, Mutex};

use metrique_core::CloseValue;

use crate::traits::{AggregateSink, RootSink};

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
    Inner: CloseValue + Default,
{
    type Closed = Inner::Closed;

    fn close(self) -> Self::Closed {
        let mut guard = self.inner.lock().unwrap();
        std::mem::take(&mut *guard).close()
    }
}
