// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    io,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use derive_where::derive_where;
use smallvec::SmallVec;

use crate::{
    Entry, EntryIoStream, EntryWriter, IoStreamError, Observation, Unit, ValidationError,
    ValueWriter,
};

use super::{MetricFlags, MetricValue, Value};

/// A trait for functions that return a [`MetricFlags<'static>`][MetricFlags]
///
/// <div id="doc-warning-1" class="warning">
/// The API for defining new flags is currently not covered by semver,
/// and might break in new versions of this library.
/// </div>
///
/// If you want to implement your own metric flag, and you want to
/// be able to use it with [`ForceFlag`], you can create a [`FlagConstructor`]
/// for your flag:
///
/// ```
/// # use metrique_writer::MetricFlags;
/// # use metrique_writer::value::{FlagConstructor, ForceFlag};
///
/// #[derive(Debug)]
/// pub struct MyFlagOpt;
///
/// pub struct MyFlagCtor;
///
/// impl FlagConstructor for MyFlagCtor {
///     fn construct() -> MetricFlags<'static> {
///         MetricFlags::upcast(&MyFlagOpt)
///     }
/// }
///
/// impl metrique_writer::value::MetricOptions for MyFlagOpt {}
///
/// pub type MyFlag<T> = ForceFlag<T, MyFlagCtor>;
/// ```
pub trait FlagConstructor {
    /// Return the desired flag
    fn construct() -> MetricFlags<'static>;
}

/// Helper to force enable metric flags on a value.
///
/// This is intentionally "punned" to work with [Entry], [Value], and [EntryIoStream] to
/// avoid duplication of the format-specific flag types like `HighStorageResolution`.
///
/// Prefer using `#[metrics(flags(...))]` instead of wrapping field types directly.
/// The macro attribute both applies the flag at write time and records it in the
/// entry descriptor, giving formats and sinks full visibility:
///
/// ```ignore
/// use my_format::flags::HighStorageResolution;
///
/// // Instead of:
/// #[metrics]
/// struct MyMetrics {
///     event_count: HighStorageResolution<Counter>,
/// }
///
/// // Prefer:
/// #[metrics]
/// struct MyMetrics {
///     #[metrics(flags(HighStorageResolution))]
///     event_count: Counter,
/// }
/// ```
#[derive_where(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash; T)]
pub struct ForceFlag<T, FLAGS: FlagConstructor>(T, PhantomData<FLAGS>);

impl<V, FLAGS: FlagConstructor> ForceFlag<V, FLAGS> {
    /// Map the value within this [ForceFlag]
    pub fn map_value<U>(self, f: impl Fn(V) -> U) -> ForceFlag<U, FLAGS> {
        ForceFlag(f(self.0), PhantomData)
    }

    /// Map the value within this [ForceFlag] by reference
    pub fn map_value_ref<U>(&self, f: impl Fn(&V) -> U) -> ForceFlag<U, FLAGS> {
        ForceFlag(f(&self.0), PhantomData)
    }
}

impl<T, FLAGS: FlagConstructor> From<T> for ForceFlag<T, FLAGS> {
    fn from(value: T) -> Self {
        Self(value, PhantomData)
    }
}

impl<T, FLAGS: FlagConstructor> Deref for ForceFlag<T, FLAGS> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, FLAGS: FlagConstructor> DerefMut for ForceFlag<T, FLAGS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, FLAGS: FlagConstructor> ForceFlag<T, FLAGS> {
    /// Return the value contained within this [ForceFlag]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Value, FLAGS: FlagConstructor> Value for ForceFlag<T, FLAGS> {
    const SHAPE: crate::descriptor::FieldShape<'static> = T::SHAPE;
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        struct Wrapper<W, FLAGS: FlagConstructor>(W, PhantomData<FLAGS>);

        impl<W: ValueWriter, FLAGS: FlagConstructor> ValueWriter for Wrapper<W, FLAGS> {
            fn string(self, value: &str) {
                self.0.string(value)
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                self.0.metric(
                    distribution,
                    unit,
                    dimensions,
                    flags.try_merge(FLAGS::construct()),
                );
            }

            fn error(self, error: ValidationError) {
                self.0.error(error)
            }

            fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
                // Wrap each element so `metric()` calls still merge the flag.
                let wrapped: SmallVec<[ForceFlag<&'a V, FLAGS>; 8]> =
                    values.into_iter().map(ForceFlag::from).collect();
                self.0.values(wrapped.iter())
            }
        }

        self.0.write(Wrapper::<_, FLAGS>(writer, PhantomData))
    }
}

impl<T: MetricValue, FLAGS: FlagConstructor> MetricValue for ForceFlag<T, FLAGS> {
    type Unit = T::Unit;
}

/// Writer wrapper that injects a flag into all value writes.
/// Used internally by flatten-site `default_flags` codegen.
#[doc(hidden)]
pub struct ForceFlagEntryWriter<'a, W, FLAGS: FlagConstructor> {
    writer: &'a mut W,
    phantom: PhantomData<FLAGS>,
}

