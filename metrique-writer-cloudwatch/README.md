A `metrique` [EntryIoStream] backend that submits metrics directly to
[Amazon CloudWatch Logs][cw-logs] via `PutLogEvents` using the
[Embedded Metric Format (EMF)][emf-docs].

This provides a direct path to CloudWatch Metrics without requiring the
[CloudWatch Agent][cw-agent] or any log routing infrastructure.

For more details, read the docs for [CwLogsStream].

## Setup

```rust,ignore
use metrique_writer_cloudwatch::CwLogsStream;
use metrique::ServiceMetrics;
use metrique::writer::{AttachGlobalEntrySinkExt, GlobalEntrySink};

let sdk_config = aws_config::load_from_env().await;
let client = aws_sdk_cloudwatchlogs::Client::new(&sdk_config);

let (stream, handle) = CwLogsStream::builder()
    .client(client)
    .log_group_name("/my-app/metrics".to_string())
    .log_stream_name("host-1".to_string())
    .namespace("MyApp".to_string())
    .build();

let _attach = ServiceMetrics::attach_to_stream(stream);

// Metrics emitted via ServiceMetrics are now published to CloudWatch.

// On shutdown:
handle.shutdown().await;
```

## Runtime Feature

The `tokio_runtime` feature is enabled by default and provides the default
Tokio task spawner. With `--no-default-features`, `CwLogsStream::builder()`
requires an explicit `task_spawner`.

Custom spawners are responsible for detaching the submitted future, or otherwise
keeping it running after the spawn callback returns. Shutdown completion is
reported through `CwLogsStreamHandle::shutdown()`, not through a task join
handle.

For non-Tokio executors, configure the AWS client with a compatible
[`AsyncSleep`][aws-smithy-async-sleep]. SDK timeouts and retry delays use that
sleep hook.

[cw-logs]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/WhatIsCloudWatchLogs.html
[cw-agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/Install-CloudWatch-Agent.html
[emf-docs]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html
[EntryIoStream]: https://docs.rs/metrique-writer-core/latest/metrique_writer_core/stream/trait.EntryIoStream.html
[CwLogsStream]: https://docs.rs/metrique-writer-cloudwatch/latest/metrique_writer_cloudwatch/struct.CwLogsStream.html
[aws-smithy-async-sleep]: https://docs.rs/aws-smithy-async/latest/aws_smithy_async/rt/sleep/trait.AsyncSleep.html
