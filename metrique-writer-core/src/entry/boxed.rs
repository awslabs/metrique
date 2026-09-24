// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{any::Any, borrow::Cow, time::SystemTime};

use smallvec::SmallVec;

use crate::{
    Descriptors, Entry, EntryWriter, Observation, Unit, ValidationError, Value, ValueWriter,
    value::{MetricFlags, VALUES_INLINE_CAPACITY},
};

use super::EntryConfig;

/// A heap-allocated [`Entry`] wrapper that uses dynamic dispatch.
///
/// While somewhat slower than a statically-dispatched entries, an [`crate::EntrySink`] of boxed
/// entries can be heterogeneous rather than requiring all entries to be the same type. This is
/// especially useful for "global" background queues that will consume entries from many
/// different components.
pub struct BoxEntry(Box<dyn DynEntry>);

impl BoxEntry {
    /// Move the entry to the heap and enable dynamic dispatch.
    pub fn new(entry: impl Entry + Send + 'static) -> Self {
        Self(Box::new(entry))
    }

    /// Returns a reference to the inner [`Entry`] value, which can be used with
    /// [`Any`] to extract a typed reference.
    pub fn inner(&self) -> &(dyn Any + Send + 'static) {
        &self.0
    }

    /// Returns a mutable reference to the inner [`Entry`] value, which can be used
    /// with [`Any`] to extract a typed reference.
    pub fn inner_mut(&mut self) -> &mut (dyn Any + Send + 'static) {
        &mut self.0
    }
}

// Behind the scenes, we use a double dispatch method to make each layer of traits (Entry <=>
// EntryWriter, Value <=> ValueWriter) object safe.
impl Entry for BoxEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        self.0.write(&mut EntryWriterToDyn(writer))
    }

    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        self.0.sample_group().into_iter()
    }

    fn descriptors(&self) -> Descriptors<'_> {
        self.0.descriptors()
    }
}

// Each Dyn* trait is the object-safe equivalent of its partner

trait DynEntry: Any + Send + 'static {
    fn write<'a>(&'a self, writer: &mut dyn DynEntryWriter<'a>);
    fn sample_group(&self) -> SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>;
    fn descriptors(&self) -> Descriptors<'_>;
}

trait DynEntryWriter<'a> {
    fn timestamp(&mut self, timestamp: SystemTime);
    fn value(&mut self, name: Cow<'a, str>, value: &dyn DynValue);
    fn config(&mut self, config: &'a dyn EntryConfig);
}

trait DynValue {
    fn write(&self, writer: &mut dyn DynValueWriter);
}

trait DynValueWriter {
    fn string(&mut self, value: &str);

    fn metric<'a>(
        &mut self,
        distribution: &[Observation],
        unit: Unit,
        dimensions: &[(&'a str, &'a str)],
        flags: MetricFlags<'_>,
    );

    fn error(&mut self, error: ValidationError);

    /// Forward a list of values across the dyn boundary without collapsing each element to
    /// text. Each element is re-wrapped as a [`DynValue`], so it keeps its
    /// [`string()`](DynValueWriter::string)/[`metric()`](DynValueWriter::metric) identity and
    /// formats with native array support (e.g. EMF) emit the correct element type.
    fn values_dyn(&mut self, values: &mut dyn Iterator<Item = &dyn DynValue>);
}

impl<E: Entry + Send + 'static> DynEntry for E {
    fn write<'a>(&'a self, writer: &mut dyn DynEntryWriter<'a>) {
        Entry::write(self, &mut EntryWriterFromDyn(writer));
    }

    fn sample_group(&self) -> SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]> {
        Entry::sample_group(self).collect()
    }

    fn descriptors(&self) -> Descriptors<'_> {
        Entry::descriptors(self)
    }
}

struct EntryWriterToDyn<W>(W);
struct EntryWriterFromDyn<'a, 'w>(&'w mut dyn DynEntryWriter<'a>);

impl<'a, W: EntryWriter<'a>> DynEntryWriter<'a> for EntryWriterToDyn<W> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.0.timestamp(timestamp)
    }

    fn value(&mut self, name: Cow<'a, str>, value: &dyn DynValue) {
        self.0.value(name, &ValueFromDyn(value));
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.0.config(config);
    }
}

impl<'a> EntryWriter<'a> for EntryWriterFromDyn<'a, '_> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.0.timestamp(timestamp)
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        self.0.value(name.into(), &ValueToDyn(value))
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.0.config(config)
    }
}

