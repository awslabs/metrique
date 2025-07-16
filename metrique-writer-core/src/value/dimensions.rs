// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ops::{Deref, DerefMut};

use smallvec::SmallVec;

use crate::{
    CowStr, MetricFlags, MetricValue, Observation, Unit, ValidationError, Value, ValueWriter,
};

/// Adds a set of dimensions to a value as (class, instance) pairs.
///
/// This will not work in [EMF] unless [split entry] mode is enabled, and is probably not what you want in EMF
/// except for time-based metrics.
///
/// [EMF]: crate::format::emf
/// [split entry]: crate::format::emf::AllowSplitEntries
///
/// The const `N` defines how many of the pairs will be stored inline with the value before being spilled to the heap.
/// In most cases, the number of dimensions is known and setting `N` accordingly will avoid an allocation.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct WithDimensions<V, const N: usize> {
    value: V,
    dimensions: SmallVec<[(CowStr, CowStr); N]>,
}

impl<V, const N: usize> WithDimensions<V, N> {
    /// Map the value within this [WithDimensions]
    pub fn map_value<U>(self, f: impl Fn(V) -> U) -> WithDimensions<U, N> {
        WithDimensions {
            value: f(self.value),
            dimensions: self.dimensions,
        }
    }
}

/// Type alias of [`WithDimensions`] for the common case of adding a single (class, instance) pair.
///
/// This will not work in [EMF] unless [split entry] mode is enabled, and is probably not what you want in EMF
/// except for time-based metrics.
///
/// [EMF]: crate::format::emf
/// [split entry]: crate::format::emf::AllowSplitEntries
///
/// Note that more than one pair can be added, but they will trigger a spill to the heap.
pub type WithDimension<V> = WithDimensions<V, 1>;

/// Type alias of [`WithDimensions`] that will always store dimensions on the heap.
///
/// This will not work in [EMF] unless [split entry] mode is enabled, and is probably not what you want in EMF
/// except for time-based metrics.
///
/// [EMF]: crate::format::emf
/// [split entry]: crate::format::emf::AllowSplitEntries
pub type WithVecDimensions<V> = WithDimensions<V, 0>;

impl<V, const N: usize> Deref for WithDimensions<V, N> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<V, const N: usize> DerefMut for WithDimensions<V, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<V, const N: usize> From<V> for WithDimensions<V, N> {
    fn from(value: V) -> Self {
        Self {
            value,
            dimensions: Default::default(),
        }
    }
}

impl<V> WithDimension<V> {
    /// Add the (`class`, `instance`) dimension to `value`.
    pub fn new(value: V, class: impl Into<CowStr>, instance: impl Into<CowStr>) -> Self {
        Self::new_with_dimensions(value, [(class, instance)])
    }
}

impl<V, const N: usize> WithDimensions<V, N> {
    /// Creates a `WithDimensions` with no dimensions (similar to `WithDimensions::from()`) that can be used in `const` contexts
    pub const fn new_const(value: V) -> Self {
        Self {
            value,
            dimensions: SmallVec::new_const(),
        }
    }

    /// Add all of the given dimensions to `value`.
    ///
    /// Note that `N` should be chosen to match the upper bound length of `dimensions`. If the upper bound is unknown or
    /// large enough that it should always be heap allocated, `N` can be chosen to be 0 (see [`WithVecDimensions`]).
    pub fn new_with_dimensions<C, I>(value: V, dimensions: impl IntoIterator<Item = (C, I)>) -> Self
    where
        C: Into<CowStr>,
        I: Into<CowStr>,
    {
        Self {
            value,
            dimensions: dimensions
                .into_iter()
                .map(|(c, i)| (c.into(), i.into()))
                .collect(),
        }
    }

    /// The set of dimensions that this [WithDimensions] will add
    pub fn dimensions(&self) -> &[(CowStr, CowStr)] {
        &self.dimensions
    }

    /// Add a `(key, value)` to this [WithDimensions]
    pub fn add_dimension(&mut self, key: impl Into<CowStr>, value: impl Into<CowStr>) -> &mut Self {
        self.dimensions.push((key.into(), value.into()));
        self
    }

    /// Clear the dimensions in this [WithDimensions]. You can add
    /// new dimensions afterwards by using [Self::add_dimension].
    pub fn clear_dimensions(&mut self) {
        self.dimensions.clear()
    }
}

impl<V: MetricValue, const N: usize> Value for WithDimensions<V, N> {
    fn write(&self, writer: impl ValueWriter) {
        struct Wrapper<'a, W> {
            writer: W,
            dimensions: &'a [(CowStr, CowStr)],
        }

        impl<W: ValueWriter> ValueWriter for Wrapper<'_, W> {
            fn string(self, _value: &str) {
                self.invalid("can't apply dimensions to a string value");
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                #[allow(clippy::map_identity)]
                // https://github.com/rust-lang/rust-clippy/issues/9280
                self.writer.metric(
                    distribution,
                    unit,
                    dimensions
                        .into_iter()
                        .map(|(k, v)| (k, v)) // reborrow to align lifetimes
                        .chain(self.dimensions.iter().map(|(c, i)| (&**c, &**i))),
                    flags,
                )
            }

            fn error(self, error: ValidationError) {
                self.writer.error(error)
            }
        }

        self.value.write(Wrapper {
            writer,
            dimensions: self.dimensions(),
        })
    }
}

impl<V: MetricValue, const N: usize> MetricValue for WithDimensions<V, N> {
    type Unit = V::Unit;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        ValidationError, Value,
        unit::{Millisecond, UnitTag as _},
    };

    use super::*;

    #[test]
    fn adds_dimensions() {
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
                let distribution = distribution.into_iter().collect::<Vec<_>>();
                let dimensions = dimensions.into_iter().collect::<Vec<_>>();

                assert_eq!(distribution, &[Observation::Floating(42.0)]);
                assert_eq!(unit, Millisecond::UNIT);
                assert_eq!(dimensions, &[("foo", "bar")]);
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected error {error}");
            }
        }

        WithDimension::new(Duration::from_millis(42), "foo", "bar").write(Writer);
    }

    #[test]
    fn appends_after_existing_dimensions() {
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
                let distribution = distribution.into_iter().collect::<Vec<_>>();
                let dimensions = dimensions.into_iter().collect::<Vec<_>>();

                assert_eq!(distribution, &[Observation::Floating(42.0)]);
                assert_eq!(unit, Millisecond::UNIT);
                assert_eq!(dimensions, &[("foo", "bar"), ("a", "b"), ("c", "d")]);
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected error {error}");
            }
        }

        let existing = Duration::from_millis(42).with_dimension("foo", "bar");
        WithDimension::new_with_dimensions(existing, [("a", "b"), ("c", "d")]).write(Writer);
    }

    #[test]
    fn test_const_with_dimensions() {
        let empty_with_dimensions: WithDimensions<Duration, 1> =
            WithDimensions::new_const(Duration::from_millis(19));
        let from_with_dimensions = WithDimensions::from(Duration::from_millis(19));

        assert_eq!(empty_with_dimensions, from_with_dimensions);
    }
}
