This mode provides a few [`metrics::Recorder`]s that can be used for emitting metrics
via metrique-writer. This includes [`MetricsReporter`]  that is designed for use in EC2/Fargate,
[`lambda_reporter`] that is designed for use in Lambda, and [`capture`] that is
designed for use in unit tests.

See the linked documentation pages for examples.

This allows capturing metrics emitted via the metrics.rs facade into metrique.

[`metrics::Recorder`]: metrics_024::Recorder
[`MetricsReporter`]: crate::MetricReporter
[`lambda_reporter`]: crate::lambda_reporter
[`capture`]: crate::capture