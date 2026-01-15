// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    fmt::{self, Debug},
    marker::PhantomData,
};

use smallvec::SmallVec;

use metrique_writer_core::{
    MetricValue, Observation, Unit, ValidationError, ValidationErrorBuilder, Value, ValueWriter,
    unit::{self, UnitTag},
};

use metrique_writer_core::value::MetricFlags;

/// Collects a distribution of [`Value`]s that will be recorded individually under a single name.
///
/// <div class="warning">
///    This type is generally intended for internal use. If you want to record multiple values
///    with duplicates merged use <code>metrique_aggregation::value::Distribution</code>
///    or <code>metrique_aggregation::histogram::Histogram&lt;T, SortAndMerge&gt;</code>
/// </div>
///
/// For example,
/*
/// ```
/// # use metrique_writer::{value::Distribution, Entry, EntrySink, sink::VecEntrySink};
/// # let sink = VecEntrySink::default();
/// #[derive(Entry)]
/// struct MyEntry {
///    some_ints: Distribution<u32, 3>,
/// }
///
/// sink.append(MyEntry { some_ints: [1, 2, 3].into_iter().collect() });
/// ```
*/
/// will write `"some_ints": [1, 2, 3]` in EMF format. Not all formats support individual observation resolution and may
/// choose to instead report their sum.
///
/// If the distribution is empty, nothing will be written.
///
/// Each value in the distribution must have the same [`Unit`] and must not record any dimensions. Dimensions are
/// unsupported because it's unclear what to write if different values have different dimensions. To add dimensions to
/// the entire distribution, instead use
/// ```
/// # use metrique_writer::value::{Distribution, MetricValue};
/// let distribution = Distribution::<u32>::from_iter([1, 2, 3]);
/// distribution.with_dimension("foo", "bar");
/// ```
/// which will attach the dimension `"foo": "bar"` in EMF format (assuming `WithSplitEntries` is enabled).
///
/// `N` should be chosen to be as large as the maximum expected number of observations recorded. If there are more
/// observations, they will be spilled to the heap. If the maximum is unknown or very large, use `0` instead.
#[derive(Clone, Default, PartialEq)]
pub struct Distribution<V, const N: usize = 0> {
    values: SmallVec<[V; N]>,
}

/// Always records observations on the heap.
pub type VecDistribution<V> = Distribution<V, 0>;

impl<V: MetricValue, const N: usize> Value for Distribution<V, N> {
    fn write(&self, writer: impl ValueWriter) {
        if self.values.is_empty() {
            return;
        }

        let unit = <V::Unit as UnitTag>::UNIT;
        let mut observations = SmallVec::<[Observation; N]>::new();
        let mut collector = Collector {
            error: ValidationErrorBuilder::default(),
            expected_unit: unit,
            on_observation: |obs| {
                observations.push(obs);
                Ok(())
            },
        };
        for value in &self.values {
            value.write(&mut collector);
        }

        struct Collected<'a> {
            result: Result<&'a [Observation], ValidationError>,
            unit: Unit,
        }

        impl Value for Collected<'_> {
            fn write(&self, writer: impl ValueWriter) {
                match &self.result {
                    Ok(obs) => {
                        writer.metric(obs.iter().copied(), self.unit, [], MetricFlags::empty())
                    }
                    Err(err) => writer.error(err.clone()),
                }
            }
        }

        Collected {
            result: collector.error.build().map(|()| &observations[..]),
            unit,
        }
        .write(writer)
    }
}

impl<V: MetricValue, const N: usize> MetricValue for Distribution<V, N> {
    type Unit = V::Unit;
}

// Utility struct to get observations out of impl Value

#[derive(Default)]
struct Collector<F> {
    error: ValidationErrorBuilder,
    expected_unit: Unit,
    on_observation: F,
}

impl<F: FnMut(Observation) -> Result<(), ValidationError>> ValueWriter for &'_ mut Collector<F> {
    fn string(self, _value: &str) {
        self.invalid("can't construct a distribution of strings")
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        if unit != self.expected_unit {
            self.invalid(format!(
                "value promised to write unit `{}` but wrote `{unit}` instead",
                self.expected_unit
            ));
        } else if dimensions.into_iter().next().is_some() {
            self.invalid("dimensions must be added after collecting into a distribution");
        } else {
            for obs in distribution {
                if let Err(e) = (self.on_observation)(obs) {
                    self.error(e);
                }
            }
        }
    }

    fn error(self, error: ValidationError) {
        self.error.extend_mut(error);
    }
}

