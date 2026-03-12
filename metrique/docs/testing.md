# Testing and Debugging

## Testing emitted metrics

### Quick assertions with `test_metric`

For simple tests where you just want to verify field values without setting up a sink, [`test_metric`](crate::test_util::test_metric) closes a metric struct and returns a [`TestEntry`](crate::test_util::TestEntry) you can assert against directly:

```rust,ignore
use metrique::test_util::test_metric;

let entry = test_metric(RequestMetrics { operation: "SayHello", number_of_ducks: 10 });
assert_eq!(entry.metrics["NumberOfDucks"], 10);
```

For tests that need to verify the full emit pipeline, use `test_entry_sink` below.

### Testing with `test_entry_sink`

`metrique` provides `test_entry_sink` which allows introspecting the entries that are emitted (without needing to read EMF directly). You can use this functionality in combination with the [`TestEntrySink`](crate::test_util::TestEntrySink) to test that you are emitting the metrics that you expect:

> Note: enable the `test-util` feature of `metrique` to enable test utility features.

```rust,no_run

use metrique::unit_of_work::metrics;

use metrique::test_util::{self, TestEntrySink};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    number_of_ducks: usize
}

#[test]
fn test_metrics () {
    let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
    let metrics = RequestMetrics {
        operation: "SayHello",
        number_of_ducks: 10
    }.append_on_drop(sink);

    // In a real application, you would run some API calls, etc.

    let entries = inspector.entries();
    assert_eq!(entries[0].values["Operation"], "SayHello");
    assert_eq!(entries[0].metrics["NumberOfDucks"], 10);
}
```

There are two ways to control the queue:
1. Pass the queue explicitly when constructing your metric object, e.g. by passing it into `init` (as done above)
2. Use the test-queue functionality provided out-of-the-box by global entry queues:
```rust
use metrique::writer::GlobalEntrySink;
use metrique::ServiceMetrics;
use metrique::test_util::{self, TestEntrySink};

let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
let _guard = ServiceMetrics::set_test_sink(sink);
```

See `examples/testing.rs` and `examples/testing-global-queues.rs` for more detailed examples.

## Debugging common issues

### Human-readable output with `LocalFormat`

[`LocalFormat`](crate::local::LocalFormat) renders metric entries in a readable
format (pretty, JSON, or markdown table) instead of EMF. Swap it in during local
development to see what your code is emitting:

```rust,no_run
use metrique::ServiceMetrics;
use metrique::local::{LocalFormat, OutputStyle};
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};

let _handle = ServiceMetrics::attach_to_stream(
    LocalFormat::new(OutputStyle::Pretty)
        .output_to(std::io::stderr()),
);
```

Example output:

```text
---
  TotalTime: 302.457ms
  Success: 1
  Failure: 0
  SegmentIndex: 180
  UncompressedSize: 20.97MB
  CompressedSize: 7.97MB
  InvalidFileHeader: 1
  Gzip.Time: 113.549ms
  Gzip.Success: 1
  Gzip.Failure: 0
  S3Upload.Time: 188.904ms
  S3Upload.Success: 1
  S3Upload.Failure: 0
```

### No entries in the log

If you see empty files e.g. "service_log.{date}.log", this could be because your entries are invalid and being dropped by `metrique-writer`. This will occur if your entry is invalid (e.g. if you have two fields with the same name). Enable tracing logs to see the errors.

```rust
# #[allow(clippy::needless_doctest_main)]
fn main() {
    tracing_subscriber::fmt::init();
}
```
