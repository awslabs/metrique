// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Cow,
    ops::{Deref, DerefMut},
    time::SystemTime,
};

use smallvec::SmallVec;

use crate::{
    CowStr, Entry, EntryConfig, EntryWriter, MetricFlags, MetricValue, Observation, Unit,
    ValidationError, Value, ValueWriter,
};

/// Adds a set of dimensions to a [Value] or [Entry] as (class, instance) pairs.
///
/// This will not work in [EMF] unless [split entry] mode is enabled - which is
/// normally not recommended in [EMF], since it loses the association between
/// different metrics in the same entry ([split entry] mode is normally used only
/// when an [Entry] represents a collection of independent metrics that is
/// collected periodically, as in the [metrics.rs integration]).
///
/// [EMF]: https://docs.rs/metrique-writer-format-emf/0.1/metrique_writer_format_emf/
/// [metrics.rs integration]: https://docs.rs/metrique-metricsrs/0.1/metrique_metricsrs/
/// [split entry]: crate::config::AllowSplitEntries
///
/// The const `N` defines how many of the pairs will be stored inline with the value before being spilled to the heap.
/// In most cases, the number of dimensions is known and setting `N` accordingly will avoid an allocation. It *is*
/// perfectly valid to pass either more or less than `N` dimensions in (though passing more than `N` will require
/// an heap allocation).
///
/// # Examples
///
/// ## Simple use
///
/// Using `metrique::unit_of_work::metrics`:
///
/// ```no_run
/// use metrique::ServiceMetrics;
/// use metrique::unit_of_work::metrics;
/// use metrique::writer::{GlobalEntrySink, MetricValue};
/// use metrique::writer::value::WithDimension;
///
/// #[metrics(subfield)]
/// struct EggCounter {
///     number_of_eggs: u32,
/// }
///
/// #[metrics]
/// struct MyEntry {
///     number_of_ducks: WithDimension<u32>,
///     #[metrics(flatten)]
///     egg_counter: WithDimension<EggCounter>,
/// }
///
/// let mut entry = MyEntry {
///     number_of_ducks: 0u32.with_dimension("Operation", "CountDucks"),
///     // for nested entries, use the constructor instead of `.with_dimension`
///     egg_counter:
///         WithDimension::new(EggCounter { number_of_eggs: 0 }, "Operation", "CountDucks"),
/// }.append_on_drop(ServiceMetrics::sink());
///
/// // WithDimensions implements Deref and DerefMut
/// *entry.number_of_ducks += 1;
/// entry.egg_counter.number_of_eggs += 2;
/// ```
///
/// ## Simple use (`Entry` API)
///
/// Using the `metrique_writer::Entry` API:
///
/// ```no_run
/// use metrique::ServiceMetrics;
/// use metrique_writer::{Entry, EntrySink, GlobalEntrySink, MetricValue};
/// use metrique_writer::value::WithDimension;
///
/// #[derive(Entry)]
/// struct EggCounter {
///     number_of_eggs: u32,
/// }
///
/// #[derive(Entry)]
/// struct MyEntry {
///     number_of_ducks: WithDimension<u32>,
///     #[entry(flatten)]
///     egg_counter: WithDimension<EggCounter>,
/// }
///
/// let mut entry = ServiceMetrics::sink().append_on_drop(MyEntry {
///     number_of_ducks: 0u32.with_dimension("Operation", "CountDucks"),
///     // for nested entries, use the constructor instead of `.with_dimension`
///     egg_counter:
///         WithDimension::new(EggCounter { number_of_eggs: 0 }, "Operation", "CountDucks"),
/// });
///
/// // WithDimensions implements Deref and DerefMut
/// *entry.number_of_ducks += 1;
/// entry.egg_counter.number_of_eggs += 2;
/// ```
///
/// ## Use with a dynamic number of dimensions
///
/// It is also possible to use `WithDimensions` with a dynamic number of dimensions. In order
/// to avoid allocations, make `N` the maximal number of possible dimensions.
///
/// For example:
/// ```no_run
/// use metrique::ServiceMetrics;
/// use metrique::unit_of_work::metrics;
/// use metrique::writer::GlobalEntrySink;
/// use metrique::writer::value::WithDimensions;
///
/// #[metrics]
/// struct MyEntry {
///     // always have a Year dimension, may have Season dimension
///     number_of_ducks: WithDimensions<u32, 2>,
/// }
///
/// // You can use a String as a dimension (tho creating the String is an
/// // allocation).
/// fn current_year() -> String {
///     "2025".to_string()
/// }
///
/// fn current_season() -> Option<&'static str> {
///     // get the (possibly-unknown) season
///     Some("Spring")
/// }
///
/// let mut entry = MyEntry {
///     // default constructor 0 dimensions
///     number_of_ducks: Default::default(),
/// }.append_on_drop(ServiceMetrics::sink());
///
/// // WithDimensions implements Deref and DerefMut
/// *entry.number_of_ducks += 1;
///
/// // add the dimensions
/// entry.number_of_ducks.add_dimension("Year", current_year());
/// if let Some(season) = current_season() {
///     entry.number_of_ducks.add_dimension("Season", season);
/// }
/// ```
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
/// This will not work in [EMF] unless [split entry] mode is enabled - which is
/// normally not recommended in [EMF], since it loses the association between
/// different metrics in the same entry ([split entry] mode is normally used only
/// when an [Entry] represents a collection of independent metrics that is
/// collected periodically, as in the [metrics.rs integration]).
///
/// [EMF]: https://docs.rs/metrique-writer-format-emf/0.1/metrique_writer_format_emf/
/// [metrics.rs integration]: https://docs.rs/metrique-metricsrs/0.1/metrique_metricsrs/
/// [split entry]: crate::config::AllowSplitEntries
///
/// Note that more than one pair can be added, but they will trigger a spill to the heap.
pub type WithDimension<V> = WithDimensions<V, 1>;

