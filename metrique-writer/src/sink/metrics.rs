// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

/// Defines callbacks for recording metrics
pub trait MetricRecorder {
    /// Records a histogram entry. metric is used to define the metric, sink to define the sink, value the histogram value
    fn record_histogram(&self, metric: &'static str, sink: &str, value: u32);
    /// Increments a histogram entry. metric is used to define the metric, sink to define the sink, value the histogram value
    fn increment_counter(&self, metric: &'static str, sink: &str, value: u64);
    /// Sets a gauge entry. metric is used to define the metric, sink to define the sink, value the histogram value
    fn set_gauge(&self, metric: &'static str, sink: &str, value: f64);
    /// Increments a gauge entry. metric is used to define the metric, sink to define the sink, value the histogram value
    fn increment_gauge(&self, metric: &'static str, sink: &str, value: f64);
    /// Decrements a gauge entry. metric is used to define the metric, sink to define the sink, value the histogram value
    fn decrement_gauge(&self, metric: &'static str, sink: &str, value: f64);
}

/// Implements MetricRecorder for a global metrics-rs 0.24 recorder
#[cfg(feature = "metrics_rs_024")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct GlobalMetricsRs024Bridge;

#[cfg(feature = "metrics_rs_024")]
impl MetricRecorder for GlobalMetricsRs024Bridge {
    fn record_histogram(&self, metric: &'static str, sink: &str, value: u32) {
        metrics::histogram!(metric, "sink" => sink.to_owned()).record(value);
    }

    fn increment_counter(&self, metric: &'static str, sink: &str, value: u64) {
        metrics::counter!(metric, "sink" => sink.to_owned()).increment(value);
    }

    fn set_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics::gauge!(metric, "sink" => sink.to_owned()).set(value);
    }

    fn increment_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics::gauge!(metric, "sink" => sink.to_owned()).increment(value);
    }

    fn decrement_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics::gauge!(metric, "sink" => sink.to_owned()).decrement(value);
    }
}

/// Implements MetricRecorder for a local metrics-rs 0.24 recorder
#[cfg(feature = "metrics_rs_024")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct LocalMetricsRs024Bridge<R>(pub(crate) R);

#[cfg(feature = "metrics_rs_024")]
impl<R: metrics::Recorder> MetricRecorder for LocalMetricsRs024Bridge<R> {
    fn record_histogram(&self, metric: &'static str, sink: &str, value: u32) {
        self.0
            .register_histogram(
                &metrics::Key::from_parts(
                    metric,
                    vec![metrics::Label::new("sink", sink.to_owned())],
                ),
                &metrics::Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!())),
            )
            .record(value);
    }

    fn increment_counter(&self, metric: &'static str, sink: &str, value: u64) {
        self.0
            .register_counter(
                &metrics::Key::from_parts(
                    metric,
                    vec![metrics::Label::new("sink", sink.to_owned())],
                ),
                &metrics::Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!())),
            )
            .increment(value);
    }

    fn set_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics::Key::from_parts(
                    metric,
                    vec![metrics::Label::new("sink", sink.to_owned())],
                ),
                &metrics::Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!())),
            )
            .set(value);
    }

    fn increment_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics::Key::from_parts(
                    metric,
                    vec![metrics::Label::new("sink", sink.to_owned())],
                ),
                &metrics::Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!())),
            )
            .increment(value);
    }

    fn decrement_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics::Key::from_parts(
                    metric,
                    vec![metrics::Label::new("sink", sink.to_owned())],
                ),
                &metrics::Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!())),
            )
            .decrement(value);
    }
}

pub(crate) trait GlobalRecorderVersion {
    fn recorder() -> impl MetricRecorder + Send + 'static;
    fn describe(metrics: &[DescribedMetric]);
}

#[cfg(feature = "metrics_rs_024")]
impl GlobalRecorderVersion for dyn metrics::Recorder {
    fn describe(metrics: &[DescribedMetric]) {
        for metric in metrics {
            let unit = match metric.unit {
                MetricsRsUnit::Count => metrics::Unit::Count,
                MetricsRsUnit::Percent => metrics::Unit::Percent,
                MetricsRsUnit::Millisecond => metrics::Unit::Milliseconds,
            };
            match metric.r#type {
                MetricsRsType::Counter => {
                    metrics::describe_counter!(metric.name, unit, metric.description)
                }
                MetricsRsType::Gauge => {
                    metrics::describe_gauge!(metric.name, unit, metric.description)
                }
                MetricsRsType::Histogram => {
                    metrics::describe_histogram!(metric.name, unit, metric.description)
                }
            }
        }
    }

    fn recorder() -> impl MetricRecorder {
        GlobalMetricsRs024Bridge
    }
}

/// Describes a metrics.rs unit in a non-exhaustive fashion
#[non_exhaustive]
#[derive(Copy, Clone, Debug)]
pub enum MetricsRsUnit {
    Percent,
    Count,
    Millisecond,
}

/// Describes a metrics.rs metric type in a non-exhaustive fashion
#[non_exhaustive]
#[derive(Copy, Clone, Debug)]
pub enum MetricsRsType {
    Gauge,
    Counter,
    Histogram,
}

#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct DescribedMetric {
    pub name: &'static str,
    pub unit: MetricsRsUnit,
    pub r#type: MetricsRsType,
    pub description: &'static str,
}

pub trait LocalRecorderVersion<R> {
    fn recorder(recorder: R) -> impl MetricRecorder + Send + 'static;
}

#[cfg(feature = "metrics_rs_024")]
impl<R> LocalRecorderVersion<R> for dyn metrics::Recorder
where
    R: metrics::Recorder + Send + 'static,
{
    fn recorder(recorder: R) -> impl MetricRecorder + Send + 'static {
        LocalMetricsRs024Bridge(recorder)
    }
}
