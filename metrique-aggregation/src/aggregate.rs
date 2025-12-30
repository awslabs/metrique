//! Traits for aggregation
//!
//! There are three traits:
//! 1. [`AggregateValue`]: This defines how individual values are merged, for example, [`crate::counter::Counter`] defines that
//! values are summed. `Histogram` is a strategy that keeps track of values then emits buckets later. This trait exists
//! so that during macro expansion we can do:
//! ```rust
//!   use metrique_aggregation::{Counter, aggregate::AggregateValue};
//!   <Counter as AggregateValue<u64>>::Aggregated
//! // ^^^^^^^                   ^^
//! // Aggregation strategy      input type
//! // And produce the correct aggregate type at compile time
//! ```
//! 2. [`SourceMetric`]: A metric that can be merged into an accumulator. You can `impl` SourceMetric + `AccumulatorMetric` for
//!    an entire entry to define how it is merged.
//!
//! 3. [`AccumulatorMetric`]: A metric that accumulates metrics (usually of the same type)

use metrique_core::CloseEntry;
use metrique_core::CloseValue;

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
///     fn add_value(accum: &mut Self::Aggregated, value: &f64) {
///         accum.sum += value;
///         accum.count += 1;
///     }
/// }
/// ```
pub trait AggregateValue<T> {
    /// The accumulated type (often same as T, but can differ for histograms).
    type Aggregated;

    /// Aggregate a value into the accumulator.
    fn add_value(accum: &mut Self::Aggregated, value: &T);
}

/// Strategy for aggregating metrics
pub trait AggregateEntry {
    /// Source type. This is often `Self`
    type Source<'a>;

    /// Aggregated type
    type Aggregated;

    /// Aggregation Key. For structs with no key, you typically use `()`
    type Key<'a>;

    /// Merge a given entry into the Aggregate
    fn merge_entry<'a>(accum: &mut Self::Aggregated, entry: Self::Source<'a>);

    /// Create a new, empty, aggregated entry for a given key
    fn new_aggregated<'a>(key: Self::Key<'a>) -> Self::Aggregated;

    /// Returns the key for a given aggregate
    fn key<'a>(source: &'a Self::Source<'a>) -> Self::Key<'a>;
}

/// An atom that can be aggregated
pub trait SourceMetric: Sized {
    /// The type that accumulates aggregated entries.
    type Aggregated: AccumulatorMetric<Source = Self>;
}

/// A metric that accumlates `Source` metrics
pub trait AccumulatorMetric: CloseEntry {
    /// The source type for this accumulation
    type Source;
    /// Aggregate another entry into this accumulator.
    fn add_entry(&mut self, entry: &Self::Source);
}

/// Aggregated allows inline-aggregation of a metric
///
/// Aggregated is simple — more complex designs allow `append_on_drop` via a queue
/// or guard. Aggregate is a minimal version.
pub struct Aggregate<T: SourceMetric> {
    aggregated: T::Aggregated,
    num_samples: usize,
}

impl<T: SourceMetric> CloseValue for Aggregate<T> {
    type Closed = <T::Aggregated as CloseValue>::Closed;

    fn close(self) -> <Self as CloseValue>::Closed {
        self.aggregated.close()
    }
}

impl<T> Aggregate<T>
where
    T: SourceMetric,
{
    /// Add a new entry into this aggregate
    pub fn add(&mut self, entry: &T) {
        self.num_samples += 1;
        self.aggregated.add_entry(entry);
    }

    /// Creates a `Aggreate` that is initialized to a given value
    pub fn new(value: T::Aggregated) -> Self {
        Self {
            aggregated: value,
            num_samples: 0,
        }
    }
}

impl<T> Default for Aggregate<T>
where
    T: SourceMetric,
    T::Aggregated: Default,
{
    fn default() -> Self {
        Self {
            aggregated: T::Aggregated::default(),
            num_samples: 0,
        }
    }
}