/// Type alias of [`WithDimensions`] that will always store dimensions on the heap.
///
/// This will not work in [EMF] unless [split entry] mode is enabled - which is
/// normally not recommended in [EMF], since it loses the association between
/// different metrics in the same entry ([split entry] mode is normally used only
/// when an [Entry] represents a collection of independent metrics that is
/// collected periodically, as in the [metrics.rs integration]).
///
/// [EMF]: https://docs.rs/metrique-writer-format-emf/0.1/metrique_writer_format_emf/
/// [metrics.rs integration]: https://docs.rs/metrique-metricsrs/0.1/metrique_metricsrs/
/// [split entry]: crate::config::AllowSplitEntries
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

    /// Allow wrapping an [EntryWriter]
    pub fn entry_writer_wrapper<'a, 'b, W: EntryWriter<'b>>(
        &'a self,
        writer: W,
    ) -> impl EntryWriter<'b> + use<'a, 'b, W, V, N> {
        Wrapper {
            value: writer,
            dimensions: &self.dimensions,
        }
    }
}

#[derive(Debug)]
struct Wrapper<'a, V> {
    value: V,
    dimensions: &'a [(CowStr, CowStr)],
}

impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for Wrapper<'_, W> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.value.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        self.value.value(
            name,
            &Wrapper {
                value,
                dimensions: self.dimensions,
            },
        )
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.value.config(config);
    }
}

impl<V: Value> Value for Wrapper<'_, V> {
    fn write(&self, writer: impl ValueWriter) {
        self.value.write(Wrapper {
            value: writer,
            dimensions: self.dimensions,
        })
    }
}

impl<W: ValueWriter> ValueWriter for Wrapper<'_, W> {
    fn string(self, value: &str) {
        // dimensions are ignored for strings
        self.value.string(value);
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
        self.value.metric(
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
        self.value.error(error)
    }
}

impl<V: Value, const N: usize> Value for WithDimensions<V, N> {
    fn write(&self, writer: impl ValueWriter) {
        self.value.write(Wrapper {
            value: writer,
            dimensions: self.dimensions(),
        })
    }
}

impl<V: MetricValue, const N: usize> MetricValue for WithDimensions<V, N> {
    type Unit = V::Unit;
}

impl<E: Entry, const N: usize> Entry for WithDimensions<E, N> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        self.value.write(&mut self.entry_writer_wrapper(writer))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use metrique_writer::{
        Entry, EntryConfig, EntryWriter, MetricFlags, Observation, Unit, ValidationError, Value,
        ValueWriter,
        unit::{Millisecond, UnitTag as _},
        value::MetricValue,
        value::{WithDimension, WithDimensions},
    };

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
    fn runs_on_entries() {
        #[derive(Entry)]
        struct TestEntry {
            #[entry(timestamp)]
            ts: SystemTime,

            #[entry(flatten)]
            config: TestConfigEntry,

            f1: Duration,
            f2: Duration,
        }

        #[derive(Debug)]
        struct TestConfig;
        impl EntryConfig for TestConfig {}
        struct TestConfigEntry;
        impl Entry for TestConfigEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.config(&TestConfig);
            }
        }

        let entry = WithDimensions::new(
            TestEntry {
                ts: SystemTime::UNIX_EPOCH,
                config: TestConfigEntry,
                f1: Duration::from_millis(42),
                f2: Duration::from_millis(43),
            },
            "foo",
            "bar",
        );

        let entry = metrique_writer::test_util::to_test_entry(&entry);
        assert_eq!(entry.metrics["f1"], 42);
        assert_eq!(
            entry.metrics["f1"].dimensions,
            vec![("foo".to_string(), "bar".to_string())]
        );
        assert_eq!(entry.metrics["f2"], 43);
        assert_eq!(
            entry.metrics["f2"].dimensions,
            vec![("foo".to_string(), "bar".to_string())]
        );
        assert!(entry.timestamp.is_some());
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
