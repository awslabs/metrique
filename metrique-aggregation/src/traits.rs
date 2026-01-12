//! Traits for aggregation
//!
//! This module provides a composable aggregation system with three main layers:
//!
//! ## Field-level aggregation: [`AggregateValue`]
//!
//! Defines how individual field values are merged. For example, [`crate::value::Sum`] sums values,
//! while [`crate::histogram::Histogram`] collects values into distributions. This trait enables
//! compile-time type resolution:
//!
//! ```rust
//! use metrique_aggregation::value::Sum;
//! use metrique_aggregation::traits::AggregateValue;
//! type AggregatedType = <Sum as AggregateValue<u64>>::Aggregated;
//! //                     ^^^                   ^^
//! //                     Strategy              Input type
//! ```
//!
//! ## Entry-level aggregation: [`Merge`] and [`AggregateStrategy`]
//!
//! The [`Merge`] trait defines how complete metric entries are combined. It specifies:
//! - The accumulated type (`Merged`)
//! - How to create new accumulators (`new_merged`)
//! - How to merge entries into accumulators (`merge`)
//!
//! The [`AggregateStrategy`] trait ties together a source type with its merge behavior and
//! key extraction strategy. The `#[aggregate]` macro generates these implementations automatically.
//!
//! ## Key extraction: [`Key`]
//!
//! The [`Key`] trait extracts grouping keys from source entries, enabling keyed aggregation
//! where entries with the same key are merged together. Fields marked with `#[aggregate(key)]`
//! become part of the key.
//!
//! ## The [`Aggregate`] wrapper
//!
//! [`Aggregate<T>`] is the simplest way to aggregate data, typically used as a field in a larger struct.
//! It wraps an aggregated value and tracks the number of samples merged.

use metrique_core::{CloseEntry, CloseValue, InflectableEntry, NameStyle};
use std::hash::Hash;

use crate::value::NoKey;

/// Defines how individual field values are aggregated.
///
/// This trait operates at the field level, not the entry level. Each aggregation
/// strategy (Counter, Histogram, etc.) implements this trait for the types it can aggregate.
///
/// # Type Parameters
///
/// - `T`: The type of value being aggregated
///
/// # Associated Types
///
/// - `Aggregated`: The accumulated type (often same as `T`, but can differ for histograms)
///
/// # Example
///
/// ```rust
/// use metrique_aggregation::traits::AggregateValue;
/// use metrique_core::CloseValue;
///
/// // Average tracks sum and count to compute average
/// pub struct Avg;
///
/// pub struct AvgAccumulator {
///     sum: f64,
///     count: u64,
/// }
///
/// impl CloseValue for AvgAccumulator {
///     type Closed = f64;
///
///     fn close(self) -> f64 {
///         if self.count == 0 {
///             0.0
///         } else {
///             self.sum / self.count as f64
///         }
///     }
/// }
///
/// impl AggregateValue<f64> for Avg {
///     type Aggregated = AvgAccumulator;
///
///     fn insert(accum: &mut Self::Aggregated, value: f64) {
///         accum.sum += value;
///         accum.count += 1;
///     }
/// }
/// ```
pub trait AggregateValue<T> {
    /// The accumulated type (often same as T, but can differ for histograms).
    type Aggregated;

    /// Aggregate a value into the accumulator.
    fn insert(accum: &mut Self::Aggregated, value: T);
}

