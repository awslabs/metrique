//! Aggregation structures for collecting and merging entries

use metrique_core::CloseValue;
use std::marker::PhantomData;

use metrique::writer::BoxEntrySink;

use crate::traits::{
    AggregateSink, AggregateSinkRef, AggregateStrategy, AggregateTy, FlushableSink, Key, KeyTy,
    Merge, MergeRef,
};
use crate::value::NoKey;

/// The Entry type you have when merging entries
pub type AggregatedEntry<T> = crate::traits::AggregationResult<
    <<<T as AggregateStrategy>::Key as Key<<T as AggregateStrategy>::Source>>::Key<'static> as CloseValue>::Closed,
    <<<T as AggregateStrategy>::Source as Merge>::Merged as CloseValue>::Closed,
>;

/// Keyed aggregator that uses a HashMap to aggregate entries by key
///
/// This is the core aggregation logic without any threading or channel concerns.
pub struct KeyedAggregator<T: AggregateStrategy, Sink = BoxEntrySink> {
    storage: hashbrown::HashMap<KeyTy<'static, T>, AggregateTy<T>>,
    sink: Sink,
    _phantom: PhantomData<T>,
}

impl<T, Sink> KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    /// Create a new keyed aggregator
    pub fn new(sink: Sink) -> Self {
        Self {
            storage: Default::default(),
            sink,
            _phantom: PhantomData,
        }
    }
}

use hashbrown::hash_map::RawEntryMut;
use std::hash::BuildHasher;

impl<T, Sink> KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn get_or_create_accum<'a>(
        storage: &'a mut hashbrown::HashMap<KeyTy<'static, T>, AggregateTy<T>>,
        entry: &T::Source,
    ) -> &'a mut AggregateTy<T> {
        let borrowed_key = T::Key::from_source(entry);
        let hash = storage.hasher().hash_one(&borrowed_key);

        match storage
            .raw_entry_mut()
            .from_hash(hash, |k| T::Key::static_key_matches(k, &borrowed_key))
        {
            RawEntryMut::Occupied(occupied) => occupied.into_mut(),
            RawEntryMut::Vacant(vacant) => {
                let static_key = T::Key::static_key(&borrowed_key);
                let new_value = T::Source::new_merged(&Default::default());
                vacant.insert_hashed_nocheck(hash, static_key, new_value).1
            }
        }
    }
}

impl<T, Sink> AggregateSink<T::Source> for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn merge(&mut self, entry: T::Source) {
        let accum = Self::get_or_create_accum(&mut self.storage, &entry);
        T::Source::merge(accum, entry);
    }
}

impl<T, Sink> AggregateSinkRef<T::Source> for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    T::Source: MergeRef,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn merge_ref(&mut self, entry: &T::Source) {
        let accum = Self::get_or_create_accum(&mut self.storage, entry);
        T::Source::merge_ref(accum, entry);
    }
}

impl<T, Sink> FlushableSink for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn flush(&mut self) {
        for (key, aggregated) in self.storage.drain() {
            let merged = crate::traits::AggregationResult {
                key: key.close(),
                aggregated: aggregated.close(),
            };
            self.sink.append(merged);
        }
    }
}

/// Embedded aggregator for collecting multiple observations within a single unit of work
///
/// Use this when a single operation fans out to multiple sub-operations that you want to aggregate.
///
/// # Example
/// ```
/// use metrique::unit_of_work::metrics;
/// use metrique_aggregation::{aggregate, histogram::Histogram};
/// use metrique_aggregation::aggregator::Aggregate;
/// use std::time::Duration;
///
/// #[aggregate]
/// #[metrics]
/// struct ApiCall {
///     #[aggregate(strategy = Histogram<Duration>)]
///     latency: Duration,
/// }
///
/// #[metrics]
/// struct RequestMetrics {
///     request_id: String,
///     #[metrics(flatten)]
///     api_calls: Aggregate<ApiCall>,
/// }
///
/// let mut metrics = RequestMetrics {
///     request_id: "req-123".to_string(),
///     api_calls: Aggregate::default(),
/// };
///
/// metrics.api_calls.insert(ApiCall {
///     latency: Duration::from_millis(45),
/// });
/// metrics.api_calls.insert(ApiCall {
///     latency: Duration::from_millis(67),
/// });
/// ```
pub struct Aggregate<T: AggregateStrategy> {
    aggregated: <T::Source as Merge>::Merged,
}

impl<T: AggregateStrategy> CloseValue for Aggregate<T>
where
    <T::Source as Merge>::Merged: CloseValue,
{
    type Closed = <<T::Source as Merge>::Merged as CloseValue>::Closed;

    fn close(self) -> <Self as CloseValue>::Closed {
        self.aggregated.close()
    }
}

impl<T: AggregateStrategy> Aggregate<T> {
    /// Add a new entry into this aggregate
    pub fn insert(&mut self, entry: T)
    where
        T: CloseValue<Closed = T::Source>,
        T: AggregateStrategy<Key = NoKey>,
        T::Source: Merge,
    {
        T::Source::merge(&mut self.aggregated, entry.close());
    }

    /// Creates an `Aggregate` initialized to a given value.
    pub fn new(value: <T::Source as Merge>::Merged) -> Self {
        Self { aggregated: value }
    }
}

impl<T> AggregateSink<T::Source> for Aggregate<T>
where
    T: AggregateStrategy,
{
    fn merge(&mut self, entry: T::Source) {
        T::Source::merge(&mut self.aggregated, entry);
    }
}

impl<T> AggregateSinkRef<T::Source> for Aggregate<T>
where
    T: AggregateStrategy,
    T::Source: MergeRef,
{
    fn merge_ref(&mut self, entry: &T::Source) {
        T::Source::merge_ref(&mut self.aggregated, entry);
    }
}

/// Aggregates values without closing them (for raw mode)
pub struct AggregateRaw<T: AggregateStrategy> {
    aggregated: <T::Source as Merge>::Merged,
}

impl<T: AggregateStrategy> Default for AggregateRaw<T>
where
    <T::Source as Merge>::Merged: Default,
{
    fn default() -> Self {
        Self {
            aggregated: Default::default(),
        }
    }
}

impl<T: AggregateStrategy> CloseValue for AggregateRaw<T>
where
    <T::Source as Merge>::Merged: CloseValue,
{
    type Closed = <<T::Source as Merge>::Merged as CloseValue>::Closed;

    fn close(self) -> <Self as CloseValue>::Closed {
        self.aggregated.close()
    }
}

impl<T: AggregateStrategy> AggregateRaw<T> {
    /// Add a new entry into this aggregate without closing
    pub fn insert(&mut self, entry: T::Source)
    where
        T::Source: Merge,
    {
        T::Source::merge(&mut self.aggregated, entry);
    }

    /// Creates an `AggregateRaw` initialized to a given value.
    pub fn new(value: <T::Source as Merge>::Merged) -> Self {
        Self { aggregated: value }
    }
}

impl<T: AggregateStrategy> Default for Aggregate<T>
where
    <T::Source as Merge>::Merged: Default,
{
    fn default() -> Self {
        Self {
            aggregated: <T::Source as Merge>::Merged::default(),
        }
    }
}
