// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::{Arc, RwLock},
    time::SystemTime,
};

use crate::{MetricsRsVersion, metrics_histogram::Bucket};
use derive_where::derive_where;
use metrique_writer_core::{Entry, EntryWriter, Observation, value::MetricFlags};

/// A [`metrics_util::Storage`] that uses [`crate::metrics_histogram::Histogram`] for its histogram implementation.
#[cfg_attr(not(feature = "metrics_rs_024"), allow(unused))]
pub struct AtomicStorageWithHistogram;

#[cfg(feature = "metrics_rs_024")]
mod impls_024 {
    use std::sync::{Arc, atomic::AtomicU64};

    use metrics_024::{
        Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
    };
    use metrics_util_020::registry::Storage;

    use crate::{MetricRecorder, unit::metrics_024_unit_to_metrique_unit};

    impl<K> Storage<K> for super::AtomicStorageWithHistogram {
        type Counter = Arc<AtomicU64>;
        type Gauge = Arc<AtomicU64>;
        type Histogram = Arc<crate::metrics_histogram::Histogram>;

        fn counter(&self, _: &K) -> Self::Counter {
            Arc::new(AtomicU64::new(0))
        }

        fn gauge(&self, _: &K) -> Self::Gauge {
            Arc::new(AtomicU64::new(0))
        }

        fn histogram(&self, _: &K) -> Self::Histogram {
            Arc::new(crate::metrics_histogram::Histogram::new())
        }
    }

    impl Recorder for MetricRecorder<dyn metrics_024::Recorder> {
        fn describe_counter(
            &self,
            key: metrics_024::KeyName,
            unit: Option<metrics_024::Unit>,
            _description: metrics_024::SharedString,
        ) {
            self.0.units.write().unwrap().insert(
                key.as_str().to_string(),
                metrics_024_unit_to_metrique_unit(unit),
            );
        }

        fn describe_gauge(
            &self,
            key: metrics_024::KeyName,
            unit: Option<metrics_024::Unit>,
            _description: metrics_024::SharedString,
        ) {
            self.0.units.write().unwrap().insert(
                key.as_str().to_string(),
                metrics_024_unit_to_metrique_unit(unit),
            );
        }

        fn describe_histogram(
            &self,
            key: metrics_024::KeyName,
            unit: Option<metrics_024::Unit>,
            _description: metrics_024::SharedString,
        ) {
            self.0.units.write().unwrap().insert(
                key.as_str().to_string(),
                metrics_024_unit_to_metrique_unit(unit),
            );
        }

        fn register_counter(
            &self,
            key: &metrics_024::Key,
            _metadata: &metrics_024::Metadata<'_>,
        ) -> metrics_024::Counter {
            metrics_024::Counter::from_arc(self.0.registry.get_or_create_counter(key, Clone::clone))
        }

        fn register_gauge(
            &self,
            key: &metrics_024::Key,
            _metadata: &metrics_024::Metadata<'_>,
        ) -> metrics_024::Gauge {
            metrics_024::Gauge::from_arc(self.0.registry.get_or_create_gauge(key, Clone::clone))
        }

        fn register_histogram(
            &self,
            key: &metrics_024::Key,
            _metadata: &metrics_024::Metadata<'_>,
        ) -> metrics_024::Histogram {
            metrics_024::Histogram::from_arc(
                self.0.registry.get_or_create_histogram(key, Clone::clone),
            )
        }
    }

    impl Recorder for super::SharedRecorder<dyn metrics_024::Recorder> {
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
}

struct MetricRecorderInner<V: MetricsRsVersion + ?Sized> {
    emit_zero_counters: bool,
    registry: V::AtomicStorageWithHistogramRegistry,
    units: RwLock<HashMap<String, metrique_writer_core::Unit>>,
}

/// The metric recorder belonging to this crate. Accumulates metrics in a registry
/// and lets them be read out via `readout`
#[derive_where(Clone; )]
pub struct MetricRecorder<V: MetricsRsVersion + ?Sized>(Arc<MetricRecorderInner<V>>);

impl<V: MetricsRsVersion + ?Sized> MetricRecorder<V> {
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
    pub fn readout(&self) -> MetricAccumulatorEntry<V> {
        self.0.readout()
    }
}

impl<V: MetricsRsVersion + ?Sized> Default for MetricRecorder<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: MetricsRsVersion + ?Sized> MetricRecorderInner<V> {
    fn new(emit_zero_counters: bool) -> Self {
        Self {
            emit_zero_counters,
            registry: V::new_atomic_storage_with_histogram_registry(),
            units: RwLock::new(HashMap::new()),
        }
    }