/// Key extraction trait for aggregation strategies.
///
/// Extracts grouping keys from source entries to enable keyed aggregation. Entries with
/// the same key are merged together. The `#[aggregate]` macro generates implementations
/// for fields marked with `#[aggregate(key)]`.
///
/// # Type Parameters
///
/// - `Source`: The type being aggregated
///
/// # Associated Types
///
/// - `Key<'a>`: The key type with lifetime parameter for borrowed data
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique_aggregation::traits::Key;
/// use std::borrow::Cow;
///
/// struct ApiCall {
///     endpoint: String,
///     latency: u64,
/// }
///
/// #[derive(Clone, Hash, PartialEq, Eq)]
/// #[metrics]
/// struct ApiCallKey<'a> {
///     endpoint: Cow<'a, String>,
/// }
///
/// struct ApiCallByEndpoint;
///
/// impl Key<ApiCall> for ApiCallByEndpoint {
///     type Key<'a> = ApiCallKey<'a>;
///
///     fn from_source(source: &ApiCall) -> Self::Key<'_> {
///         ApiCallKey {
///             endpoint: Cow::Borrowed(&source.endpoint),
///         }
///     }
///
///     fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
///         ApiCallKey {
///             endpoint: Cow::Owned(key.endpoint.clone().into_owned()),
///         }
///     }
///
///     fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool {
///         owned.endpoint == borrowed.endpoint
///     }
/// }
/// ```
pub trait Key<Source> {
    /// The key type with lifetime parameter
    type Key<'a>: Send + Hash + Eq + CloseEntry;
    /// Extract key from source
    fn from_source(source: &Source) -> Self::Key<'_>;
    /// Convert borrowed key to static lifetime
    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static>;

    /// Compare the static and reference versions of the key for equality.
    ///
    /// Why not use Equivalent or another trait?
    /// - Equivalent has a blanket impl that prevents it from being used
    /// - You can implement this by doing `owned == borrowed` on your concrete type, however the bounds seem hard to express
    fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool;
}

/// Defines how complete metric entries are merged together.
///
/// This trait operates at the entry level, combining entire structs rather than individual fields.
/// The `#[aggregate]` macro generates implementations that merge each field according to its
/// `#[aggregate(strategy = ...)]` attribute.
///
/// # Type Parameters
///
/// - `Self`: The source type being aggregated
///
/// # Associated Types
///
/// - `Merged`: The accumulated type that holds aggregated values
/// - `MergeConfig`: Configuration needed to create new merged values (often `()`)
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique_aggregation::traits::Merge;
/// use metrique_aggregation::histogram::Histogram;
/// use std::time::Duration;
///
/// struct ApiCall {
///     latency: Duration,
///     response_size: usize,
/// }
///
/// #[derive(Default)]
/// #[metrics]
/// struct AggregatedApiCall {
///     latency: Histogram<Duration>,
///     response_size: usize,
/// }
///
/// impl Merge for ApiCall {
///     type Merged = AggregatedApiCall;
///     type MergeConfig = ();
///
///     fn new_merged(_conf: &Self::MergeConfig) -> Self::Merged {
///         Self::Merged::default()
///     }
///
///     fn merge(accum: &mut Self::Merged, input: Self) {
///         accum.latency.add_value(&input.latency);
///         accum.response_size += input.response_size;
///     }
/// }
/// ```
pub trait Merge {
    /// The merged/accumulated type
    type Merged: CloseEntry;
    /// Configuration for creating new merged values
    type MergeConfig;
    /// Create a new merged value with configuration
    fn new_merged(conf: &Self::MergeConfig) -> Self::Merged;
    /// Create a new merged value using Default
    fn new_default_merged() -> Self::Merged
    where
        Self::Merged: Default,
    {
        Self::Merged::default()
    }
    /// Merge input into accumulator
    fn merge(accum: &mut Self::Merged, input: Self);
}

/// Borrowed version of [`Merge`] for more efficient aggregation.
///
/// When the source type can be borrowed during merging, it becomes possible
/// to aggregate the same input across multiple sinks (or to send it to multiple sinks.)
pub trait MergeRef: Merge {
    /// Merge borrowed input into accumulator
    fn merge_ref(accum: &mut Self::Merged, input: &Self);
}

