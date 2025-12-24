//! Core traits for aggregating metrics.
//!
//! This module defines the trait system that enables type-safe, efficient aggregation
//! of metric entries. The system operates at two levels:
//!
//! 1. **Field-level aggregation** via [`AggregateValue`] - defines how individual field
//!    values combine (e.g., Counter sums, Histogram collects observations)
//! 2. **Entry-level aggregation** via [`AggregatableEntry`] and [`AggregatedEntry`] -
//!    defines how complete metric entries are accumulated
//!
//! # Example
//!
//! ```rust
//! use metrique_aggregation::aggregate::{AggregateValue, AggregatableEntry, AggregatedEntry};
//! use metrique_aggregation::Counter;
//! use metrique_writer::{Entry, EntryWriter};
//! use std::borrow::Cow;
//!
//! // Define a metric entry
//! struct RequestMetrics {
//!     operation: &'static str,
//!     request_count: u64,
//! }
//!
//! // Define the aggregated version
//! struct AggregatedRequestMetrics {
//!     key: &'static str,
//!     request_count: u64,
//! }
//!
//! // Implement Entry for both
//! impl Entry for RequestMetrics {
//!     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
//!         writer.value("Operation", &self.operation);
//!         writer.value("RequestCount", &self.request_count);
//!     }
//!     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
//!         std::iter::empty()
//!     }
//! }
//!
//! impl Entry for AggregatedRequestMetrics {
//!     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
//!         writer.value("Operation", &self.key);
//!         writer.value("RequestCount", &self.request_count);
//!     }
//!     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
//!         std::iter::empty()
//!     }
//! }
//!
//! // Implement aggregation traits
//! impl AggregatableEntry for RequestMetrics {
//!     type Key = &'static str;
//!     type Aggregated = AggregatedRequestMetrics;
//!
//!     fn new_aggregated(key: Self::Key) -> Self::Aggregated {
//!         AggregatedRequestMetrics {
//!             key,
//!             request_count: Counter::init(),
//!         }
//!     }
//!
//!     fn key(&self) -> Self::Key {
//!         self.operation
//!     }
//! }
//!
//! impl AggregatedEntry for AggregatedRequestMetrics {
//!     type Key = &'static str;
//!     type Source = RequestMetrics;
//!
//!     fn aggregate_into(&mut self, entry: &Self::Source) {
//!         Counter::aggregate(&mut self.request_count, &entry.request_count);
//!     }
//! }
//! ```

use metrique_writer::Entry;
use std::hash::Hash;

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
/// use metrique_aggregation::aggregate::AggregateValue;
/// use std::ops::AddAssign;
///
/// // Counter sums values
/// pub struct Counter;
///
/// impl<T: Default + AddAssign + Copy> AggregateValue<T> for Counter {
///     type Aggregated = T;
///     
///     fn init() -> T {
///         T::default()
///     }
///     
///     fn aggregate(accum: &mut T, value: &T) {
///         *accum += *value;
///     }
/// }
/// ```
pub trait AggregateValue<T> {
    /// The accumulated type (often same as T, but can differ for histograms).
    type Aggregated;

    /// Initialize a new accumulator.
    fn init() -> Self::Aggregated;

    /// Aggregate a value into the accumulator.
    fn aggregate(accum: &mut Self::Aggregated, value: &T);
}

/// Marks an entry type as capable of being aggregated.
///
/// This trait defines the key type that identifies which entries can be aggregated
/// together, and provides the associated aggregated type.
///
/// # Type Parameters
///
/// - `Key`: Type that identifies which entries merge together (must be `Eq + Hash + Clone`)
/// - `Aggregated`: The type that accumulates aggregated entries
///
/// # Example
///
/// ```rust
/// use metrique_aggregation::aggregate::{AggregatableEntry, AggregatedEntry};
/// use metrique_writer::{Entry, EntryWriter};
/// use std::borrow::Cow;
///
/// struct RequestMetrics {
///     operation: &'static str,
///     count: u64,
/// }
///
/// struct AggregatedRequestMetrics {
///     key: &'static str,
///     count: u64,
/// }
///
/// # impl Entry for RequestMetrics {
/// #     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {}
/// #     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
/// #         std::iter::empty()
/// #     }
/// # }
/// # impl Entry for AggregatedRequestMetrics {
/// #     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {}
/// #     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
/// #         std::iter::empty()
/// #     }
/// # }
/// # impl AggregatedEntry for AggregatedRequestMetrics {
/// #     type Key = &'static str;
/// #     type Source = RequestMetrics;
/// #     fn aggregate_into(&mut self, entry: &Self::Source) {}
/// # }
///
/// impl AggregatableEntry for RequestMetrics {
///     type Key = &'static str;
///     type Aggregated = AggregatedRequestMetrics;
///
///     fn new_aggregated(key: Self::Key) -> Self::Aggregated {
///         AggregatedRequestMetrics { key, count: 0 }
///     }
///
///     fn key(&self) -> Self::Key {
///         self.operation
///     }
/// }
/// ```
pub trait AggregatableEntry: Entry {
    /// Key type that identifies which entries can be aggregated together.
    type Key: Eq + Hash + Clone;

    /// The type that accumulates aggregated entries.
    type Aggregated: AggregatedEntry<Source = Self, Key = Self::Key>;

    /// Create a new aggregator for this entry type with the given key.
    fn new_aggregated(key: Self::Key) -> Self::Aggregated;

    /// Extract the key from this entry.
    fn key(&self) -> Self::Key;
}

/// Accumulates multiple entries and produces aggregated results.
///
/// This trait is typically implemented on the aggregated version of a metric struct.
/// It defines how to merge individual entries into the accumulated state.
///
/// # Type Parameters
///
/// - `Key`: The key type for this aggregated entry
/// - `Source`: The source entry type being aggregated
///
/// # Example
///
/// ```rust
/// use metrique_aggregation::aggregate::{AggregateValue, AggregatedEntry, AggregatableEntry};
/// use metrique_aggregation::Counter;
/// use metrique_writer::{Entry, EntryWriter};
/// use std::borrow::Cow;
///
/// struct RequestMetrics {
///     operation: &'static str,
///     count: u64,
/// }
///
/// struct AggregatedRequestMetrics {
///     key: &'static str,
///     count: u64,
/// }
///
/// # impl Entry for RequestMetrics {
/// #     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {}
/// #     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
/// #         std::iter::empty()
/// #     }
/// # }
/// # impl Entry for AggregatedRequestMetrics {
/// #     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {}
/// #     fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
/// #         std::iter::empty()
/// #     }
/// # }
/// # impl AggregatableEntry for RequestMetrics {
/// #     type Key = &'static str;
/// #     type Aggregated = AggregatedRequestMetrics;
/// #     fn new_aggregated(key: Self::Key) -> Self::Aggregated {
/// #         AggregatedRequestMetrics { key, count: 0 }
/// #     }
/// #     fn key(&self) -> Self::Key { self.operation }
/// # }
///
/// impl AggregatedEntry for AggregatedRequestMetrics {
///     type Key = &'static str;
///     type Source = RequestMetrics;
///
///     fn aggregate_into(&mut self, entry: &Self::Source) {
///         Counter::aggregate(&mut self.count, &entry.count);
///     }
/// }
/// ```
pub trait AggregatedEntry: Entry {
    /// The key type for this aggregated entry.
    type Key: Eq + Hash + Clone;

    /// The source entry type being aggregated.
    type Source: AggregatableEntry<Aggregated = Self, Key = Self::Key>;

    /// Aggregate another entry into this accumulator.
    fn aggregate_into(&mut self, entry: &Self::Source);
}
