# Using `metrique` with the EMF Format

`metrique` has out-of-the box support for creating logs in [EMF (Embedded Metrics Format)](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html). There are a few key concepts to understand to effectively leverage EMF.

## Dimensions
EMF has the concept of dimensions. When your metrics are ingested to CloudWatch, they will **only** be aggregated against the dimensions that you specify. Each unique "dimension set" creates a new metric in CloudWatch. If you need to analyze after the fact, you can use [Cloudwatch Log Insights] (adhoc general purpose search and analysis) and [Cloudwatch Contributor Insights] (an online processing system that aggregates your data according to predefined rules).

> **Note**: Dimensions MUST be string values. metrique will not perform any automatic conversions of dimension values into strings.

There are two ways you define dimensions with `metrique`:

1. Global Dimensions: These are set on the [`Emf`] itself. When validations are enabled, **every** entry that is emitted MUST have these set. Global dimensions are set when you construct the builder:

   ```rust
    use metrique_writer_format_emf::Emf;
    let emf = Emf::builder(
       // namespace
        "Ns".to_string(),
        // global dimensions
        vec![vec!["region".to_string()]],
    )
    .build();
   ```

   Applications frequently have "devops" dimensions like `region`, `cell`, etc. Since these are outside of your direct business logic,
   it is often useful to set them on the sink itself via `merge_globals`.

   Using `merge_globals` also ensures that libraries that publish to the global sink use your devops dimensions.

   ```rust
   use std::io::BufWriter;
   use metrique::ServiceMetrics;
   use metrique_writer::{
       FormatExt,
       AttachGlobalEntrySinkExt,
       EntryIoStreamExt,
       Entry, EntryIoStream, GlobalEntrySink,
       sink::global_entry_sink,
   };
   use metrique_writer_format_emf::Emf;

   #[derive(Entry)]
   struct Globals {
       region: String,
   }

   let globals = Globals {
       // Generally, this is usually sourced from CLI args or the environment
       region: "us-east-1".to_string(),
   };

   let _handle = ServiceMetrics::attach_to_stream(
       Emf::builder("Ns".to_string(), vec![vec![]])
           .build()
           .output_to(std::io::stdout())
           // All entries will contain `region: us-east-1` as a dimension
           .merge_globals(globals),
   );
   ```

2. **Per-entry dimensions**: If not every record flushed to this sink uses the same dimensions _(note that libraries MAY also write to the global sink)_, you can set them when creating your metrics:

    ```rust
    use metrique::unit_of_work::metrics;

    #[metrics(
        emf::dimension_sets = [
            ["Status", "Operation"],
            ["Operation"]
        ],
    )]
    struct RequestMetrics {
        operation: &'static str,
        status: &'static str,
        number_of_ducks: usize,
    }
    ```

    If you flush `RequestMetrics` to the global sink you will get the following output:
    ```json
    {
    "_aws": {
        "CloudWatchMetrics": [
        {
            "Namespace": "MyApp",
            "Dimensions": [
            // NOTE: Entry-level dimension sets are cartesian joined
            // with the global dimensions:
            [
                "region",
                "Status",
                "Operation"
            ],
            [
                "region",
                "Operation"
            ]
            ],
            "Metrics": [
            {
                "Name": "NumberOfDucks"
            }
            ]
        }
        ],
        "Timestamp": 1744312627337
    },
    "NumberOfDucks": 9000,
    "region": "us-east-1",
    "Operation": "CountDucks",
    "Status": "Ok"
    }
    ```

### Setting Per-Entry Dimensions
Per entry dimensions can be set with the `emf::dimension_sets` attribute when declaring metrics.

**Note:**
1. Only the root metric may define dimension_sets. Failing to do this will result in an error at runtime. This behavior may be improved in the future.
2. There is no compile-time validation on the dimensions that are set. You must validate that that dimension sets are set as you expect. A good way to check this is by invoking the EMF formatter directly with `all_validations` enabled:

    ```rust
    use metrique_writer::format::Format;
    use metrique_writer_format_emf::Emf;
    use metrique::unit_of_work::metrics;

    #[metrics(
        emf::dimension_sets = [
            ["Status", "Operation"],
            ["Operation"]
        ],
    )]
    struct RequestMetrics {
        operation: &'static str,
        status: &'static str,
        number_of_ducks: usize,
    }

    #[test]
    fn test_metrics() {
        // Use all validations so that formatting produces a runtime error
        let mut emf = Emf::all_validations("MyApp".to_string(), vec![vec![]]);
        let mut output = vec![];


        emf.format(
            &RequestMetrics {
                operation: "operation",
                status: "status",
                number_of_ducks: 1000,
            }
            .close(),
            &mut output,
        )
        .unwrap();
    }
    ```

### Relationship Between Dimension Types
When combining global dimensions and entry-specific dimensions, the resulting dimension set is cartesian-joined, meaning for the following setup:
- Global: `[[region], [region, cell]]`
- Entry: `[[operation], [operation, status]]`

You will have the following dimension sets produced in your final record:
```not_rust
[region, operation]
[region, operation, status]
[region, cell, operation]
[region, cell, operation, status]
```

### Unset Dimensions
When validations are **enabled** (not recommended for production), any DimensionSet that does not contain a corresponding dimension will result
in the entire record being dropped. When validations are **disabled**, the record will still be sent to cloudwatch. In this case, any **dimension sets** that contain unset fields will be ignored.

## Histogram / Sampling Support
When either:
1. Flushing a histogram type (or other type that emits multiple `Obervations` in the same metric value),
2. Enabling sampling on the EMF Formatter using `with_sampling`.

