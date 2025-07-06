// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! [`TestEntry`] provides a way to directly introspect the result of writing out fields with `Entry`
//!
//! This requires that the `test-util` feature be enabled.
//!
//! For usage examples, see [`test_sink`] and `examples/testing.rs`

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use crate::{
    AnyEntrySink, BoxEntrySink, Entry, EntryWriter, Observation, Unit, ValueWriter, sink::FlushWait,
};

/// A test representation of a metric entry.
///
/// This struct provides a way to inspect metric entries for testing purposes.
/// It captures the timestamp, string values, and metric values from an entry.
///
/// This requires that the `test-util` feature be enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct TestEntry {
    /// The timestamp of the entry, if one was provided.
    pub timestamp: Option<SystemTime>,
    /// String values in the entry, mapped by field name.
    pub values: HashMap<String, String>,
    /// Metric values in the entry, mapped by field name.
    pub metrics: HashMap<String, Metric>,
}

impl<T: Entry> From<T> for TestEntry {
    fn from(value: T) -> Self {
        to_test_entry(value)
    }
}

impl TestEntry {
    // does not implement default since publicly, default does not do anything useful
    fn empty() -> Self {
        Self {
            timestamp: None,
            values: Default::default(),
            metrics: Default::default(),
        }
    }
}

/// A representation of a metric value for testing.
///
/// This struct captures the distribution, unit, and dimensions of a metric
/// to allow for inspection in tests.
///
/// This requires that the `test-util` feature be enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    /// The distribution of observations for this metric.
    pub distribution: Vec<Observation>,
    /// The unit of measurement for this metric.
    pub unit: Unit,
    /// The dimensions associated with this metric as key-value pairs.
    pub dimensions: Vec<(String, String)>,
}

impl Metric {
    /// Returns the value in this observation as a u64
    ///
    /// If the value was originally provided as an f64, it will be cast into a u64
    ///
    /// # Panics
    /// If this observation is repeated (e.g. a histogram), this function will panic
    #[track_caller]
    pub fn as_u64(&self) -> u64 {
        assert_eq!(self.distribution.len(), 1);
        match &self.distribution[0] {
            Observation::Unsigned(v) => *v,
            Observation::Floating(f) => *f as u64,
            Observation::Repeated { .. } => {
                panic!("found a repeated sample, expected one value")
            }
            _ => unreachable!(),
        }
    }

    /// Returns the value in this observation as a bool
    ///
    /// All values > 0 are considered true
    #[track_caller]
    pub fn as_bool(&self) -> bool {
        self.as_u64() > 0
    }

    /// Returns the value in this observation as an f64
    ///
    /// If the value was originally provided as an u64, it will be cast into a f64
    ///
    /// # Panics
    /// If this observation is repeated (e.g. a histogram), this function will panic
    #[track_caller]
    pub fn as_f64(&self) -> f64 {
        assert_eq!(self.distribution.len(), 1);
        match &self.distribution[0] {
            Observation::Unsigned(v) => *v as f64,
            Observation::Floating(f) => *f,
            Observation::Repeated { .. } => {
                panic!("found a repeated sample, expected one value")
            }
            _ => unreachable!(),
        }
    }
}

impl PartialEq<bool> for Metric {
    #[track_caller]
    fn eq(&self, other: &bool) -> bool {
        self.as_bool() == *other
    }
}

impl PartialEq<u64> for Metric {
    #[track_caller]
    fn eq(&self, other: &u64) -> bool {
        self.as_u64() == *other
    }
}

impl PartialEq<f64> for Metric {
    #[track_caller]
    fn eq(&self, other: &f64) -> bool {
        self.as_f64() == *other
    }
}

impl PartialOrd<u64> for Metric {
    #[track_caller]
    fn partial_cmp(&self, other: &u64) -> Option<std::cmp::Ordering> {
        self.as_u64().partial_cmp(other)
    }
}

impl PartialOrd<f64> for Metric {
    #[track_caller]
    fn partial_cmp(&self, other: &f64) -> Option<std::cmp::Ordering> {
        self.as_f64().partial_cmp(other)
    }
}

