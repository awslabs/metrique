// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Histogram class to record a distribution of values

use std::ops::RangeInclusive;

use histogram::AtomicHistogram;
use metrics::HistogramFn;

/// A histogram with known-good configuration and supporting of parallel insertion and draining.
///
/// This normally uses `histogram::Config::new(4, 32)` - 32-bit range and 16 buckets
/// per binary order of magnitude (tracking error = 6.25%). You could call it
/// a floating-point number with a 1+4-bit mantissa and an exponent running in [4, 32) + denormals
/// (using the usual convention of a mantissa between 1 and 2). However, I don't think
/// the histogram crate describes this bucketing as stable.
pub struct Histogram {
    inner: histogram::AtomicHistogram,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    /// Creates a default histogram instance
    pub fn new() -> Self {
        let standard_config = Self::default_configuration();
        Self {
            inner: AtomicHistogram::with_config(&standard_config),
        }
    }

    fn default_configuration() -> histogram::Config {
        histogram::Config::new(4, 32).expect("known good configuration")
    }

    /// Records an occurrence of a value in the histogram.
    pub fn record(&self, value: u32) {
        self.inner
            .add(value as u64, 1)
            .expect("known within bounds because of type");
    }

    /// Returns an iterator providing the value and count of each bucket of the histogram.
    /// Only non-empty buckets are returned.
    /// During the iteration, the histogram counts are atomically reset to zero.
    pub(crate) fn drain(&self) -> Vec<Bucket> {
        self.inner
            .drain()
            .into_iter()
            .filter(|bucket| bucket.count() > 0)
            .map(|bucket| Bucket {
                value: midpoint(bucket.range()) as u32,
                count: bucket.count() as u32,
            })
            // TODO: We need to upstream a change to `histogram` to fix `into_iter`
            .collect::<Vec<_>>()
    }
}

fn midpoint(range: RangeInclusive<u64>) -> u64 {
    let size = range.end() - range.start();
    range.start() + size / 2
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
/// A histogram bucket
pub struct Bucket {
    /// Value is the midpoint of the bucket
    pub value: u32,
    /// Counts of entries within the bucket
    pub count: u32,
}

impl HistogramFn for Histogram {
    fn record(&self, value: f64) {
        if value > u32::MAX as f64 {
            self.record(u32::MAX);
        } else {
            self.record(value as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Histogram;
    use metrics::HistogramFn;
    use rand::{RngCore, rng};

    use super::Bucket;

    #[test]
    fn test_number_of_buckets() {
        let standard_config = Histogram::default_configuration();
        assert_eq!(standard_config.total_buckets(), 464);
    }

    #[test]
    fn test_record_value_multiple_times() {
        let histogram = Histogram::default();

        // Record value 0 50 times
        for _ in 0..50 {
            histogram.record(0);
        }

        // Record value 10 100 times
        for _ in 0..100 {
            histogram.record(10);
        }

        // Record value 11 200 times
        for _ in 0..200 {
            histogram.record(11);
        }

        // Record value 1000 300 times
        for _ in 0..300 {
            histogram.record(1000);
        }

        // Record value 1001 300 times (same bucket as before)
        for _ in 0..300 {
            histogram.record(1001);
        }

        // Check histogram values resetting
        assert_eq!(
            vec![(0, 50), (10, 100), (11, 200), (1007, 600)],
            buckets(histogram.drain())
        );

        // Check histogram values read-only again, the histogram should be empty
        assert_eq!(0, histogram.drain().len());
    }

    fn buckets(iter: impl IntoIterator<Item = Bucket>) -> Vec<(u32, u32)> {
        iter.into_iter()
            .map(|bucket| (bucket.value, bucket.count))
            .collect()
    }

    #[test]
    fn test_value_recorded() {
        let histogram = Histogram::default();

        // Values from 0 to 32 are in their own buckets
        for i in 0..32 {
            assert_eq!(i, recorded_value(&histogram, i));
        }
        // Values from 32 to 64 are 2 by bucket
        for i in 32..64 {
            assert_eq!(i / 2 * 2, recorded_value(&histogram, i));
        }
        // Values from 64 to 128 are 4 by bucket
        for i in 64..128 {
            assert_eq!(i / 4 * 4 + 1, recorded_value(&histogram, i));
        }
        // Values from 128 to 256 are 8 by bucket
        for i in 128..256 {
            assert_eq!(i / 8 * 8 + 3, recorded_value(&histogram, i));
        }
        // Values from 256 to 512 are 16 by bucket
        for i in 256..512 {
            assert_eq!(i / 16 * 16 + 7, recorded_value(&histogram, i));
        }
    }

    /// Checks that all values are recorded with a precision of more than 1/2^4
    #[test]
    fn test_accuracy() {
        let histogram = Histogram::default();

        let mut min_accuracy: f64 = 0.0;
        for i in (0..5_000) // First 5000
            .chain((u32::MAX - 5_000)..u32::MAX) // Last 5000
            .chain((u32::MAX / 2 - 2_500)..(u32::MAX / 2 + 2_500)) // Middle 5000
            .chain((0..5_000).map(|_| rng().next_u32()))
        // 5000 random
        {
            let val = recorded_value(&histogram, i);

            // Zero is a special case
            if i == 0 {
                assert_eq!(0, val);
                continue;
            }

            // Compute accuracy
            let accuracy: f64 = (val as f64 / i as f64 - 1.0).abs();
            assert!(
                accuracy <= 1.0 / 16.0 / 2.0,
                "{:?} > {:?}",
                accuracy,
                1.0 / 16.0 / 2.0
            );
            min_accuracy = min_accuracy.max(accuracy);
        }
        println!("Min accuracy = {}%", min_accuracy * 100.0);
    }

    /// Records a value in a histogram and returns the bucket value it was recorded at.
    fn recorded_value(histogram: &Histogram, value: u32) -> u32 {
        // Record value
        histogram.record(value);

        // Check the index that was used
        let mut recorded_value: Option<u32> = None;
        for Bucket { value, count } in histogram.drain() {
            assert_eq!(1, count);
            assert!(recorded_value.is_none());
            recorded_value = Some(value);
        }
        assert!(recorded_value.is_some());
        recorded_value.unwrap()
    }

    #[test]
    fn large_values_are_capped() {
        let h = Histogram::new();
        (&h as &dyn HistogramFn).record(f64::MAX);
        // large values are truncated to u32::MAX
        assert_eq!(
            h.drain(),
            vec![Bucket {
                value: 4227858432,
                count: 1
            }]
        );
    }
}