impl<V: MetricValue, const N: usize> FromIterator<V> for Distribution<V, N> {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl<V: MetricValue, const N: usize> Extend<V> for Distribution<V, N> {
    fn extend<T: IntoIterator<Item = V>>(&mut self, iter: T) {
        self.values.extend(iter)
    }
}

impl<V: MetricValue, const N: usize> IntoIterator for Distribution<V, N> {
    type Item = V;
    type IntoIter = smallvec::IntoIter<[V; N]>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<V: Debug, const N: usize> fmt::Debug for Distribution<V, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Distribution").field(&self.values).finish()
    }
}

impl<V: MetricValue, const N: usize> Distribution<V, N> {
    /// Return the list of values in this distribution
    pub fn values(&self) -> &[V] {
        &self.values
    }

    /// Add a a value to the distribution
    pub fn add(&mut self, value: V) -> &mut Self {
        self.values.push(value);
        self
    }

    /// Clear the values in this distribution, making it empty
    pub fn clear(&mut self) {
        self.values.clear()
    }

    /// Try to return turn this distribution into a [Mean], which will
    /// be recorded as a total / occurrences pair.
    ///
    /// See validation rules in [Mean::try_new]
    pub fn try_to_mean(&self) -> Result<Mean<V::Unit>, ValidationError> {
        Mean::try_new(self.values())
    }
}

/// Record the mean value of a distribution of observations.
///
/// This struct tracks both the total value and the number of observations that occurred. Some formats, like EMF,
/// support writing this as `mean*occurrences` to include the weight of the average, while others will only report the
/// mean value.
///
/// If the distribution is empty, nothing will be written.
pub struct Mean<U = unit::None> {
    total: f64,
    occurrences: u64,
    _unit: PhantomData<U>,
}

impl<U: UnitTag> Default for Mean<U> {
    fn default() -> Self {
        Self {
            total: 0.0,
            occurrences: 0,
            _unit: PhantomData,
        }
    }
}

impl<U> Clone for Mean<U> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<U> Copy for Mean<U> {}

impl<U> PartialEq for Mean<U> {
    fn eq(&self, other: &Self) -> bool {
        self.total == other.total && self.occurrences == other.occurrences
    }
}

impl<U: UnitTag> fmt::Debug for Mean<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mean")
            .field("total", &self.total)
            .field("occurrences", &self.occurrences)
            .field("unit", &U::UNIT)
            .finish()
    }
}

impl<U: UnitTag, V: Into<f64>> FromIterator<V> for Mean<U> {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let mut mean = Self::default();
        mean.extend(iter);
        mean
    }
}

impl<U: UnitTag> Mean<U> {
    /// Compute the mean from a distribution of [`MetricValue`]s.
    ///
    /// An empty [Mean] is fine, and will lead to no metric being recorded.
    ///
    /// Each value in the distribution must have the same [`Unit`] and must not record any dimensions. Dimensions are
    /// unsupported because it's unclear what to write if different values have different dimensions. To add dimensions
    /// to the entire distribution, instead use
    /// ```
    /// # use metrique_writer::value::{Mean, MetricValue};
    /// # use std::time::Duration;
    /// let mean = Mean::try_new([&Duration::from_millis(42), &Duration::from_millis(98)]).unwrap();
    /// mean.with_dimension("foo", "bar");
    /// ```
    pub fn try_new<'a, V: 'a + MetricValue<Unit = U>>(
        values: impl IntoIterator<Item = &'a V>,
    ) -> Result<Self, ValidationError> {
        let mut mean = Self::default();
        mean.try_extend(values)?;
        Ok(mean)
    }

    /// Return the total sum of observations
    pub fn total(self) -> f64 {
        self.total
    }

    /// Return the number of occurrences recorded
    pub fn occurrences(self) -> u64 {
        self.occurrences
    }

    /// Will return [`None`] if no observations have been recorded yet.
    pub fn mean(self) -> Option<f64> {
        match self.occurrences {
            0 => None,
            n => Some(self.total / (n as f64)),
        }
    }

    /// Record a new observation into this [Mean].
    pub fn record(&mut self, f: impl Into<f64>) {
        self.total += f.into();
        self.occurrences += 1;
    }

    /// See validation rules in [`Mean::try_new`].
    pub fn record_value(&mut self, value: &impl Value) -> Result<(), ValidationError> {
        let mut collector = Collector {
            error: ValidationErrorBuilder::default(),
            expected_unit: U::UNIT,
            on_observation: |obs| match obs {
                Observation::Unsigned(u) => {
                    self.total += u as f64;
                    self.occurrences += 1;
                    Ok(())
                }
                Observation::Floating(f) => {
                    self.total += f;
                    self.occurrences += 1;
                    Ok(())
                }
                Observation::Repeated { total, occurrences } => {
                    self.total += total;
                    self.occurrences += occurrences;
                    Ok(())
                }
                _ => Err(ValidationError::invalid("unknown observation type")),
            },
        };
        value.write(&mut collector);

        collector.error.build()
    }

