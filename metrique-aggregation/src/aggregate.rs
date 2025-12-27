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

/// An atom that can be aggregated
pub trait SourceMetric {
    /// The type that accumulates aggregated entries.
    type Aggregated: AccumulatorMetric<Source = Self>;
}

/// A metric that accumlates `Source` metrics
pub trait AccumulatorMetric: CloseEntry {
    /// The source entry type being aggregated.
    type Source: SourceMetric<Aggregated = Self>;

    /// Aggregate another entry into this accumulator.
    fn add_entry(&mut self, entry: &Self::Source);
}

/// Aggregated allows inline-aggregation of a metric
///
/// Aggregated is simple — more complex designs allow `append_on_drop` via a queue
/// or guard. Aggregate is a minimal version.
pub struct Aggregate<T: SourceMetric> {
    aggregated: Option<T::Aggregated>,
}

impl<T: SourceMetric> CloseValue for Aggregate<T> {
    type Closed = Option<<<T as SourceMetric>::Aggregated as CloseValue>::Closed>;

    fn close(self) -> <Self as CloseValue>::Closed {
        self.aggregated.map(|t| CloseValue::close(t))
    }
}

impl<T> Aggregate<T>
where
    T: SourceMetric,
    T::Aggregated: Default,
{
    /// Add a new entry into this aggregate
    pub fn add(&mut self, entry: T) {
        match &mut self.aggregated {
            Some(agg) => agg.add_entry(&entry),
            None => {
                let mut agg = T::Aggregated::default();
                agg.add_entry(&entry);
                self.aggregated = Some(agg);
            }
        }
    }
}

impl<T: SourceMetric> Default for Aggregate<T> {
    fn default() -> Self {
        Self { aggregated: None }
    }
}
