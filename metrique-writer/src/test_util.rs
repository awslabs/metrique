// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! [`TestEntry`] provides a way to directly introspect the result of writing out fields with `Entry`
//!
//! This requires that the `test-util` feature be enabled.
//!
//! For usage examples, see [`test_entry_sink`] and `examples/testing.rs`

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use metrique_core::{CloseEntry, InflectableEntry};
use metrique_writer_core::{
    MetricFlags,
    entry::SampleGroupElement,
    value::{FlagConstructor, ForceFlag, MetricOptions},
};
use ordered_float::OrderedFloat;

use crate::{
    AnyEntrySink, BoxEntrySink, Entry, EntryWriter, Observation, Unit, ValueWriter, sink::FlushWait,
};

/// Test flag. This is merely reflected in [TestEntry] to allow seeing that flags are set.
#[derive(Debug)]
pub struct TestFlagOpt;

impl MetricOptions for TestFlagOpt {}

/// Flag constructor for setting a test flag. This is merely reflected
/// in [TestEntry] to allow seeing that flags are set.
pub struct TestFlagCtor;

impl FlagConstructor for TestFlagCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&TestFlagOpt)
    }
}

/// ForceFlag wrapper for [TestFlagOpt]
pub type TestFlag<T> = ForceFlag<T, TestFlagCtor>;

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
    pub values: TestMap<String>,
    /// Metric values in the entry, mapped by field name.
    pub metrics: TestMap<Metric>,
}

/// A wrapper around HashMap that provides better error messages when indexing with missing keys.
#[derive(Debug, Clone, PartialEq)]
pub struct TestMap<T>(HashMap<String, T>);

impl<T> Default for TestMap<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> std::ops::Deref for TestMap<T> {
    type Target = HashMap<String, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::Index<&str> for TestMap<T> {
    type Output = T;

    #[track_caller]
    fn index(&self, key: &str) -> &Self::Output {
        match self.0.get(key) {
            Some(key) => key,
            None => {
                let available_keys: Vec<_> = self.0.keys().map(|k| k.as_str()).collect();
                panic!(
                    "key '{}' not found. Available keys: {:?}",
                    key, available_keys
                )
            }
        }
    }
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
#[non_exhaustive]
pub struct Metric {
    /// The distribution of observations for this metric.
    pub distribution: Vec<Observation>,
    /// The unit of measurement for this metric.
    pub unit: Unit,
    /// The dimensions associated with this metric as key-value pairs.
    pub dimensions: Vec<(String, String)>,
    /// True if the [TestFlag] is set on this metric
    pub test_flag: bool,
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

    /// Returns the total number of observations, correctly accounting for `Repeated`
    pub fn num_observations(&self) -> u64 {
        self.distribution
            .iter()
            .map(|obs| match obs {
                Observation::Unsigned(_) => 1,
                Observation::Floating(_) => 1,
                Observation::Repeated { occurrences, .. } => *occurrences,
                _ => unreachable!(),
            })
            .sum()
    }

    /// Returns all observations in a sorted Vec of f64, flatten repeated obsevations
    pub fn flatten_and_sort(&self) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.num_observations() as usize);
        self.distribution.iter().for_each(|obs| match obs {
            Observation::Unsigned(v) => out.push(*v as f64),
            Observation::Floating(v) => out.push(*v),
            Observation::Repeated { occurrences, total } => {
                for _ in 0..*occurrences {
                    out.push(total / *occurrences as f64)
                }
            }
            _ => unreachable!(),
        });
        out.sort_by_key(|f| OrderedFloat(*f));
        out
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
                self.values.0.insert(name.to_string(), s);
            }
            TestValue::Metric(metric) => {
                self.metrics.0.insert(name.to_string(), metric);
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
        flags: metrique_writer_core::MetricFlags<'_>,
    ) {
        *self.inner = TestValue::Metric(Metric {
            distribution: distribution.into_iter().collect(),
            unit,
            dimensions: dimensions
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            test_flag: flags.downcast::<TestFlagOpt>().is_some(),
        })
    }

    fn error(self, error: metrique_writer_core::ValidationError) {
        panic!("metric returned an error: {error}")
    }
}

/// Converts an [`Entry`] into a `TestEntry` that can be introspected
///
/// > NOTE: This method is probably not what you want. For testing an individual metric,
/// > use [`test_metric`]. For a test-sink that can be installed, use [`test_entry_sink`].
pub fn to_test_entry(e: impl Entry) -> TestEntry {
    let mut entry = TestEntry::empty();
    e.write(&mut entry);
    entry
}

/// Convert a `#[metric]` directly to `TestEntry`
///
/// # Example
///
/// ```
/// use metrique::unit_of_work::metrics;
/// use metrique_writer::test_util::test_metric;
///
/// #[metrics]
/// struct MyMetrics {
///     request_count: u64,
/// }
///
/// let metrics = MyMetrics { request_count: 42 };
/// let entry = test_metric(metrics);
/// assert_eq!(entry.metrics["request_count"], 42);
/// ```
pub fn test_metric(e: impl CloseEntry) -> TestEntry {
    let root_entry = RootEntry::new(e.close());
    to_test_entry(root_entry)
}

struct RootEntry<M: InflectableEntry> {
    metric: M,
}

impl<M: InflectableEntry> RootEntry<M> {
    /// create a new [`RootEntry`]
    pub fn new(metric: M) -> Self {
        Self { metric }
    }
}

impl<M: InflectableEntry> Entry for RootEntry<M> {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        self.metric.write(w);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.metric.sample_group()
    }
}

/// A test sink for capturing and inspecting metric entries.
///
/// This struct provides both a sink that can be used in place of a real sink
/// and an inspector that can be used to examine the entries that were appended
/// to the sink.
///
/// This requires that the `test-util` feature be enabled.
#[derive(Clone, Debug)]
pub struct TestEntrySink {
    /// The inspector for examining captured metric entries.
    pub inspector: Inspector,
    /// The sink to which metric entries can be appended.
    pub sink: BoxEntrySink,
}

/// Create a [`TestEntrySink`] and a connected [`BoxEntrySink`] that can be used in your application
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
/// See [`test_entry_sink`] for usage examples.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entry, EntrySink};

    #[derive(Entry)]
    struct TestMetrics {
        operation: &'static str,
        request_count: u64,
    }

    #[test]
    #[should_panic(expected = "key 'wrong_name' not found. Available keys: [\"request_count\"]")]
    fn test_metric_map_missing_key_error() {
        let sink = test_entry_sink();
        sink.sink.append(TestMetrics {
            operation: "test",
            request_count: 42,
        });

        let entries = sink.inspector.entries();
        let _ = &entries[0].metrics["wrong_name"];
    }

    #[test]
    #[should_panic(expected = "key 'wrong_name' not found. Available keys: [\"operation\"]")]
    fn test_value_map_missing_key_error() {
        let sink = test_entry_sink();
        sink.sink.append(TestMetrics {
            operation: "test",
            request_count: 42,
        });

        let entries = sink.inspector.entries();
        let _ = &entries[0].values["wrong_name"];
    }
}
