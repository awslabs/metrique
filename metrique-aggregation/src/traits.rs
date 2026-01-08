//! Traits for aggregation
//!
//! This module provides a two-level aggregation system:
//!
//! ## Field-level aggregation: [`AggregateValue`]
//!
//! Defines how individual field values are merged. For example, [`crate::value::Sum`] sums values,
//! while `Histogram` collects values into buckets. This trait enables compile-time type resolution:
//!
//! ```rust
//! use metrique_aggregation::value::Sum;
//! use metrique_aggregation::traits::AggregateValue;
//! type AggregatedType = <Sum as AggregateValue<u64>>::Aggregated;
//! //                     ^^^                   ^^
//! //                     Aggregation strategy  input type
//! ```
//!
//! ## Entry-level aggregation: [`AggregateEntry`]
//!
//! Defines how entire metric entries are merged together. Implement this trait to aggregate
//! complete metric structs, not just individual fields.
//!
//! ## The [`Aggregate`] wrapper
//!
//! [`Aggregate<T>`] is the simplest way to aggregate data, typically used as a field in a larger struct.
//! It wraps an aggregated value and tracks the number of samples merged.

use metrique_core::{CloseEntry, CloseValue, InflectableEntry, NameStyle};
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
///     fn add_value(accum: &mut Self::Aggregated, value: f64) {
///         accum.sum += value;
///         accum.count += 1;
///     }
/// }
/// ```
pub trait AggregateValue<T> {
    /// The accumulated type (often same as T, but can differ for histograms).
    type Aggregated;

    /// Aggregate a value into the accumulator.
    fn add_value(accum: &mut Self::Aggregated, value: T);
}

/// Key extraction trait for aggregation strategies
pub trait Key<Source> {
    /// The key type with lifetime parameter
    type Key<'a>: Send + Hash + Eq + CloseEntry;
    /// Extract key from source
    fn from_source(source: &Source) -> Self::Key<'_>;
    /// Convert borrowed key to static lifetime
    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static>;
}

/// Merge trait for aggregating values
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

/// A version of `Merge` where the input is borrowed
pub trait MergeRef: Merge {
    /// Merge input into accumulator
    fn merge_ref(accum: &mut Self::Merged, input: &Self);
}

/// Aggregation strategy combining source, merge, and key extraction
pub trait AggregateStrategy: 'static {
    /// The source type being aggregated
    type Source: Merge;
    /// The key extraction strategy
    type Key: Key<Self::Source>;
}

/// The key type for an aggregation strategy
pub type KeyTy<'a, T> =
    <<T as AggregateStrategy>::Key as Key<<T as AggregateStrategy>::Source>>::Key<'a>;

/// The aggregated type for an aggregation strategy
pub type AggregateTy<T> = <<T as AggregateStrategy>::Source as Merge>::Merged;

/// Merges two entries together by writing both
pub struct AggregationResult<K, Agg> {
    pub(crate) key: K,
    pub(crate) b: Agg,
}

impl<Ns: NameStyle, A: InflectableEntry<Ns>, B: InflectableEntry<Ns>> InflectableEntry<Ns>
    for AggregationResult<A, B>
{
    fn write<'a>(&'a self, w: &mut impl metrique_writer::EntryWriter<'a>) {
        self.key.write(w);
        self.b.write(w);
    }
}

impl<A: InflectableEntry, B: InflectableEntry> metrique_writer::Entry for AggregationResult<A, B> {
    fn write<'a>(&'a self, w: &mut impl metrique_writer::EntryWriter<'a>) {
        self.key.write(w);
        self.b.write(w);
    }

    fn sample_group(
        &self,
    ) -> impl Iterator<Item = metrique_writer_core::entry::SampleGroupElement> {
        self.key.sample_group().chain(self.b.sample_group())
    }
}

/// Aggregated allows inline-aggregation of a metric
///
/// Aggregated is simple — more complex designs allow `append_on_drop` via a queue
/// or guard. Aggregate is a minimal version.
pub struct Aggregate<T: AggregateStrategy> {
    aggregated: <T::Source as Merge>::Merged,
    num_samples: usize,
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
    pub fn add(&mut self, entry: T::Source)
    where
        T::Source: Merge,
    {
        self.num_samples += 1;
        T::Source::merge(&mut self.aggregated, entry);
    }

    /// Creates a `Aggreate` that is initialized to a given value
    pub fn new(value: <T::Source as Merge>::Merged) -> Self {
        Self {
            aggregated: value,
            num_samples: 0,
        }
    }
}

impl<T: AggregateStrategy> Default for Aggregate<T>
where
    <T::Source as Merge>::Merged: Default,
{
    fn default() -> Self {
        Self {
            aggregated: <T::Source as Merge>::Merged::default(),
            num_samples: 0,
        }
    }
}

/*/
#[cfg(test)]
mod test {
    use assert2::check;
    use metrique::{CloseValue, unit_of_work::metrics};
    use metrique_writer::test_util::test_metric;

    use crate::traits::{Aggregate, Merge};

    #[test]
    fn test_merge() {
        #[metrics]
        struct A {
            key_1: usize,
        }

        #[metrics]
        struct B {
            key_2: usize,
        }

        #[metrics(rename_all = "PascalCase")]
        struct RootMerge {
            #[metrics(flatten, no_close)]
            merge: Aggregate<<A as CloseValue>::Closed, <B as CloseValue>::Closed>,
        }

        let entry = RootMerge {
            merge: Aggregate {
                key: A { key_1: 1 }.close(),
                b: B { key_2: 10 }.close(),
            },
        };
        let entry = test_metric(entry);
        check!(entry.metrics["Key1"] == 1);
    }
}

*/
