//! Counter aggregation strategy.
//!
//! Counter sums values together, making it ideal for counts, totals, and accumulated metrics.

use crate::aggregate::AggregateValue;
use std::ops::AddAssign;

/// Counter aggregation strategy that sums values.
///
/// Use for request counts, error counts, bytes transferred, or any metric
/// where you want to sum values together.
pub struct Counter;

impl<T> AggregateValue<T> for Counter
where
    T: Default + AddAssign + Copy,
{
    type Aggregated = T;

    fn add_value(accum: &mut T, value: &T) {
        *accum += *value;
    }
}