struct ValueToDyn<'a, V: ?Sized>(&'a V);
struct ValueFromDyn<'a>(&'a dyn DynValue);

// Blanket bridge: every `Value` is usable as an object-safe `DynValue`. This lets
// `ValueWriterFromDyn::values` coerce `&V` straight to `&dyn DynValue` with no intermediate
// buffer. `DynValue` is a private trait, so this doesn't widen the public API.
impl<V: Value + ?Sized> DynValue for V {
    fn write(&self, writer: &mut dyn DynValueWriter) {
        Value::write(self, ValueWriterFromDyn(writer));
    }
}

// Adapts a `?Sized` value (e.g. `str`) into a `Sized` type that can be coerced to
// `&dyn DynValue` for the scalar-field bridge, where a `?Sized` value can't be unsize-coerced
// to a trait object directly.
impl<V: Value + ?Sized> Value for ValueToDyn<'_, V> {
    const SHAPE: crate::descriptor::FieldShape<'static> = V::SHAPE;
    const UNIT: crate::Unit = V::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        <V as Value>::write(self.0, writer)
    }
}

impl Value for ValueFromDyn<'_> {
    const SHAPE: crate::descriptor::FieldShape<'static> = crate::descriptor::FieldShape::Opaque;
    const UNIT: crate::Unit = crate::Unit::None;

    fn write(&self, writer: impl ValueWriter) {
        DynValue::write(self.0, &mut ValueWriterToDyn(Some(writer)));
    }
}

// Holds the real `ValueWriter` in an `Option` because `ValueWriter`'s methods consume `self`
// by value, but the object-safe `DynValueWriter` takes `&mut self`. Exactly one method is
// invoked per writer (the `Value`/`DynValue` write contract), so the `take().unwrap()` in each
// method cannot fail; a second call would panic.
struct ValueWriterToDyn<W>(Option<W>);
struct ValueWriterFromDyn<'a>(&'a mut dyn DynValueWriter);

impl<W: ValueWriter> DynValueWriter for ValueWriterToDyn<W> {
    fn string(&mut self, value: &str) {
        self.0.take().unwrap().string(value)
    }

    fn metric<'a>(
        &mut self,
        distribution: &[Observation],
        unit: Unit,
        dimensions: &[(&'a str, &'a str)],
        flags: MetricFlags<'_>,
    ) {
        self.0.take().unwrap().metric(
            distribution.iter().copied(),
            unit,
            dimensions.iter().copied(),
            flags,
        )
    }

    fn error(&mut self, error: ValidationError) {
        self.0.take().unwrap().error(error)
    }

    fn values_dyn(&mut self, values: &mut dyn Iterator<Item = &dyn DynValue>) {
        // Re-wrap each `&dyn DynValue` as a `Value` so the inner writer sees the elements
        // intact. `ValueWriter::values` takes references, so the wrappers must be materialized
        // here (one buffer, inline up to VALUES_INLINE_CAPACITY, heap only on spill).
        let wrapped: SmallVec<[ValueFromDyn<'_>; VALUES_INLINE_CAPACITY]> =
            values.map(ValueFromDyn).collect();
        self.0.take().unwrap().values(wrapped.iter())
    }
}

