// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

/// Defines callbacks for recording metrics
pub trait MetricRecorder: Send + Sync {
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
#[cfg(feature = "metrics-rs-024")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct GlobalMetricsRs024Bridge;

#[cfg(feature = "metrics-rs-024")]
impl MetricRecorder for GlobalMetricsRs024Bridge {
    fn record_histogram(&self, metric: &'static str, sink: &str, value: u32) {
        metrics_024::histogram!(metric, "sink" => sink.to_owned()).record(value);
    }

    fn increment_counter(&self, metric: &'static str, sink: &str, value: u64) {
        metrics_024::counter!(metric, "sink" => sink.to_owned()).increment(value);
    }

    fn set_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics_024::gauge!(metric, "sink" => sink.to_owned()).set(value);
    }

    fn increment_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics_024::gauge!(metric, "sink" => sink.to_owned()).increment(value);
    }

    fn decrement_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        metrics_024::gauge!(metric, "sink" => sink.to_owned()).decrement(value);
    }
}

/// Implements MetricRecorder for a local metrics-rs 0.24 recorder
#[cfg(feature = "metrics-rs-024")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct LocalMetricsRs024Bridge<R>(pub(crate) R);

#[cfg(feature = "metrics-rs-024")]
impl<R: metrics_024::Recorder + Send + Sync> MetricRecorder for LocalMetricsRs024Bridge<R> {
    fn record_histogram(&self, metric: &'static str, sink: &str, value: u32) {
        self.0
            .register_histogram(
                &metrics_024::Key::from_parts(
                    metric,
                    vec![metrics_024::Label::new("sink", sink.to_owned())],
                ),
                &metrics_024::Metadata::new(
                    module_path!(),
                    metrics_024::Level::INFO,
                    Some(module_path!()),
                ),
            )
            .record(value);
    }

    fn increment_counter(&self, metric: &'static str, sink: &str, value: u64) {
        self.0
            .register_counter(
                &metrics_024::Key::from_parts(
                    metric,
                    vec![metrics_024::Label::new("sink", sink.to_owned())],
                ),
                &metrics_024::Metadata::new(
                    module_path!(),
                    metrics_024::Level::INFO,
                    Some(module_path!()),
                ),
            )
            .increment(value);
    }

    fn set_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics_024::Key::from_parts(
                    metric,
                    vec![metrics_024::Label::new("sink", sink.to_owned())],
                ),
                &metrics_024::Metadata::new(
                    module_path!(),
                    metrics_024::Level::INFO,
                    Some(module_path!()),
                ),
            )
            .set(value);
    }

    fn increment_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics_024::Key::from_parts(
                    metric,
                    vec![metrics_024::Label::new("sink", sink.to_owned())],
                ),
                &metrics_024::Metadata::new(
                    module_path!(),
                    metrics_024::Level::INFO,
                    Some(module_path!()),
                ),
            )
            .increment(value);
    }

    fn decrement_gauge(&self, metric: &'static str, sink: &str, value: f64) {
        self.0
            .register_gauge(
                &metrics_024::Key::from_parts(
                    metric,
                    vec![metrics_024::Label::new("sink", sink.to_owned())],
                ),
                &metrics_024::Metadata::new(
                    module_path!(),
                    metrics_024::Level::INFO,
                    Some(module_path!()),
                ),
            )
            .decrement(value);
    }
}

pub(crate) trait GlobalRecorderVersion {
    fn recorder() -> impl MetricRecorder + 'static;
    fn describe(metrics: &[DescribedMetric]);
}

#[cfg(feature = "metrics-rs-024")]
impl GlobalRecorderVersion for dyn metrics_024::Recorder {
    fn describe(metrics: &[DescribedMetric]) {
        for metric in metrics {
            let unit = match metric.unit {
                MetricsRsUnit::Count => metrics_024::Unit::Count,
                MetricsRsUnit::Percent => metrics_024::Unit::Percent,
                MetricsRsUnit::Millisecond => metrics_024::Unit::Milliseconds,
            };
            match metric.r#type {
                MetricsRsType::Counter => {
                    metrics_024::describe_counter!(metric.name, unit, metric.description)
                }
                MetricsRsType::Gauge => {
                    metrics_024::describe_gauge!(metric.name, unit, metric.description)
                }
                MetricsRsType::Histogram => {
                    metrics_024::describe_histogram!(metric.name, unit, metric.description)
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
    fn recorder(recorder: R) -> impl MetricRecorder + 'static;
}

#[cfg(feature = "metrics-rs-024")]
impl<R> LocalRecorderVersion<R> for dyn metrics_024::Recorder
where
    R: metrics_024::Recorder + Send + Sync + 'static,
{
    fn recorder(recorder: R) -> impl MetricRecorder + 'static {
        LocalMetricsRs024Bridge(recorder)
    }
}
