use std::{collections::HashMap, hash::Hash, sync::atomic::Ordering, time::SystemTime};

use crate::metrics::{
    MetricAccumulatorEntry, MetricRecorder, accumulator::AtomicStorageWithHistogram,
};

mod private {
    pub trait Sealed {}
    #[cfg(feature = "metrics_rs_024")]
    impl Sealed for dyn metrics::Recorder {}

    pub trait Sealed2<V: super::MetricsRsVersion + ?Sized> {}
}

/// A trait to allow the metrics.rs bridge to be generic over metrics.rs versions
///
/// This is not to be implemented or called directly by users.
pub trait MetricsRsVersion: 'static + private::Sealed {
    #[doc(hidden)]
    type Key: Hash + Send;
    #[doc(hidden)]
    type AtomicStorageWithHistogramRegistry: Send + Sync;
    #[doc(hidden)]
    type Recorder: ?Sized;
    #[doc(hidden)]
    fn new_atomic_storage_with_histogram_registry() -> Self::AtomicStorageWithHistogramRegistry;
    #[doc(hidden)]
    fn readout(
        registry: &Self::AtomicStorageWithHistogramRegistry,
        emit_zero_counters: bool,
        units: impl FnOnce() -> HashMap<String, metrique_writer_core::Unit>,
    ) -> MetricAccumulatorEntry<Self>;
    #[doc(hidden)]
    fn key_name(name: &Self::Key) -> &str;
    #[doc(hidden)]
    fn key_labels(key: &Self::Key) -> Vec<(&str, &str)>;
    #[doc(hidden)]
    fn set_global_recorder(recorder: MetricRecorder<Self>);
}

#[cfg(feature = "metrics_rs_024")]
impl MetricsRsVersion for dyn metrics::Recorder {
    #[doc(hidden)]
    type Key = metrics::Key;
    #[doc(hidden)]
    type AtomicStorageWithHistogramRegistry =
        metrics_util::registry::Registry<metrics::Key, AtomicStorageWithHistogram>;
    #[doc(hidden)]
    type Recorder = dyn metrics::Recorder + Send + Sync;
    #[doc(hidden)]
    fn new_atomic_storage_with_histogram_registry() -> Self::AtomicStorageWithHistogramRegistry {
        metrics_util::registry::Registry::new(AtomicStorageWithHistogram)
    }
    fn readout(
        registry: &Self::AtomicStorageWithHistogramRegistry,
        emit_zero_counters: bool,
        units: impl FnOnce() -> HashMap<String, metrique_writer_core::Unit>,
    ) -> MetricAccumulatorEntry<Self> {
        let mut counters = Vec::new();
        let mut gauges = Vec::new();
        let mut histograms = Vec::new();
        registry.visit_counters(|key, counter| {
            let counter = counter.swap(0, Ordering::Relaxed);
            // don't include counters that weren't incremented in the log
            if emit_zero_counters || counter != 0 {
                counters.push((key.clone(), counter));
            }
        });
        counters.sort_by(|u, v| u.0.cmp(&v.0));
        registry.visit_gauges(|key, gauge| {
            gauges.push((key.clone(), f64::from_bits(gauge.load(Ordering::Relaxed))));
        });
        gauges.sort_by(|u, v| u.0.cmp(&v.0));
        registry.visit_histograms(|key, histogram| {
            histograms.push((key.clone(), histogram.drain()));
        });
        histograms.sort_by(|u, v| u.0.cmp(&v.0));
        MetricAccumulatorEntry {
            counters,
            gauges,
            histograms,
            units: units(),
            timestamp: Some(SystemTime::now()),
        }
    }
    fn key_name(key: &Self::Key) -> &str {
        key.name()
    }
    fn key_labels(key: &Self::Key) -> Vec<(&str, &str)> {
        key.labels()
            .map(|label| (label.key(), label.value()))
            .collect()
    }
    #[track_caller]
    fn set_global_recorder(recorder: MetricRecorder<Self>) {
        metrics::set_global_recorder(recorder).expect("failed to set global recorder");
    }
}

/// A trait to allow the metrics.rs bridge to be generic over versions of metrics.rs
/// [metrics::Recorder]s. This trait is not to be manually implemented or called by
/// users.
pub trait ParametricRecorder<V: MetricsRsVersion + ?Sized>: private::Sealed2<V> {
    #[doc(hidden)]
    fn with_local_recorder<T>(&self, body: impl FnOnce() -> T) -> T;
}

#[cfg(feature = "metrics_rs_024")]
impl<R: metrics::Recorder> ParametricRecorder<dyn metrics::Recorder> for R {
    fn with_local_recorder<T>(&self, body: impl FnOnce() -> T) -> T {
        metrics::with_local_recorder(self, body)
    }
}
#[cfg(feature = "metrics_rs_024")]
impl<R: metrics::Recorder> private::Sealed2<dyn metrics::Recorder> for R {}