    /// Add the total and number of occurrences from one `Mean` to another.
    pub fn add(&mut self, source: &Mean<U>) {
        self.total += source.total;
        self.occurrences += source.occurrences;
    }

    /// See validation rules in [`Mean::try_new`].
    pub fn try_extend<'a, V: 'a + MetricValue<Unit = U>>(
        &mut self,
        values: impl IntoIterator<Item = &'a V>,
    ) -> Result<(), ValidationError> {
        for v in values {
            self.record_value(v)?;
        }
        Ok(())
    }
}

impl<U: UnitTag, V: Into<f64>> Extend<V> for Mean<U> {
    fn extend<T: IntoIterator<Item = V>>(&mut self, iter: T) {
        for v in iter {
            self.record(v.into());
        }
    }
}

impl<U: UnitTag> Value for Mean<U> {
    fn write(&self, writer: impl ValueWriter) {
        if self.occurrences > 0 {
            writer.metric(
                [Observation::Repeated {
                    total: self.total,
                    occurrences: self.occurrences,
                }],
                U::UNIT,
                [],
                MetricFlags::empty(),
            );
        }
    }
}

impl<U: UnitTag> MetricValue for Mean<U> {
    type Unit = U;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn distribution_interchange_with_iterators() {
        let mut distribution = VecDistribution::from_iter([1u32, 2, 3, 4]);
        distribution.extend([5, 6, 7, 8]);
        assert!(distribution.into_iter().eq([1, 2, 3, 4, 5, 6, 7, 8]));
    }

    #[test]
    fn distribution_records_individual_observations() {
        let distribution = VecDistribution::from_iter([1u32, 2, 3]);
        assert_eq!(distribution.values(), &[1, 2, 3]);
        assert_eq!(distribution.try_to_mean().unwrap().mean().unwrap(), 2.0);

        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                assert_eq!(unit, Unit::None);
                assert!(dimensions.into_iter().next().is_none());
                assert_eq!(
                    Vec::from_iter(distribution),
                    &[
                        Observation::Unsigned(1),
                        Observation::Unsigned(2),
                        Observation::Unsigned(3)
                    ]
                );
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected {error}");
            }
        }
        distribution.write(Writer);
    }

    #[test]
    fn distribution_writes_nothing_with_no_observations() {
        let distribution = VecDistribution::<u32>::default();
        assert_eq!(distribution.values(), &[] as &[u32]);
        assert_eq!(distribution.try_to_mean().unwrap().mean(), None);

        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                _distribution: impl IntoIterator<Item = Observation>,
                _unit: Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                panic!("shouldn't have written a metric");
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected {error}");
            }
        }
        distribution.write(Writer);
    }

    #[test]
    fn mean_sums_individual_observations() {
        // using f64 directly
        let mean = Mean::<unit::Millisecond>::from_iter([1, 2, 3]);

        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                assert_eq!(unit, unit::Millisecond::UNIT);
                assert!(dimensions.into_iter().next().is_none());
                assert_eq!(
                    Vec::from_iter(distribution),
                    &[Observation::Repeated {
                        total: 6.0,
                        occurrences: 3
                    }]
                );
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected {error}");
            }
        }
        mean.write(Writer);

        // using values
        let mean = Mean::try_new([
            &Duration::from_millis(1),
            &Duration::from_millis(2),
            &Duration::from_millis(3),
        ])
        .unwrap();
        mean.write(Writer);
    }

    #[test]
    fn mean_writes_nothing_with_no_observations() {
        let mean = Mean::<unit::None>::default();

        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                _distribution: impl IntoIterator<Item = Observation>,
                _unit: Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                panic!("shouldn't have written a metric");
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected {error}");
            }
        }
        mean.write(Writer);
    }

    #[test]
    fn mean_add() {
        let mut mean_main = Mean::<unit::Millisecond>::default();
        let mean_source = Mean::try_new([
            &Duration::from_millis(1),
            &Duration::from_millis(2),
            &Duration::from_millis(3),
        ])
        .unwrap();

        mean_main.add(&mean_source);

        assert_eq!(mean_main.total(), mean_source.total());
        assert_eq!(mean_main.occurrences(), mean_source.occurrences());

        let mut mean_main = Mean::<unit::Millisecond>::default();
        mean_main.record_value(&Duration::from_millis(10)).unwrap();
        let mean_source = Mean::<unit::Millisecond>::default();

        mean_main.add(&mean_source);

        assert_eq!(mean_main.total(), 10.0);
        assert_eq!(mean_main.occurrences(), 1);
    }
}