impl<'a> EntryWriter<'a> for TestEntry {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.timestamp = Some(timestamp);
    }

    fn value(
        &mut self,
        name: impl Into<std::borrow::Cow<'a, str>>,
        value: &(impl crate::Value + ?Sized),
    ) {
        let name = name.into();
        let mut raw_value = TestValue::Unset;
        let writer = TestValueWriter {
            inner: &mut raw_value,
        };
        value.write(writer);
        match raw_value {
            TestValue::Property(s) => {
                self.values.insert(name.to_string(), s);
            }
            TestValue::Metric(metric) => {
                self.metrics.insert(name.to_string(), metric);
            }
            TestValue::Unset => {
                // This case happens if, e.g. the value is `Option<T>` and it is None
            }
        };
    }

    fn config(&mut self, _config: &'a dyn metrique_writer_core::EntryConfig) {
        // this EntryWriter does not support any user-defined config
    }
}

struct TestValueWriter<'a> {
    inner: &'a mut TestValue,
}

#[derive(Default)]
enum TestValue {
    Property(String),
    Metric(Metric),
    #[default]
    Unset,
}

impl ValueWriter for TestValueWriter<'_> {
    fn string(self, value: &str) {
        *self.inner = TestValue::Property(value.to_string())
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: metrique_writer_core::MetricFlags<'_>,
    ) {
        *self.inner = TestValue::Metric(Metric {
            distribution: distribution.into_iter().collect(),
            unit,
            dimensions: dimensions
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        })
    }

    fn error(self, error: metrique_writer_core::ValidationError) {
        panic!("metric returned an error: {error}")
    }
}

/// Converts an [`Entry`] into a `TestEntry` that can be introspected
///
/// > NOTE: This method is probably not what you want. Use [`test_sink`] instead.
pub fn to_test_entry(e: impl Entry) -> TestEntry {
    let mut entry = TestEntry::empty();
    e.write(&mut entry);
    entry
}

/// A test sink for capturing and inspecting metric entries.
///
/// This struct provides both a sink that can be used in place of a real sink
/// and an inspector that can be used to examine the entries that were appended
/// to the sink.
///
/// This requires that the `test-util` feature be enabled.
pub struct TestEntrySink {
    /// The inspector for examining captured metric entries.
    pub inspector: Inspector,
    /// The sink to which metric entries can be appended.
    pub sink: BoxEntrySink,
}

/// Create a [`TestSink`] and a connected [`BoxEntrysink`] that can be used in your application
///
/// This requires that the `test-util` feature be enabled.
/// # Examples
/// ```
/// use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
/// use metrique_writer::{Entry, EntrySink};
///
/// #[derive(Entry)]
/// struct RequestMetrics {
///     operation: &'static str,
///     number_of_ducks: usize
/// }
///
/// #[test]
/// # fn test_in_doctests_ignored() {}
/// fn test_metrics () {
///     let TestEntrySink { inspector, sink } = test_entry_sink();
///     sink.append(RequestMetrics {
///         operation: "SayHello",
///         number_of_ducks: 10
///     });
///     // In a real application, you would run some API calls, etc.
///
///     let entries = inspector.entries();
///     assert_eq!(entries[0].values["Operation"], "SayHello");
///     assert_eq!(entries[0].metrics["NumberOfDucks"].as_u64(), 10);
/// }
/// ```
pub fn test_entry_sink() -> TestEntrySink {
    let sink = Inspector::default();
    TestEntrySink {
        inspector: sink.clone(),
        sink: BoxEntrySink::new(sink),
    }
}

/// `Inspector` can be used as a sink while making it easy to read the metrics that have been emitted
///
/// See [`test_sink`] for usage examples.
#[derive(Default, Clone, Debug)]
pub struct Inspector {
    entries: Arc<Mutex<Vec<TestEntry>>>,
}

impl Inspector {
    /// Return all the entries inside the test sink
    ///
    /// Note: this does not drain or otherwise modify the contained entries
    pub fn entries(&self) -> Vec<TestEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Returns an entry at a specific index
    pub fn get(&self, index: usize) -> TestEntry {
        self.entries()[index].clone()
    }
}

impl AnyEntrySink for Inspector {
    fn append_any(&self, entry: impl Entry + Send + 'static) {
        self.entries.lock().unwrap().push(to_test_entry(entry));
    }

    fn flush_async(&self) -> FlushWait {
        FlushWait::ready()
    }
}
