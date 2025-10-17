// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Aggregation support for combining multiple entries into a single entry.

use crate::Entry;

pub mod local_sink;
pub mod sink;
pub mod strategies;

pub use local_sink::LocalAggregatingEntrySink;
pub use sink::{AggregateConfig, AggregatingEntrySink};
pub use strategies::{Counter, Gauge, Max, Min, VecHistogram};

/// Defines how to aggregate individual field values.
///
/// This trait is used by the proc macro to generate aggregation implementations
/// for individual fields in a metrics struct.
pub trait AggregateValue<T> {
    /// The accumulated type. Often the same as T, but can differ (e.g., Histogram).
    type Aggregated;

    /// Initialize a new accumulator.
    fn init() -> Self::Aggregated;

    /// Aggregate a value into the accumulator.
    fn aggregate(accum: &mut Self::Aggregated, value: &T);
}

/// An entry that can be aggregated with other entries of the same type.
///
/// This enables in-memory aggregation of metrics before emission, reducing
/// the number of metric records sent to the backend.
pub trait AggregatableEntry: Entry {
    /// The key type that identifies which entries can be aggregated together.
    /// Use `()` for entries without keys.
    type Key: Eq + std::hash::Hash + Clone;

    /// The type that accumulates aggregated entries.
    type Aggregated: AggregatedEntry<Source = Self, Key = Self::Key>;

    /// Create a new aggregator for this entry type with the given key.
    fn new_aggregated(key: Self::Key) -> Self::Aggregated;

    /// Extract the key from this entry.
    fn key(&self) -> Self::Key;
}

/// Accumulates multiple entries and produces an aggregated result.
pub trait AggregatedEntry: Entry {
    /// The key type for this aggregated entry.
    type Key: Eq + std::hash::Hash + Clone;

    /// The source entry type being aggregated.
    type Source: AggregatableEntry<Aggregated = Self, Key = Self::Key>;

    /// Aggregate another entry into this accumulator.
    fn aggregate_into(&mut self, entry: &Self::Source);

    /// Get the number of entries aggregated so far.
    fn count(&self) -> usize;
}