`metrique` will emit data like this:
```json
"CacheLoadTime": {
  "Values": [
    1.5E-4
  ],
  "Counts": [
    100
  ],
  "Count": 1
},
```

This data will be properly handled by CloudWatch Metrics — however — if you are doing any queries that _manually_ read the data (e.g. Cloudwatch Logs Insights), you will need to parse the fields individually.

## Setting a Destination

Your choice of destination will depend on your deployment platform. In all cases, you'll want to decide whether you want to comingle logs and metrics or publish them to separate streams. There are pros and cons to each approach.
- When events are comingled, it is easy to see the context of relevant logs around a specific metric event.
- When events are _not_ comingled, things can be "cleaner" since you have one logstream that is the dedicated emission of unit-of-work metrics and a totally separate stream of tracing events.

Both approaches are used successfully in production.

If you decide to not comingle, you will typically either use two different files, a file and stdout, or a file and a socket to separate logs and metrics.

In all cases, your destination will be set by calling [`output_to`] on your format (in this case `Emf`).

There are three destinations typically used:

1. `std::io::stdout`
2. A file (typically with `RollingFileAppender`)

    ```rust,no_run
    use std::path::PathBuf;

    use metrique::ServiceMetrics;
    use metrique_writer::{
        FormatExt,
        sink::{AttachGlobalEntrySink, AttachHandle},
    };
    use metrique_writer_format_emf::Emf;
    use tracing_appender::rolling::{RollingFileAppender, Rotation};

    # let service_log_dir = std::path::PathBuf::default();
    let stream = Emf::all_validations("MyApp".into(), vec![vec![]])
        .output_to_makewriter(
                RollingFileAppender::new(Rotation::MINUTELY, &service_log_dir, "service_log.log")
        );
    ```
3. A TCP Stream
    ```rust,no_run
    use std::net::SocketAddr;
    use metrique_writer::FormatExt;
    use metrique_writer_format_emf::Emf;
    # async fn initialize_metrics() {
    let emf_port = 1234;
    let addr = SocketAddr::from(([127, 0, 0, 1], emf_port));
    // Use tokio to establish the socket to avoid blocking the runtime, then convert it to std
    let tcp_connection = tokio::net::TcpStream::connect(addr)
        .await
        .expect("failed to connect to Firelens TCP port")
        .into_std().unwrap();
    let stream =
        Emf::all_validations("QPersonalizationService".to_string(), vec![vec![]])
            .output_to(tcp_connection);
    # }
    ```


Your choice of destination will depend on your platform and performance needs. See specific guidance for [Fargate](#fargate--ecs), [Lambda](#lambda), and [EC2](#ec2).

**Important Note**: In all cases, you do not (and should not) use a nonblocking writer like [`tracing_appender::non_blocking`](https://docs.rs/tracing-appender/latest/tracing_appender/non_blocking/index.html) when configuring `metrique`. There is _already_ a background sink. By using a non-blocking writer, you're adding a second level of indirection that is both unnecessary and will consume more memory during failure.

## Platform Specific Guidance

### Fargate / ECS

On Fargate & ECS it is common to emit to `stdout` and collect logs with [`firelens`](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/using_firelens.html). If you do **not** want to comingle,
generally you will direct one of the streams (e.g. metrics) to a TCP destination instead. The simplest approach, however, is using the [awslogs driver].

If you have extremely high throughput, you may want to flush to disk and [use firelens/fluentbit and exec for file rotation](https://github.com/aws-samples/amazon-ecs-firelens-examples/tree/mainline/examples/fluent-bit/ecs-log-deletion).

To configure `metrique` to output to a file, use the `RollingFileAppender`:

```rust,no_run
use std::path::PathBuf;

use metrique_writer::{
    FormatExt,
    sink::{AttachGlobalEntrySink, AttachHandle},
    sink::global_entry_sink,
};
use metrique_writer_format_emf::Emf;

# let service_log_dir = std::path::PathBuf::default();

use tracing_appender::rolling::{RollingFileAppender, Rotation};

let stream = Emf::all_validations("MyApp".into(), vec![vec![]])
    .output_to_makewriter(
        RollingFileAppender::new(Rotation::MINUTELY, &service_log_dir, "service_log.log")
    );
```

### Lambda
On Lambda, you will generally output to `std::io::stdout()` and use the default lambda log driver that is already included.

### EC2

On EC2, the standard approach is to use [CloudWatch Agent (CWA)][Cloudwatch Agent]. The Cloudwatch Agent will handle deletion for you. You should configure output to a file using the `RollingFileAppender`.

```rust,no_run
use metrique_writer_format_emf::Emf;
use metrique_writer::FormatExt;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
# let service_log_dir = std::path::PathBuf::default();
let stream = Emf::all_validations("MyApp".into(), vec![vec![]])
    .output_to_makewriter(
        RollingFileAppender::new(Rotation::MINUTELY, &service_log_dir, "service_log.log")
    );
```

[awslogs driver]: https://docs.aws.amazon.com/AmazonECS/latest/developerguide/using_awslogs.html
[Cloudwatch Agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/Install-CloudWatch-Agent.html
[Cloudwatch Log Insights]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/AnalyzingLogData.html
[Cloudwatch Contributor Insights]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/ContributorInsights.html
[`Emf`]: https://docs.rs/metrique-writer-format-emf/0.1/metrique_writer_format_emf/struct.Emf.html
[`output_to`]: metrique_writer::FormatExt::output_to