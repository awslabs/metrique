// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{any::Any, borrow::Cow, time::SystemTime};

use smallvec::SmallVec;

use crate::{
    Entry, EntryWriter, Observation, Unit, ValidationError, Value, ValueWriter, value::MetricFlags,
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
}

// Each Dyn* trait is the object-safe equivalent of its partner

trait DynEntry: Any + Send + 'static {
    fn write<'a>(&'a self, writer: &mut dyn DynEntryWriter<'a>);
    fn sample_group(&self) -> SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>;
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
}

impl<E: Entry + Send + 'static> DynEntry for E {
    fn write<'a>(&'a self, writer: &mut dyn DynEntryWriter<'a>) {
        Entry::write(self, &mut EntryWriterFromDyn(writer));
    }

    fn sample_group(&self) -> SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]> {
        Entry::sample_group(self).collect()
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

impl<V: Value + ?Sized> DynValue for ValueToDyn<'_, V> {
    fn write(&self, writer: &mut dyn DynValueWriter) {
        self.0.write(ValueWriterFromDyn(writer));
    }
}

impl Value for ValueFromDyn<'_> {
    fn write(&self, writer: impl ValueWriter) {
        self.0.write(&mut ValueWriterToDyn(Some(writer)));
    }
}

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
}
