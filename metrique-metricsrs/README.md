This mode provides a few [`metrics::Recorder`]s that can be used for emitting metrics
via metrique-writer. This includes [`MetricsReporter`]  that is designed for use in EC2/Fargate,
[`lambda_reporter`] that is designed for use in Lambda, and [`capture`] that is
designed for use in unit tests.

See the linked documentation pages for examples.

This allows capturing metrics emitted via the metrics.rs facade into metrique.

This crate intends to be able to support multiple metrics.rs versions with a single
`metrique-metricsrs` major version. Therefore, you'll need to enable feature flags
corresponding to the `metrics.rs` version you are using, and pass the version via
a `dyn metrics::Recorder` "witness".

For example, enable this in your `Cargo.toml`:

```toml
[dependencies]
...
metrique-metricsrs = { version = "0.1", features = ["metrics-rs-024"] }
```

Then in your main code, for example:

```rust,no_run
# use metrics_024 as metrics;
use metrique_metricsrs::MetricReporter;
use metrique_writer::{Entry, EntryIoStream, FormatExt, EntryIoStreamExt};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

let log_dir = std::path::PathBuf::from("example");
let logger = MetricReporter::builder()
    .metrics_rs_version::<dyn metrics::Recorder>()
    .metrics_io_stream(Emf::all_validations("MyNS".to_string(),
        vec![vec![], vec!["service".to_string()]]).output_to_makewriter(
            RollingFileAppender::new(Rotation::HOURLY, &log_dir, "metric_log.log")
        )
    )
    .build_and_install();
```

Currently, there is only 1 metrics.rs version supported (0.24), but when there
will be more, having the feature-flag for an unused metrics.rs version will do no harm.

[`metrics::Recorder`]: metrics_024::Recorder
[`MetricsReporter`]: crate::MetricReporter
[`lambda_reporter`]: crate::lambda_reporter
[`capture`]: crate::capture