impl ValueWriter for ValueWriterFromDyn<'_> {
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
            distribution
                .into_iter()
                .collect::<SmallVec<[_; 2]>>()
                .as_slice(),
            unit,
            dimensions
                .into_iter()
                .collect::<SmallVec<[_; 1]>>()
                .as_slice(),
            flags,
        )
    }

    fn error(self, error: ValidationError) {
        self.0.error(error)
    }

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        // Every `Value` is a `DynValue` (blanket impl), so each element coerces straight to
        // `&dyn DynValue` and is forwarded across the boundary intact — no buffer, no
        // stringification. The receiver materializes the elements it needs.
        let mut iter = values.into_iter().map(|v| v as &dyn DynValue);
        self.0.values_dyn(&mut iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EntryWriter, MetricValue as _, test_stream::DummyEntryWriter, value::WithDimensions,
    };
    use std::time::{Duration, SystemTime};

    #[test]
    fn dummy() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1.5));
                writer.value("Time", &Duration::from_millis(42));
                writer.value("StringProp", "some string value");
                writer.value("BasicIntCount", &1234u64);
                writer.value(
                    "BasicIntCountWithDimensions",
                    &(1234u64.with_dimensions([("A", "x"), ("B", "y")]) as WithDimensions<_, 2>),
                );
                writer.value("BasicFloatCount", &5.4321f64);
                writer.value("SomeDuration", &Duration::from_micros(12345678));
            }
        }

        let mut writer = DummyEntryWriter::default();
        <BoxEntry as Entry>::write(&TestEntry.boxed(), &mut writer);
        assert_eq!(
            writer.0,
            vec![
                ("timestamp".to_string(), "1.5".to_string()),
                (
                    "Time".to_string(),
                    "[Floating(42.0)] Milliseconds []".to_string()
                ),
                ("StringProp".to_string(), "some string value".to_string()),
                (
                    "BasicIntCount".to_string(),
                    "[Unsigned(1234)] None []".to_string()
                ),
                (
                    "BasicIntCountWithDimensions".to_string(),
                    "[Unsigned(1234)] None [(\"A\", \"x\"), (\"B\", \"y\")]".to_string()
                ),
                (
                    "BasicFloatCount".to_string(),
                    "[Floating(5.4321)] None []".to_string()
                ),
                (
                    "SomeDuration".to_string(),
                    "[Floating(12345.678)] Milliseconds []".to_string()
                ),
            ]
        );
    }

    /// An [`EntryWriter`] that records each list element with its native shape (metric vs
    /// string), so a test can tell whether the dyn bridge preserved element types.
    #[derive(Default)]
    struct ListRecorder(Vec<(String, Vec<String>)>);

    impl<'a> EntryWriter<'a> for ListRecorder {
        fn timestamp(&mut self, _timestamp: SystemTime) {}

        fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
            let mut elements = Vec::new();
            value.write(ListValueWriter(&mut elements));
            self.0.push((name.into().into_owned(), elements));
        }

        fn config(&mut self, _config: &'a dyn EntryConfig) {}
    }

    struct ListValueWriter<'a>(&'a mut Vec<String>);

    impl ValueWriter for ListValueWriter<'_> {
        fn string(self, value: &str) {
            self.0.push(format!("string:{value}"));
        }

        fn metric<'a>(
            self,
            distribution: impl IntoIterator<Item = Observation>,
            unit: Unit,
            dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
            _flags: MetricFlags<'_>,
        ) {
            // Record unit and dimensions too: a regression that dropped per-element
            // unit/dimensions on the way across the bridge must fail this test, not pass it.
            self.0.push(format!(
                "metric:{:?} unit={unit:?} dims={:?}",
                distribution.into_iter().collect::<Vec<_>>(),
                dimensions.into_iter().collect::<Vec<_>>(),
            ));
        }

        fn error(self, error: ValidationError) {
            panic!("{error}");
        }

        fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
            for value in values {
                value.write(ListValueWriter(self.0));
            }
        }
    }

    // Regression test for #349: list elements must cross the dyn (boxing) bridge without being
    // stringified. A `Vec<u64>` must reach the sink as metric observations, not as strings.
    #[test]
    fn boxed_list_elements_retain_shape() {
        struct ListEntry;
        impl Entry for ListEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Ints", &vec![1u64, 2, 3]);
                writer.value("Strs", &vec!["a".to_string(), "b".to_string()]);
                // Elements carrying a unit and per-element dimensions, to prove those
                // survive the bridge rather than being flattened away.
                writer.value("Times", &vec![Duration::from_millis(5)]);
                writer.value(
                    "Dimmed",
                    &vec![7u64.with_dimensions([("Region", "us")]) as WithDimensions<_, 1>],
                );
            }
        }

        // Unboxed and boxed must produce identical element-level output.
        let mut unboxed = ListRecorder::default();
        Entry::write(&ListEntry, &mut unboxed);

        let mut boxed = ListRecorder::default();
        <BoxEntry as Entry>::write(&ListEntry.boxed(), &mut boxed);

        let expected = vec![
            (
                "Ints".to_string(),
                vec![
                    "metric:[Unsigned(1)] unit=None dims=[]".to_string(),
                    "metric:[Unsigned(2)] unit=None dims=[]".to_string(),
                    "metric:[Unsigned(3)] unit=None dims=[]".to_string(),
                ],
            ),
            (
                "Strs".to_string(),
                vec!["string:a".to_string(), "string:b".to_string()],
            ),
            (
                "Times".to_string(),
                vec!["metric:[Floating(5.0)] unit=Milliseconds dims=[]".to_string()],
            ),
            (
                "Dimmed".to_string(),
                vec!["metric:[Unsigned(7)] unit=None dims=[(\"Region\", \"us\")]".to_string()],
            ),
        ];
        assert_eq!(unboxed.0, expected);
        assert_eq!(boxed.0, expected);
    }
}
