metrique is a crate to emit unit-of-work metrics

## Getting Started (Applications)

Most metrics your application records will be "unit of work" metrics. These are typically tied to the request/response scope. You declare a struct the represents the metrics you plan to capture over the course of the request and annotate it with `#[metrics]`

This library exposes a wrapper `<MetricName>Guard` type that implicitly appends to a global queue when the struct is dropped. This wrapper in turn exposes other APIs to further tune behavior.

```rust
use std::path::PathBuf;

use metrique::unit_of_work::metrics;
use metrique::timers::{Timestamp, Timer};
use metrique::unit::Millisecond;
use metrique_writer::{GlobalEntrySink, sink::global_entry_sink};
use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, sink::AttachHandle};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(timestamp)]
    timestamp: Timestamp,
    number_of_ducks: usize,
    #[metrics(unit = Millisecond)]
    operation_time: Timer,
}

impl RequestMetrics {
    // It is generally a good practice to expose a single initializer that sets up
    // append on drop.
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            timestamp: Timestamp::now(),
            operation,
            number_of_ducks: 0,
            operation_time: Timer::start_now(),
        }.append_on_drop(ServiceMetrics::sink())
    }
}

async fn count_ducks() {
    let mut metrics = RequestMetrics::init("CountDucks");
    metrics.number_of_ducks = 5;
    // metrics flushes as scope drops
    // timer records the total time until scope exits
}


fn initialize_metrics(service_log_dir: PathBuf) -> AttachHandle {
    ServiceMetrics::attach_to_stream(
        Emf::builder("Ns".to_string(), vec![vec![]])
            .build()
            .output_to_makewriter(RollingFileAppender::new(
                Rotation::MINUTELY,
                &service_log_dir,
                "service_log.log",
            )),
    )
}

fn main() {
    tracing_subscriber::fmt::init();
    initialize_metrics("my/metrics/dir".into());
    // ...
    // call count_ducks
}
```

## Getting Started (Libraries)

Library operations should normally return a struct implementing `CloseEntry` that contains the metrics for their operation.

Read docs/usage_in_libraries.md for more details

## Security Concerns

### Sensitive information in metrics

Metrics and logs are often exported to places where they can be read by a large number of people. Therefore, it is important to keep sensitive information, including secret keys and private information, out of them.

The `metrique` library intentionally does not have mechanisms that put *unexpected* data within metric entries (for example, bridges from `Debug` implementations that can put unexpected struct fields in metrics).

However, the `metrique` library controls neither the information placed in metric entries nor where the metrics end up. Therefore, it is your responsibility of an application writer to avoid using the `metrique` library to emit sensitive information to where it shouldn't be present.

### Metrics being dropped

The `metrique` library is intended to be used for operational metrics, and therefore it is intentionally designed to drop metrics under high-load conditions rather than having the application grind to a halt.

There are 2 *main* places where this can happen:

1. `BackgroundQueue` will drop the earliest metric in the queue under load.
2. It is possible to explicitly enable sampling (by using
   `sample_by_fixed_fraction` or `sample_by_congress_at_fixed_entries_per_second`).
   If sampling is being used, metrics will be dropped at random.

If your application's security relies on metric entries not being dropped (for example,
if you use metric entries to track user log-in operations, and your application relies on log-in operations not being dropped), it is your responsibility to engineer your application to avoid the metrics being dropped.

In that case, you should not be using `BackgroundQueue` or sampling. It is probably fine to use the `Format` implementations in that case, but I would recommend trying to find ways to test and audit your use-case to make sure nothing is being missed.

### Use of exporters

The `metrique` library does not currently contain any code that exports the metrics outside of the current process. To make a working system, you normally need to integrate the `metrique` library with some exporter such as the [Amazon CloudWatch Agent].

It is your responsibility to ensure that any agents you are using are kept up to date and configured in a secure manner.

[Amazon CloudWatch Agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Generation_CloudWatch_Agent.html