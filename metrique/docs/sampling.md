# Sampling

If your service's TPS is too high for full metric emission, sampling lets you
reduce volume while preserving visibility into rare events. See also the
["My TPS is too high"](crate::_guide::cookbook#my-tps-is-too-high)
section in the cookbook.

## Overview

High-volume services may want to trade lower accuracy for lower CPU time spent on metric emission. Offloading metrics to
CloudWatch can become bottlenecked if the agent isn't able to keep up with the rate of written metric entries.

It is common to tee the metric into 2 destinations:

 1. A highly-compressed "log of record" that contains all entries and is eventually persisted to S3 or other long-term storage.
 1. An uncompressed, but sampled, metrics log that is published to CloudWatch.

The sampling can be done naively at some [fixed fraction](`crate::writer::sample::FixedFractionSample`), but at low rates can
cause low-frequency events to be missed. This includes service errors or validation errors, especially when the service is
designed to have an availability much higher than the chosen sample rate. Instead, we recommend the use of the
[congressional sampler](`crate::writer::sample::CongressSample`). It uses a fixed metric emisssion target rate and
gives lower-frequency events a higher sampling rate to boost their accuracy.

The example below uses the congressional sampler keyed by the request operation and the status code to
ensure lower-frequency APIs and status codes have enough samples.

When using EMF, you need to call [`with_sampling`] before calling a sampler, for example:

```rust,no_run
use metrique::unit_of_work::metrics;
use metrique::emf::Emf;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique::writer::sample::SampledFormatExt;
use metrique::writer::stream::tee;
use metrique::ServiceMetrics;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

# let service_log_dir = "./service_log";
# let metrics_log_dir = "./metrics_log";

#[metrics(value(string))]
enum Operation {
    CountDucks,
    // ...
}

#[metrics(rename_all="PascalCase")]
struct RequestMetrics {
    #[metrics(sample_group)]
    operation: Operation,
    #[metrics(sample_group)]
    status_code: &'static str,
    number_of_ducks: u32,
    exception: Option<String>,
}

let _join_service_metrics = ServiceMetrics::attach_to_stream(
    tee(
        // non-uploaded, archived log of record
        Emf::all_validations("MyNS".to_string(), /* dimensions */ vec![vec![], vec!["Operation".to_string()]])
            .output_to_makewriter(RollingFileAppender::new(
                Rotation::MINUTELY,
                service_log_dir,
                "service_log.log",
            )),
        // sampled log, will be uploaded to CloudWatch
        Emf::all_validations("MyNS".to_string(), /* dimensions */ vec![vec![], vec!["Operation".to_string()]])
            .with_sampling()
            .sample_by_congress_at_fixed_entries_per_second(100)
            .output_to_makewriter(RollingFileAppender::new(
                Rotation::MINUTELY,
                metrics_log_dir,
                "metric_log.log",
            )),
    )
);

let metric = RequestMetrics {
    operation: Operation::CountDucks,
    status_code: "OK",
    number_of_ducks: 2,
    exception: None,
}.append_on_drop(ServiceMetrics::sink());

// _join_service_metrics drop (e.g. during service shutdown) blocks until the queue is drained
```

[`with_sampling`]: crate::emf::Emf::with_sampling
