use std::{collections::HashMap, hash::Hash, marker::PhantomData};

use crate::{MetricAccumulatorEntry, MetricRecorder};

mod private {
    pub trait Sealed {}
    #[cfg(feature = "metrics-rs-024")]
    impl Sealed for dyn metrics_024::Recorder {}

    pub trait Sealed2<V: super::MetricsRsVersion + ?Sized> {}

    impl<M> Sealed for super::YouMustSpecifyAMetricsRsVersion<M> {}
}

// internal trait to make sure inference doesn't magic-pick a metrics.rs
// version. The generic parameter is to make sure that
// `PrivateMetricVersionForInference` itself can't be picked.
#[allow(unused)]
struct YouMustSpecifyAMetricsRsVersion<M: 'static>(PhantomData<M>);
#[diagnostic::do_not_recommend]
impl<M> MetricsRsVersion for YouMustSpecifyAMetricsRsVersion<M> {
    type Key = ();
    type AtomicStorageWithHistogramRegistry = ();
    type Recorder = ();
    fn new_atomic_storage_with_histogram_registry() {}
    fn readout(
        _registry: &Self::AtomicStorageWithHistogramRegistry,
        _emit_zero_counters: bool,
        units: impl FnOnce() -> HashMap<String, metrique_writer_core::Unit>,
    ) -> MetricAccumulatorEntry<Self> {
        MetricAccumulatorEntry {
            counters: vec![],
            gauges: vec![],
            histograms: vec![],
            units: units(),
            timestamp: Some(metrique_timesource::time_source().system_time()),
        }
    }
    fn key_name(_name: &Self::Key) -> &str {
        ""
    }
    fn key_labels(_key: &Self::Key) -> Vec<(&str, &str)> {
        vec![]
    }
    fn set_global_recorder(_recorder: MetricRecorder<Self>) {}
}
#[diagnostic::do_not_recommend]
impl<M> ParametricRecorder<YouMustSpecifyAMetricsRsVersion<M>>
    for MetricRecorder<YouMustSpecifyAMetricsRsVersion<M>>
{
    fn with_local_recorder<T>(&self, body: impl FnOnce() -> T) -> T {
        body()
    }
}
impl<M> private::Sealed2<YouMustSpecifyAMetricsRsVersion<M>>
    for MetricRecorder<YouMustSpecifyAMetricsRsVersion<M>>
{
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

#[cfg(feature = "metrics-rs-024")]
mod impls {
    use std::{collections::HashMap, sync::atomic::Ordering};

    use crate::{
        MetricAccumulatorEntry, MetricRecorder, MetricsRsVersion, ParametricRecorder,
        accumulator::AtomicStorageWithHistogram,
    };

    impl MetricsRsVersion for dyn metrics_024::Recorder {
        #[doc(hidden)]
        type Key = metrics_024::Key;
        #[doc(hidden)]
        type AtomicStorageWithHistogramRegistry =
            metrics_util_020::registry::Registry<metrics_024::Key, AtomicStorageWithHistogram>;
        #[doc(hidden)]
        type Recorder = dyn metrics_024::Recorder + Send + Sync;
        #[doc(hidden)]
        fn new_atomic_storage_with_histogram_registry() -> Self::AtomicStorageWithHistogramRegistry
        {
            metrics_util_020::registry::Registry::new(AtomicStorageWithHistogram)
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
                timestamp: Some(metrique_timesource::time_source().system_time()),
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
            metrics_024::set_global_recorder(recorder).expect("failed to set global recorder");
        }
    }

    impl<R: metrics_024::Recorder> ParametricRecorder<dyn metrics_024::Recorder> for R {
        fn with_local_recorder<T>(&self, body: impl FnOnce() -> T) -> T {
            metrics_024::with_local_recorder(self, body)
        }
    }

    impl<R: metrics_024::Recorder> super::private::Sealed2<dyn metrics_024::Recorder> for R {}
}

/// A trait to allow the metrics.rs bridge to be generic over versions of metrics.rs
/// [`metrics::Recorder`]s. This trait is not to be manually implemented or called by
/// users.
///
/// [`metrics::Recorder`]: metrics_024::Recorder
pub trait ParametricRecorder<V: MetricsRsVersion + ?Sized>: private::Sealed2<V> {
    #[doc(hidden)]
    fn with_local_recorder<T>(&self, body: impl FnOnce() -> T) -> T;
}
