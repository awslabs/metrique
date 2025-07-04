// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use super::unit::metrics_unit_to_metrique_unit;
use crate::metrics::metrics_histogram::Bucket;
use metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use metrics_util::registry::{Registry, Storage};
use metrique_writer_core::{Entry, EntryWriter, Observation, value::MetricFlags};

/// A [`metrics_util::Storage`] that uses [`crate::metrics_histogram::Histogram`] for its histogram implementation.
pub struct AtomicStorageWithHistogram;

impl<K> Storage<K> for AtomicStorageWithHistogram {
    type Counter = Arc<AtomicU64>;
    type Gauge = Arc<AtomicU64>;
    type Histogram = Arc<crate::metrics::metrics_histogram::Histogram>;

    fn counter(&self, _: &K) -> Self::Counter {
        Arc::new(AtomicU64::new(0))
    }

    fn gauge(&self, _: &K) -> Self::Gauge {
        Arc::new(AtomicU64::new(0))
    }

    fn histogram(&self, _: &K) -> Self::Histogram {
        Arc::new(crate::metrics::metrics_histogram::Histogram::new())
    }
}

struct MetricRecorderInner {
    emit_zero_counters: bool,
    registry: Registry<metrics::Key, AtomicStorageWithHistogram>,
    units: RwLock<HashMap<String, metrique_writer_core::Unit>>,
}

/// The metric recorder belonging to this crate. Accumulates metrics in a registry
/// and lets them be read out via `readout`
#[derive(Clone)]
pub struct MetricRecorder(Arc<MetricRecorderInner>);

impl MetricRecorder {
    /// Create a new metric recorder
    pub fn new() -> Self {
        Self::new_with_emit_zero_counters(false)
    }

    /// Create a new metric recorder
    ///
    /// If `emit_zero_counters` is true, counters with a value of 0 will be emitted
    pub fn new_with_emit_zero_counters(emit_zero_counters: bool) -> Self {
        Self(Arc::new(MetricRecorderInner::new(emit_zero_counters)))
    }

    /// Read out the current value of the metrics, resetting counters and histograms (and
    /// not resetting gauges).
    pub fn readout(&self) -> MetricAccumulatorEntry {
        self.0.readout()
    }
}

impl Default for MetricRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricRecorderInner {
    fn new(emit_zero_counters: bool) -> Self {
        Self {
            emit_zero_counters,
            registry: Registry::new(AtomicStorageWithHistogram),
            units: RwLock::new(HashMap::new()),
        }
    }

    fn readout(&self) -> MetricAccumulatorEntry {
        let mut counters = Vec::new();
        let mut gauges = Vec::new();
        let mut histograms = Vec::new();
        self.registry.visit_counters(|key, counter| {
            let counter = counter.swap(0, Ordering::Relaxed);
            // don't include counters that weren't incremented in the log
            if self.emit_zero_counters || counter != 0 {
                counters.push((key.clone(), counter));
            }
        });
        counters.sort_by(|u, v| u.0.cmp(&v.0));
        self.registry.visit_gauges(|key, gauge| {
            gauges.push((key.clone(), f64::from_bits(gauge.load(Ordering::Relaxed))));
        });
        gauges.sort_by(|u, v| u.0.cmp(&v.0));
        self.registry.visit_histograms(|key, histogram| {
            histograms.push((key.clone(), histogram.drain()));
        });
        histograms.sort_by(|u, v| u.0.cmp(&v.0));
        MetricAccumulatorEntry {
            counters,
            gauges,
            histograms,
            units: self.units.read().unwrap().clone(),
            timestamp: Some(SystemTime::now()),
        }
    }
}

/// Represents a readout of metrics, with values for all the given metrics.
#[derive(Clone, Debug)]
pub struct MetricAccumulatorEntry {
    counters: Vec<(metrics::Key, u64)>,
    gauges: Vec<(metrics::Key, f64)>,
    histograms: Vec<(metrics::Key, Vec<Bucket>)>,
    units: HashMap<String, metrique_writer_core::Unit>,
    timestamp: Option<SystemTime>,
}

impl MetricAccumulatorEntry {
    /// Remove the timestamp from this MetricAccumulatorEntry. This should be used
    /// if it is nested in a different metrics struct to avoid double timestamp
    /// recording, which might cause the metrics writer to panic.
    pub fn remove_timestamp(&mut self) {
        self.timestamp = None;
    }

    /// Get the current timestamp from this metrics accumulator entry
    pub fn timestamp(&self) -> Option<SystemTime> {
        self.timestamp
    }
}

#[cfg(any(test, feature = "test-util"))]
impl MetricAccumulatorEntry {
    /// Get counter value. O(n) in number of metrics so use only for tests.
    ///
    /// Use the `Entry` implementation when performance is needed.
    pub fn counter_value(&self, name: &str) -> Option<u64> {
        self.counters
            .iter()
            .find(|(key, _)| key.name() == name)
            .map(|(_, v)| *v)
    }

    /// Get gauge value. O(n) in number of metrics so use only for tests.
    ///
    /// Use the `Entry` implementation when performance is needed.
    pub fn gauge_value(&self, name: &str) -> Option<f64> {
        self.gauges
            .iter()
            .find(|(key, _)| key.name() == name)
            .map(|(_, v)| *v)
    }

    /// Get a list of histogram samples. O(n) in number of histograms so use only for tests
    ///
    /// Note that histograms use sampling which means that the result is somewhat inaccurate.
    ///
    /// Use the `Entry` implementation when performance is needed.
    pub fn histogram_value(&self, name: &str) -> Vec<u32> {
        self.histograms
            .iter()
            .filter(|(key, _)| key.name() == name)
            .flat_map(|(_key, buckets)| buckets)
            .flat_map(|bucket| vec![bucket.value; bucket.count as usize])
            .collect()
    }
}

