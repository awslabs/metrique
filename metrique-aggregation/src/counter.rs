//! Counter aggregation strategy.
//!
//! Counter sums values together, making it ideal for counts, totals, and accumulated metrics.
//!
//! # Example
//!
//! ```rust
//! use metrique_aggregation::aggregate::AggregateValue;
//! use metrique_aggregation::Counter;
//!
//! let mut total = Counter::init();
//! Counter::aggregate(&mut total, &5u64);
//! Counter::aggregate(&mut total, &3u64);
//! Counter::aggregate(&mut total, &2u64);
//! assert_eq!(total, 10);
//! ```

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

    fn init() -> T {
        T::default()
    }

    fn aggregate(accum: &mut T, value: &T) {
        *accum += *value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert2::check;

    #[test]
    fn counter_sums_u64() {
        let mut total = Counter::init();
        Counter::aggregate(&mut total, &5u64);
        Counter::aggregate(&mut total, &3u64);
        Counter::aggregate(&mut total, &2u64);
        check!(total == 10);
    }

    #[test]
    fn counter_sums_f64() {
        let mut total = Counter::init();
        Counter::aggregate(&mut total, &5.5f64);
        Counter::aggregate(&mut total, &3.2f64);
        Counter::aggregate(&mut total, &1.3f64);
        check!(total == 10.0);
    }

    #[test]
    fn counter_starts_at_zero() {
        let total: u64 = Counter::init();
        check!(total == 0);
    }
}
