This mode provides a few [`metrics::Recorder`]s that can be used for emitting metrics
via metrique-writer. This includes [`MetricReporter`]  that is designed for use in EC2/Fargate,
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
metrics = "0.24"
metrique-metricsrs = { version = "0.1", features = ["metrics-rs-024"] }
metrique-writer = "0.1"
metrique-writer-format-emf = "0.1"
tracing-appender = "0.2"
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

## Publishing interval and CloudWatch resolution

The reporter's publishing interval and CloudWatch's storage resolution are
separate settings. `metrics_publish_interval` controls how often collected
metrics are sent to the output stream. To store EMF metrics at 1-second
resolution in CloudWatch, wrap that stream in `HighStorageResolution`.
Construct the reporter within a Tokio runtime:

```rust,no_run
# use metrics_024 as metrics;
use std::time::Duration;
use metrique_metricsrs::MetricReporter;
use metrique_writer::format::FormatExt as _;
use metrique_writer_format_emf::{Emf, HighStorageResolution};

let stream = Emf::all_validations("MyNS".into(), vec![vec![]])
    .output_to(std::fs::File::create("metrics.log").unwrap());
let reporter = MetricReporter::builder()
    .metrics_publish_interval(Duration::from_secs(1))
    .metrics_io_stream(HighStorageResolution::from(stream))
    .metrics_rs_version::<dyn metrics::Recorder>()
    .build_and_install();
```

For standard 60-second CloudWatch storage resolution, pass `stream` directly
to `metrics_io_stream` instead. The `metrics_rs_version` call goes after
`metrics_publish_interval`, because choosing the recorder version changes the
builder type. The example writes EMF to a local file. Delivering that file to
CloudWatch is a separate step.

[`metrics::Recorder`]: https://docs.rs/metrics/latest/metrics/trait.Recorder.html
[`MetricReporter`]: https://docs.rs/metrique-metricsrs/latest/metrique_metricsrs/struct.MetricReporter.html
[`lambda_reporter`]: https://docs.rs/metrique-metricsrs/latest/metrique_metricsrs/lambda_reporter/
[`capture`]: https://docs.rs/metrique-metricsrs/latest/metrique_metricsrs/capture/