impl Entry for MetricAccumulatorEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        struct MultiObservation<'a, T> {
            value: T,
            unit: metrique_writer_core::Unit,
            dimensions: Vec<(&'a str, &'a str)>,
        }

        impl<T> metrique_writer_core::Value for MultiObservation<'_, T>
        where
            T: IntoIterator<Item = Observation> + Clone,
        {
            fn write(&self, writer: impl metrique_writer_core::ValueWriter) {
                writer.metric(
                    self.value.clone(),
                    self.unit,
                    self.dimensions.iter().cloned(),
                    MetricFlags::empty(),
                )
            }
        }

        if let Some(timestamp) = self.timestamp {
            writer.timestamp(timestamp);
        }

        // Reporting time-based metrics, split entries is what we want.
        writer.config(&const { metrique_writer_core::config::AllowSplitEntries::new() });

        for (key, value) in &self.counters {
            let labels = key
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect();
            let unit = self
                .units
                .get(key.name())
                .unwrap_or(&metrique_writer_core::Unit::None);
            writer.value(
                key.name(),
                &MultiObservation {
                    value: [Observation::Unsigned(*value)],
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }

        for (key, value) in &self.gauges {
            let labels = key
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect();
            let unit = self
                .units
                .get(key.name())
                .unwrap_or(&metrique_writer_core::Unit::None);
            writer.value(
                key.name(),
                &MultiObservation {
                    value: [Observation::Floating(*value)],
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }

        for (key, buckets) in &self.histograms {
            let labels = key
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect();
            let unit = self
                .units
                .get(key.name())
                .unwrap_or(&metrique_writer_core::Unit::None);
            let observations = buckets.iter().map(|bucket| Observation::Repeated {
                total: bucket.value as f64 * bucket.count as f64,
                occurrences: bucket.count as u64,
            });
            writer.value(
                key.name(),
                &MultiObservation {
                    value: observations,
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }
    }
}

impl Recorder for MetricRecorder {
    fn describe_counter(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
        self.0
            .units
            .write()
            .unwrap()
            .insert(key.as_str().to_string(), metrics_unit_to_metrique_unit(unit));
    }

    fn describe_gauge(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
        self.0
            .units
            .write()
            .unwrap()
            .insert(key.as_str().to_string(), metrics_unit_to_metrique_unit(unit));
    }

    fn describe_histogram(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
        self.0
            .units
            .write()
            .unwrap()
            .insert(key.as_str().to_string(), metrics_unit_to_metrique_unit(unit));
    }

    fn register_counter(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Counter {
        metrics::Counter::from_arc(self.0.registry.get_or_create_counter(key, Clone::clone))
    }

    fn register_gauge(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Gauge {
        metrics::Gauge::from_arc(self.0.registry.get_or_create_gauge(key, Clone::clone))
    }

    fn register_histogram(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        metrics::Histogram::from_arc(self.0.registry.get_or_create_histogram(key, Clone::clone))
    }
}

/// A Cloneable dynamic recorder that implements the Recorder trait
#[derive(Clone)]
pub struct SharedRecorder(Arc<dyn Recorder + Send + Sync>);
impl SharedRecorder {
    pub fn new(recorder: Arc<dyn Recorder + Send + Sync>) -> Self {
        Self(recorder)
    }
}

impl Debug for SharedRecorder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRecorder").finish()
    }
}

impl Recorder for SharedRecorder {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.describe_counter(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.describe_gauge(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.describe_histogram(key, unit, description);
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        self.0.register_counter(key, metadata)
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        self.0.register_gauge(key, metadata)
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        self.0.register_histogram(key, metadata)
    }
}

#[cfg(test)]
mod test {
    use metrics::{histogram, with_local_recorder};
    use metrique_writer_core::{format::Format, test_stream::DummyFormat};
    use test_case::test_case;

    use crate::metrics::MetricRecorder;

    #[test_case(false, None; "no_emit_zero_counters")]
    #[test_case(true, Some(0); "emit_zero_counters")]
    fn test_emit_zero_counters(emit_zero_counters: bool, expected_result: Option<u64>) {
        let accumulator: MetricRecorder =
            MetricRecorder::new_with_emit_zero_counters(emit_zero_counters);
        metrics::with_local_recorder(&accumulator, || {
            metrics::counter!("a").increment(1);
        });
        let read0 = accumulator.readout();
        assert_eq!(read0.counter_value("a"), Some(1));
        let read1 = accumulator.readout();
        assert_eq!(read1.counter_value("a"), expected_result);
    }

    #[test]
    fn simple() {
        let recorder = MetricRecorder::new();
        with_local_recorder(&recorder, || {
            let histogram = histogram!("test");
            histogram.record(1);
            histogram.record(2);
            histogram.record(2);
            histogram.record(3);
            histogram.record(3);
            histogram.record(3);
        });
        let mut readout = recorder.readout();
        // force some timestamp for test purposes
        readout.timestamp =
            Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400));
        let mut writer = DummyFormat;
        let mut output = Vec::new();

        writer.format(&readout, &mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output,
            r#"[("timestamp", "86400"), ("test", "[Repeated { total: 1.0, occurrences: 1 }, Repeated { total: 4.0, occurrences: 2 }, Repeated { total: 9.0, occurrences: 3 }] None []")]"#
        );
    }
}
