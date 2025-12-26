//! Sink that aggregates entries before forwarding them.
//!
//! This module provides a minimal demonstration of how aggregation sinks work.
//! A full implementation would integrate with metrique's sink infrastructure.

use crate::aggregate::{AggregatableEntry, AggregatedEntry, DefaultKey, FromKey, Key};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Mutex;

/// Type-specific aggregating sink that aggregates entries of type `T`.
///
/// This demonstrates the core aggregation logic. A full implementation would
/// integrate with metrique's `EntrySink` trait and handle flushing, but this
/// shows the essential pattern.
///
/// # Example
///
/// ```rust
/// use metrique_aggregation::sink::TypedAggregatingEntrySink;
/// use metrique_aggregation::aggregate::{AggregateValue, AggregatableEntry, AggregatedEntry};
/// use metrique_aggregation::Counter;
/// use metrique_writer::{Entry, EntryWriter};
/// use std::borrow::Cow;
///
/// #[derive(Clone)]
/// struct TestMetrics {
///     operation: &'static str,
///     count: u64,
/// }
///
/// struct AggregatedTestMetrics {
///     key: &'static str,
///     count: u64,
/// }
///
/// impl Entry for TestMetrics {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.value("Operation", &self.operation);
///         writer.value("Count", &self.count);
///     }
///     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
///         std::iter::empty()
///     }
/// }
///
/// impl Entry for AggregatedTestMetrics {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.value("Operation", &self.key);
///         writer.value("Count", &self.count);
///     }
///     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
///         std::iter::empty()
///     }
/// }
///
/// impl AggregatableEntry for TestMetrics {
///     type Key = &'static str;
///     type Aggregated = AggregatedTestMetrics;
///     fn new_aggregated(key: Self::Key) -> Self::Aggregated {
///         AggregatedTestMetrics { key, count: Counter::init() }
///     }
///     fn key(&self) -> Self::Key {
///         self.operation
///     }
/// }
///
/// impl AggregatedEntry for AggregatedTestMetrics {
///     type Key = &'static str;
///     type Source = TestMetrics;
///     fn aggregate_into(&mut self, entry: &Self::Source) {
///         Counter::aggregate(&mut self.count, &entry.count);
///     }
/// }
///
/// // Create sink and aggregate entries
/// let sink = TypedAggregatingEntrySink::<TestMetrics>::new();
/// sink.append(TestMetrics { operation: "read", count: 5 });
/// sink.append(TestMetrics { operation: "read", count: 3 });
/// sink.append(TestMetrics { operation: "write", count: 2 });
///
/// // Drain aggregated results
/// let results = sink.drain();
/// assert_eq!(results.len(), 2);
/// ```
pub struct TypedAggregatingEntrySink<T, K>
where
    T: AggregatableEntry,
    K: Key<T>,
{
    state: Mutex<HashMap<K::Key, T::Aggregated>>,
    k: PhantomData<K>,
}

impl<T, K> TypedAggregatingEntrySink<T, K>
where
    T: AggregatableEntry,
    K: Key<T>,
    T::Aggregated: FromKey<K::Key>,
{
    /// Create a new typed aggregating sink.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            k: Default::default(),
        }
    }

    /// Append an entry, aggregating it with existing entries that have the same key.
    pub fn append(&self, entry: T) {
        let mut state = self.state.lock().unwrap();
        let key = K::key(&entry);

        state
            .entry(key.clone())
            .and_modify(|agg| agg.aggregate_into(&entry))
            .or_insert_with(|| {
                let mut agg = T::Aggregated::new_from_key(key);
                agg.aggregate_into(&entry);
                agg
            });
    }

    /// Drain all aggregated entries, returning them as a vector.
    ///
    /// This clears the internal state.
    pub fn drain(&self) -> Vec<T::Aggregated> {
        let mut state = self.state.lock().unwrap();
        state.drain().map(|(_key, agg)| agg).collect()
    }
}

impl<T> Default for TypedAggregatingEntrySink<T, T::KeyType>
where
    T: AggregatableEntry,
    T: DefaultKey,
    T::Aggregated: FromKey<<T::KeyType as Key<T>>::Key>,
    T::KeyType: Key<T>,
{
    fn default() -> Self {
        Self::new()
    }
}