impl<'a, W, FLAGS: FlagConstructor> ForceFlagEntryWriter<'a, W, FLAGS> {
    /// Create a new `ForceFlagEntryWriter` wrapping the given writer.
    #[doc(hidden)]
    pub fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            phantom: PhantomData,
        }
    }
}

impl<'a, W: EntryWriter<'a>, FLAGS: FlagConstructor> EntryWriter<'a>
    for ForceFlagEntryWriter<'_, W, FLAGS>
{
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.writer.timestamp(timestamp)
    }

    fn value(
        &mut self,
        name: impl Into<std::borrow::Cow<'a, str>>,
        value: &(impl crate::Value + ?Sized),
    ) {
        self.writer.value(name, &ForceFlag::<_, FLAGS>::from(value))
    }

    fn config(&mut self, config: &'a dyn crate::EntryConfig) {
        self.writer.config(config);
    }
}

impl<E: Entry, FLAGS: FlagConstructor> Entry for ForceFlag<E, FLAGS> {
    fn write<'a>(&'a self, writer: &mut impl crate::EntryWriter<'a>) {
        self.0.write(&mut ForceFlagEntryWriter {
            writer,
            phantom: self.1,
        })
    }

    fn sample_group(&self) -> impl Iterator<Item = crate::entry::SampleGroupElement> {
        self.0.sample_group()
    }

    fn descriptors(&self) -> crate::Descriptors<'_> {
        self.0.descriptors()
    }
}

impl<S: EntryIoStream, FLAGS: FlagConstructor> EntryIoStream for ForceFlag<S, FLAGS> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.0.next(&ForceFlag(entry, self.1))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::MetricOptions;

    #[derive(Debug)]
    struct TestFlagOpt;
    impl MetricOptions for TestFlagOpt {}

    struct TestFlagCtor;
    impl FlagConstructor for TestFlagCtor {
        fn construct() -> MetricFlags<'static> {
            MetricFlags::upcast(&TestFlagOpt)
        }
    }

    #[derive(Debug, PartialEq)]
    enum Event {
        String(String),
        ValuesStart,
        Metric {
            total: u64,
            flagged: bool,
            dimensions: Vec<(String, String)>,
        },
    }

    struct Recorder<'a>(&'a mut Vec<Event>);

    impl ValueWriter for Recorder<'_> {
        fn string(self, value: &str) {
            self.0.push(Event::String(value.to_string()));
        }

        fn metric<'a>(
            self,
            distribution: impl IntoIterator<Item = Observation>,
            _unit: Unit,
            dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
            flags: MetricFlags<'_>,
        ) {
            let total = distribution
                .into_iter()
                .map(|obs| match obs {
                    Observation::Unsigned(v) => v,
                    other => panic!("unexpected observation {other:?}"),
                })
                .sum();
            self.0.push(Event::Metric {
                total,
                flagged: flags.downcast::<TestFlagOpt>().is_some(),
                dimensions: dimensions
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            });
        }

        fn error(self, error: ValidationError) {
            panic!("unexpected error: {error:?}");
        }

        // Distinguishes a forwarded `values()` call from the default
        // comma-joined `string()` fallback.
        fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
            self.0.push(Event::ValuesStart);
            for value in values {
                value.write(Recorder(self.0));
            }
        }
    }

    #[test]
    fn forwards_values_to_wrapped_writer() {
        let value: ForceFlag<Vec<String>, TestFlagCtor> =
            vec!["a".to_string(), "b".to_string()].into();
        let mut events = Vec::new();
        value.write(Recorder(&mut events));
        assert_eq!(
            events,
            [
                Event::ValuesStart,
                Event::String("a".into()),
                Event::String("b".into()),
            ],
        );
    }

    #[test]
    fn values_elements_carry_flag() {
        let value: ForceFlag<Vec<u64>, TestFlagCtor> = vec![1, 2].into();
        let mut events = Vec::new();
        value.write(Recorder(&mut events));
        assert_eq!(
            events,
            [
                Event::ValuesStart,
                Event::Metric {
                    total: 1,
                    flagged: true,
                    dimensions: vec![]
                },
                Event::Metric {
                    total: 2,
                    flagged: true,
                    dimensions: vec![]
                },
            ],
        );
    }

    #[test]
    fn forwards_empty_values() {
        let value: ForceFlag<Vec<u64>, TestFlagCtor> = vec![].into();
        let mut events = Vec::new();
        value.write(Recorder(&mut events));
        assert_eq!(events, [Event::ValuesStart]);
    }

    #[test]
    fn stacked_wrappers_apply_flag_and_dimensions_to_elements() {
        use crate::value::WithDimension;

        let value = WithDimension::new(
            ForceFlag::<_, TestFlagCtor>::from(vec![1u64, 2u64]),
            "foo",
            "bar",
        );
        let mut events = Vec::new();
        value.write(Recorder(&mut events));
        let dimensions = vec![("foo".to_string(), "bar".to_string())];
        assert_eq!(
            events,
            [
                Event::ValuesStart,
                Event::Metric {
                    total: 1,
                    flagged: true,
                    dimensions: dimensions.clone()
                },
                Event::Metric {
                    total: 2,
                    flagged: true,
                    dimensions
                },
            ],
        );
    }
}