    fn readout(&self) -> MetricAccumulatorEntry<V> {
        V::readout(&self.registry, self.emit_zero_counters, || {
            self.units.read().unwrap().clone()
        })
    }
}

/// Represents a readout of metrics, with values for all the given metrics.
#[derive(Clone, Debug)]
pub struct MetricAccumulatorEntry<V: MetricsRsVersion + ?Sized> {
    pub(crate) counters: Vec<(V::Key, u64)>,
    pub(crate) gauges: Vec<(V::Key, f64)>,
    pub(crate) histograms: Vec<(V::Key, Vec<Bucket>)>,
    pub(crate) units: HashMap<String, metrique_writer_core::Unit>,
    pub(crate) timestamp: Option<SystemTime>,
}

impl<V: MetricsRsVersion + ?Sized> MetricAccumulatorEntry<V> {
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
impl<V: MetricsRsVersion + ?Sized> MetricAccumulatorEntry<V> {
    /// Get counter value. O(n) in number of metrics so use only for tests.
    ///
    /// Use the `Entry` implementation when performance is needed.
    pub fn counter_value(&self, name: &str) -> Option<u64> {
        self.counters
            .iter()
            .find(|(key, _)| V::key_name(key) == name)
            .map(|(_, v)| *v)
    }

    /// Get gauge value. O(n) in number of metrics so use only for tests.
    ///
    /// Use the `Entry` implementation when performance is needed.
    pub fn gauge_value(&self, name: &str) -> Option<f64> {
        self.gauges
            .iter()
            .find(|(key, _)| V::key_name(key) == name)
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
            .filter(|(key, _)| V::key_name(key) == name)
            .flat_map(|(_key, buckets)| buckets)
            .flat_map(|bucket| vec![bucket.value; bucket.count as usize])
            .collect()
    }
}

impl<V: MetricsRsVersion + ?Sized> Entry for MetricAccumulatorEntry<V> {
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
            let labels = V::key_labels(key);
            let unit = self
                .units
                .get(V::key_name(key))
                .unwrap_or(&metrique_writer_core::Unit::None);
            writer.value(
                V::key_name(key),
                &MultiObservation {
                    value: [Observation::Unsigned(*value)],
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }

        for (key, value) in &self.gauges {
            let labels = V::key_labels(key);
            let unit = self
                .units
                .get(V::key_name(key))
                .unwrap_or(&metrique_writer_core::Unit::None);
            writer.value(
                V::key_name(key),
                &MultiObservation {
                    value: [Observation::Floating(*value)],
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }

        for (key, buckets) in &self.histograms {
            let labels = V::key_labels(key);
            let unit = self
                .units
                .get(V::key_name(key))
                .unwrap_or(&metrique_writer_core::Unit::None);
            let observations = buckets.iter().map(|bucket| Observation::Repeated {
                total: bucket.value as f64 * bucket.count as f64,
                occurrences: bucket.count as u64,
            });
            writer.value(
                V::key_name(key),
                &MultiObservation {
                    value: observations,
                    unit: *unit,
                    dimensions: labels,
                },
            );
        }
    }
}

/// A Cloneable dynamic recorder that implements the Recorder trait
#[derive(Clone)]
pub struct SharedRecorder<V: MetricsRsVersion + ?Sized>(Arc<V::Recorder>);
impl<V: MetricsRsVersion> SharedRecorder<V> {
    /// Creates a new [SharedRecorder]
    pub fn new(recorder: Arc<V::Recorder>) -> Self {
        Self(recorder)
    }
}

impl<V: MetricsRsVersion> Debug for SharedRecorder<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRecorder").finish()
    }
}

#[cfg(feature = "metrics_rs_024")]
#[cfg(test)]
mod test {
    use metrics_024::{histogram, with_local_recorder};
    use metrique_writer_core::{format::Format, test_stream::DummyFormat};
    use test_case::test_case;

    use crate::MetricRecorder;

    #[test_case(false, None; "no_emit_zero_counters")]
    #[test_case(true, Some(0); "emit_zero_counters")]
    fn test_emit_zero_counters(emit_zero_counters: bool, expected_result: Option<u64>) {
        let accumulator: MetricRecorder<dyn metrics_024::Recorder> =
            MetricRecorder::new_with_emit_zero_counters(emit_zero_counters);
        metrics_024::with_local_recorder(&accumulator, || {
            metrics_024::counter!("a").increment(1);
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
