// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Cow,
    io, mem,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use crate::{
    Entry, EntryConfig, EntryIoStream, EntryWriter, IoStreamError, MetricFlags, Observation, Unit,
    ValidationError, Value, ValueWriter, format::Format,
};

pub struct FuelGuard {
    fuel: Option<Arc<AtomicU64>>,
}

impl Drop for FuelGuard {
    fn drop(&mut self) {
        if let Some(fuel) = &self.fuel {
            // add fuel for safe shutdown
            fuel.fetch_add(1_000_000_000_000, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
pub struct TestStream {
    pub values: Vec<u64>,
    pub found_errors: u64,
    pub error: Option<IoStreamError>,
    pub flushes: u64,
    pub values_flushed: usize,
    pub fuel: Option<Arc<AtomicU64>>,
}

impl TestStream {
    pub fn set_up_fuel(&mut self, initial_fuel: u64) -> FuelGuard {
        self.fuel = Some(Arc::new(AtomicU64::new(initial_fuel)));
        FuelGuard {
            fuel: self.fuel.clone(),
        }
    }
}

impl EntryIoStream for Arc<Mutex<TestStream>> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        let fuel = self.lock().unwrap().fuel.clone();
        if let Some(fuel) = fuel {
            while let Ok(0) = fuel.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| {
                Some(x.saturating_sub(1))
            }) {}
        }
        entry.write(self);
        match self.lock().unwrap().error.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut this = self.lock().unwrap();
        this.flushes += 1;
        this.values_flushed = this.values.len();
        Ok(())
    }
}

impl<'a> EntryWriter<'a> for Arc<Mutex<TestStream>> {
    fn timestamp(&mut self, _timestamp: SystemTime) {
        unreachable!()
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        match &name[..] {
            "value" => value.write(&mut *self.lock().unwrap()),
            "Error" => self.lock().unwrap().found_errors += 1,
            _ => panic!("unexpected name {name}"),
        }
    }

    #[inline]
    fn config(&mut self, _config: &'a dyn crate::entry::EntryConfig) {}
}

impl ValueWriter for &'_ mut TestStream {
    fn string(self, _value: &str) {
        unreachable!()
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
        let Some(Observation::Unsigned(value)) = distribution.into_iter().next() else {
            unreachable!();
        };
        self.values.push(value);
    }

    fn error(self, _error: ValidationError) {
        unreachable!()
    }
}

pub struct TestEntry(pub u64);

impl Entry for TestEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("value", &self.0);
    }
}

/// A helper struct that implements `io::Write` to write to an `Arc<Mutex<Vec<u8>>>`, to be
/// used in tests.
#[derive(Clone, Debug, Default)]
pub struct TestSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl TestSink {
    /// Return the content of the inner `Vec<u8>` as a String, panicking if it's not valid utf-8
    pub fn dump(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// Return the content of the inner `Vec<u8>` as a String and take it, panicking if it's not valid utf-8
    pub fn take_string(&self) -> String {
        String::from_utf8(mem::take(&mut self.0.lock().unwrap())).unwrap()
    }
}

impl io::Write for TestSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write_vectored(bufs)
    }
}

pub struct DummyFormat;
impl Format for DummyFormat {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        let mut writer = DummyEntryWriter::default();
        entry.write(&mut writer);
        output
            .write(format!("{:?}", writer.0).as_bytes())
            .map_err(IoStreamError::Io)?;
        Ok(())
    }
}

#[derive(Default)]
pub struct DummyEntryWriter(pub Vec<(String, String)>);

impl<'a> EntryWriter<'a> for DummyEntryWriter {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.0.push((
            "timestamp".to_string(),
            format!(
                "{}",
                timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64()
            ),
        ));
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        value.write(DummyValueWriter(self, name.into()));
    }

    fn config(&mut self, _config: &dyn EntryConfig) {}
}
pub struct DummyValueWriter<'a>(&'a mut DummyEntryWriter, Cow<'a, str>);
impl ValueWriter for DummyValueWriter<'_> {
    fn string(self, value: &str) {
        self.0.0.push((self.1.to_string(), value.to_string()));
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        self.0.0.push((
            self.1.to_string(),
            format!(
                "{:?} {:?} {:?}",
                distribution.into_iter().collect::<Vec<_>>(),
                unit,
                dimensions.into_iter().collect::<Vec<_>>()
            ),
        ));
    }

    fn error(self, error: ValidationError) {
        panic!("{error}");
    }
}

#[cfg(test)]
mod test {
    use crate::{
        Entry, EntryIoStream, IoStreamError, config::BasicErrorMessage, format::Format,
        test_stream::DummyFormat,
    };

    struct BasicEntryIoStream(Vec<u8>);
    impl EntryIoStream for BasicEntryIoStream {
        fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
            DummyFormat.format(entry, &mut self.0)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_format_error() {
        let mut stream = BasicEntryIoStream(vec![]);
        stream.next(&BasicErrorMessage::new("basic-error")).unwrap();
        assert_eq!(
            String::from_utf8(stream.0).unwrap(),
            "[(\"Error\", \"basic-error\")]"
        );
    }
}