/// Ties together source type, merge behavior, and key extraction.
///
/// This trait combines all the pieces needed for aggregation into a single strategy type.
/// The `#[aggregate]` macro generates an implementation automatically.
///
/// # Type Parameters
///
/// None - this is a marker trait that associates types
///
/// # Associated Types
///
/// - `Source`: The type being aggregated (must implement [`Merge`])
/// - `Key`: The key extraction strategy (must implement [`Key<Source>`])
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique_aggregation::traits::{AggregateStrategy, Key, Merge};
/// use metrique_aggregation::value::NoKey;
///
/// struct ApiCall {
///     latency: u64,
/// }
///
/// #[derive(Default)]
/// #[metrics]
/// struct AggregatedApiCall {
///     latency: u64,
/// }
///
/// impl Merge for ApiCall {
///     type Merged = AggregatedApiCall;
///     type MergeConfig = ();
///     fn new_merged(_: &()) -> Self::Merged { Self::Merged::default() }
///     fn merge(accum: &mut Self::Merged, input: Self) { accum.latency += input.latency; }
/// }
///
/// // Strategy type generated by #[aggregate]
/// struct ApiCallStrategy;
///
/// impl AggregateStrategy for ApiCallStrategy {
///     type Source = ApiCall;
///     type Key = NoKey;  // No key fields, aggregate everything together
/// }
/// ```
pub trait AggregateStrategy: 'static {
    /// The source type being aggregated
    type Source: Merge;
    /// The key extraction strategy
    type Key: Key<Self::Source>;
}

/// Type alias for the key type of an aggregation strategy.
pub type KeyTy<'a, T> =
    <<T as AggregateStrategy>::Key as Key<<T as AggregateStrategy>::Source>>::Key<'a>;

/// Type alias for the aggregated type of an aggregation strategy.
pub type AggregateTy<T> = <<T as AggregateStrategy>::Source as Merge>::Merged;

/// Result of keyed aggregation combining key and aggregated value.
///
/// Used by [`crate::keyed_sink::KeyedAggregationSink`] to emit aggregated entries
/// with their associated keys.
pub struct AggregationResult<K, Agg> {
    pub(crate) key: K,
    pub(crate) aggregated: Agg,
}

impl<Ns: NameStyle, A: InflectableEntry<Ns>, B: InflectableEntry<Ns>> InflectableEntry<Ns>
    for AggregationResult<A, B>
{
    fn write<'a>(&'a self, w: &mut impl metrique_writer::EntryWriter<'a>) {
        self.key.write(w);
        self.aggregated.write(w);
    }
}

impl<A: InflectableEntry, B: InflectableEntry> metrique_writer::Entry for AggregationResult<A, B> {
    fn write<'a>(&'a self, w: &mut impl metrique_writer::EntryWriter<'a>) {
        self.key.write(w);
        self.aggregated.write(w);
    }

    fn sample_group(
        &self,
    ) -> impl Iterator<Item = metrique_writer_core::entry::SampleGroupElement> {
        self.key
            .sample_group()
            .chain(self.aggregated.sample_group())
    }
}

/// Inline metrics aggregator
///
/// `Aggregate<T>` is designed to be used in a larger metrics struct to aggregate on an indiviual field.
///
/// It is not designed to be used when `Key` is present.
///
/// For thread-safe aggregation or more advanced patterns, see [`crate::sink::MutexAggregator`]
/// and [`crate::keyed_sink::KeyedAggregationSink`].
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique_aggregation::{aggregate, traits::Aggregate};
/// use metrique_aggregation::histogram::Histogram;
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

/// Root sink trait for thread-safe entry points (takes `&self`)
///
/// This is the trait that `WorkerSink` and other thread-safe wrappers implement.
/// Use this for `MergeOnDrop` and `CloseAndMergeOnDrop` targets.
pub trait RootSink<T> {
    /// Add an entry to be aggregated
    fn merge(&self, entry: T);
}

/// Trait for sinks that accept aggregated entries by value (takes `&mut self`)
///
/// This is for single-threaded aggregation contexts where exclusive access is available.
pub trait AggregateSink<T> {
    /// Add an entry to be aggregated
    fn merge(&mut self, entry: T);
}

/// Trait for sinks that accept aggregated entries by reference (takes `&mut self`)
///
/// This enables aggregating the same input across multiple sinks without cloning.
/// Requires the source type to implement `MergeRef`.
pub trait AggregateSinkRef<T: ?Sized> {
    /// Add an entry to be aggregated by reference
    fn merge_ref(&mut self, entry: &T);
}

/// Trait for sinks that can be flushed (takes `&mut self`)
pub trait FlushableSink {
    /// Flush all accumulated entries to the output sink
    fn flush(&mut self);
}
