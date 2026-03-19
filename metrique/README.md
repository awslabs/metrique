metrique is a crate to emit unit-of-work metrics

- [`#[metrics]` macro reference](https://docs.rs/metrique/0.1/metrique/unit_of_work/attr.metrics.html)

Unlike many popular metric frameworks that are based on the concept of your application having a fixed-ish set of counters and gauges, which are periodically updated to a central place, metrique is based on the concept of structured **metric records**. Your application emits a series of metric records - that are essentially structured log entries - to an observability service such as [Amazon CloudWatch], and the observability service allows you to view and alarm on complex aggregations of the metrics.

The log entries being structured means that you can easily use problem-specific aggregations to track down the cause of issues, rather than only observing the symptoms.

[Amazon CloudWatch]: https://docs.aws.amazon.com/AmazonCloudWatch

## Further reading

- [`_guide::cookbook`] - principles for effective instrumentation and choosing the right pattern
- [`_guide::concurrency`] - flush guards, slots, atomics, and shared handles for concurrent metrics
- [`_guide::sinks`] - destinations, sink types, and alternatives to `ServiceMetrics`
- [`_guide::sampling`] - congressional sampling and the tee pattern for high-volume services
- [`_guide::testing`] - test utilities and debugging common issues

[`_guide::cookbook`]: https://docs.rs/metrique/latest/metrique/_guide/cookbook/
[`_guide::concurrency`]: https://docs.rs/metrique/latest/metrique/_guide/concurrency/
[`_guide::sinks`]: https://docs.rs/metrique/latest/metrique/_guide/sinks/
[`_guide::sampling`]: https://docs.rs/metrique/latest/metrique/_guide/sampling/
[`_guide::testing`]: https://docs.rs/metrique/latest/metrique/_guide/testing/

## Getting Started (Applications)

Most metrics your application records will be "unit of work" metrics. In a classic HTTP server, these are typically tied to the request/response scope.

You declare a struct that represents the metrics you plan to capture over the course of the request and annotate it with `#[metrics]`. That makes it possible to write it to a `Sink`. Rather than writing to the sink directly, you typically use `append_on_drop(sink)` to obtain a guard that will automatically write to the sink when dropped.

The simplest way to emit the entry is by emitting it to the [`ServiceMetrics`] global sink. That is a global
rendezvous point - you can attach a destination by using [`attach`] or [`attach_to_stream`], and then write to it
by using the [`sink`] method (you must attach a destination before calling [`sink`], otherwise you will encounter
a panic!).

If the global sink is not suitable, see
[sinks other than `ServiceMetrics`](https://docs.rs/metrique/latest/metrique/_guide/sinks/#sinks-other-than-servicemetrics).

The example below will write the metrics to a `tracing_appender::rolling::RollingFileAppender`
in EMF format.

[`sink`]: https://docs.rs/metrique/latest/metrique/writer/trait.GlobalEntrySink.html#method.sink
[`attach`]: https://docs.rs/metrique/latest/metrique/writer/trait.AttachGlobalEntrySink.html#method.attach
[`attach_to_stream`]: https://docs.rs/metrique/latest/metrique/writer/trait.AttachGlobalEntrySinkExt.html#method.attach_to_stream

```rust,no_run
use std::path::PathBuf;

use metrique::unit_of_work::metrics;
use metrique::timers::{Timestamp, Timer};
use metrique::unit::Millisecond;
use metrique::ServiceMetrics;
use metrique::writer::GlobalEntrySink;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, sink::AttachHandle};
use metrique::emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

// Define operation as an enum (you can also define operation as a &'static str).
// Enums containing fields are also supported - see <#entry-enums>
#[metrics(value(string))]
#[derive(Copy, Clone)]
enum Operation {
    CountDucks,
}

// define our metrics struct
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: Operation,
    #[metrics(timestamp)]
    timestamp: Timestamp,
    number_of_ducks: usize,
    #[metrics(unit = Millisecond)]
    operation_time: Timer,
}

impl RequestMetrics {
    // It is generally a good practice to expose a single initializer that sets up
    // append on drop.
    fn init(operation: Operation) -> RequestMetricsGuard {
        RequestMetrics {
            timestamp: Timestamp::now(),
            operation,
            number_of_ducks: 0,
            operation_time: Timer::start_now(),
        }.append_on_drop(ServiceMetrics::sink())
    }
}

async fn count_ducks() {
    let mut metrics = RequestMetrics::init(Operation::CountDucks);
    metrics.number_of_ducks = 5;
    // metrics flushes as scope drops
    // timer records the total time until scope exits
}

fn initialize_metrics(service_log_dir: PathBuf) -> AttachHandle {
    // `metrique::ServiceMetrics` is a single global metric sink
    // defined by `metrique` that can be used by your application.
    //
    // If you want to have more than 1 stream of metrics in your
    // application (for example, to have separate streams of
    // metrics for your application's control and data planes),
    // you can define your own global entry sink (which will
    // behave exactly like `ServiceMetrics`) by using the
    // `metrique::writer::sink::global_entry_sink!` macro.
    //
    // See the examples in metrique/examples for that.

    // attach an EMF-formatted rolling file appender to `ServiceMetrics`
    // which will write the metrics asynchronously.
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

#[tokio::main]
async fn main() {
    // not strictly needed, but metrique will emit tracing errors
    // when entries are invalid and it's best to be able to see them.
    tracing_subscriber::fmt::init();
    let _join = initialize_metrics("my/metrics/dir".into());
    // ...
    // call count_ducks
    // for example
    count_ducks().await;
}

#[cfg(test)]
mod test {
    #[tokio::test]
    async fn my_metrics_are_emitted() {
        let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
        let _guard = crate::ServiceMetrics::set_test_sink(sink);
        super::count_ducks().await;
        let entry = inspector.get(0);
        assert_eq!(entry.metrics["NumberOfDucks"], 5);
    }
}
```

That code will create a single metric line (your timestamp and `OperationTime` may vary).

```json
{"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"NumberOfDucks"},{"Name":"OperationTime","Unit":"Milliseconds"}]}],"Timestamp":1752774958378},"NumberOfDucks":5,"OperationTime":0.003024,"Operation":"CountDucks"}
```

## Getting Started (Libraries)

Library operations should normally return a struct implementing `CloseEntry` that contains the metrics for their operation. Generally, the best way of getting that is by just using the `#[metrics]` macro:

```rust
use metrique::instrument::Instrumented;
use metrique::timers::Timer;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use std::io;

#[derive(Default)]
#[metrics(subfield)]
struct MyLibraryOperation {
    #[metrics(unit = Millisecond)]
    my_library_operation_time: Timer,
    my_library_count_of_ducks: usize,
}

async fn my_operation() -> Instrumented<Result<usize, io::Error>, MyLibraryOperation> {
    Instrumented::instrument_async(MyLibraryOperation::default(), async |metrics| {
        let count_of_ducks = 1;
        metrics.my_library_count_of_ducks = count_of_ducks;
        Ok(count_of_ducks)
    }).await
}
```

Note that we do not use `rename_all` - the application should be able to choose the naming style.

Read [docs/usage_in_libraries.md][usage-in-libs] for more details

[usage-in-libs]: https://github.com/awslabs/metrique/blob/main/metrique/docs/usage_in_libraries.md

## Common Patterns

For more complex examples, see the [examples folder].

[examples folder]: https://github.com/awslabs/metrique/tree/main/metrique/examples

### Entry Enums

Enums can be used as entries with different fields per variant. See the [macro documentation](https://docs.rs/metrique/latest/metrique/unit_of_work/attr.metrics.html#enums) for details. 

Entry enums handle container and field-level attributes like structs. You can optionally include a "tag" field that contains the variant name.

```rust
use metrique::unit_of_work::metrics;

// generally entry enums will be used as subfields,
// though they can also be root containers
#[metrics(tag(name = "operation"), subfield)]
enum Operation {
    MeetDogs { dogs_met: usize },
    FindGoose { goose_found: bool },
    CountCats(#[metrics(flatten)] CatMetrics),
}

#[metrics(subfield)]
struct CatMetrics {
    cats_counted: usize,
}

#[metrics]
struct RequestMetrics {
    request_id: String,
    success: bool,
    #[metrics(flatten)]
    operation: Operation,
}
```

When `RequestMetrics` with `Operation::MeetDogs { dogs_met: 3 }` is emitted, the output includes:
- `operation` (string value): `"MeetDogs"`
- `dogs_met` (metric): `3`

When `RequestMetrics` with `Operation::FindGoose { goose_found: true }` is emitted, the output includes:
- `operation` (string value): `"FindGoose"`
- `goose_found` (metric): `1` (booleans emit as 0 or 1)

When `RequestMetrics` with `Operation::CountCats(CatMetrics { cats_counted: 7 })` is emitted, the output includes:
- `operation` (string value): `"CountCats"`
- `cats_counted` (metric): `7`


### Timing Events

`metrique` provides several timing primitives to simplify measuring time. They are all mockable via
[`metrique_timesource`]: https://docs.rs/metrique-timesource/latest/metrique_timesource/

 * [`Timer`] / [`Stopwatch`]: Reports a [`Duration`] using the [`Instant`] time-source. It can either be a
   [`Timer`] (in which case it starts as soon as it is created), or a [`Stopwatch`] (in which case you must
   start it manually). In all cases, if you don't stop it manually, it will drop when the record containing
   it is closed.
 * [`Timestamp`]: records a timestamp using the [`SystemTime`] time-source. When used with
   `#[metrics(timestamp)]`, it will be written as the canonical timestamp field for whatever format
   is in use. Otherwise, it will report its value as a string property containing the duration
   since the Unix Epoch.

   You can control the formatting of a `Timestamp` (that is not used
   as a `#[metrics(timestamp)]` - the formatting of the canonical timestamp
   is controlled solely by the formatter) by setting
   `#[metrics(format = ...)]` to one of [`EpochSeconds`], [`EpochMillis`]
   (the default), or [`EpochMicros`].
 * [`TimestampOnClose`]: records the timestamp when the record is closed.

Usage example:

```rust
use metrique::timers::{Timestamp, TimestampOnClose, Timer, Stopwatch};
use metrique::unit::Millisecond;
use metrique::timers::EpochSeconds;
use metrique::unit_of_work::metrics;
use std::time::Duration;

#[metrics]
struct TimerExample {
    // record a timestamp when the record is created (the name
    // of the field doesn't affect the generated metrics)
    //
    // If you don't provide a timestamp, most formats will use the
    // timestamp of when your record is formatted (read your
    // formatter's docs for the exact details).
    //
    // Multiple `#[metrics(timestamp)]` will cause a validation error, so
    // normally only the top-level metric should have a
    // `#[metrics(timestamp)]` field.
    #[metrics(timestamp)]
    timestamp: Timestamp,

    // some other timestamp - not emitted if `None` since it's optional.
    //
    // formatted as seconds from epoch.
    #[metrics(format = EpochSeconds)]
    some_other_timestamp: Option<Timestamp>,

    // records the total time the record is open for
    time: Timer,

    // manually record the duration of a specific event
    subevent: Stopwatch,

    // typically, you won't have durations directly since you'll use
    // timing primitives instead. However, note that `Duration` works
    // just fine as a metric type:
    #[metrics(unit = Millisecond)]
    manual_duration: Duration,

    #[metrics(format = EpochSeconds)]
    end_timestamp: TimestampOnClose,
}
```

[`Instant`]: std::time::Instant
[`Duration`]: std::time::Duration
[`Timer`]: timers::Timer
[`Stopwatch`]: timers::Stopwatch
[`Timestamp`]: timers::Timestamp
[`TimestampOnClose`]: timers::TimestampOnClose
[`SystemTime`]: std::time::SystemTime
[`EpochSeconds`]: timers::EpochSeconds
[`EpochMillis`]: timers::EpochMillis
[`EpochMicros`]: timers::EpochMicros

### Returning Metrics from Subcomponents

`#[metrics]` are composable. There are two main patterns for subcomponents
recording their own metrics. You can define sub-metrics by having a
`#[metrics(subfield)]`. Then, you can either return a metric struct along with
the data - `metrique` provides `Instrument` to standardize this - or pass a
(mutable) reference to the metrics struct. See [the library metrics example](#getting-started-libraries).

This is the recommended approach. It has minimal performance overhead and makes your metrics very predictable.

### Metrics with complex lifetimes

Sometimes, managing metrics with a simple ownership and mutable reference pattern does not work well -
for example when spawning background tasks or fanning out work in parallel. `metrique` provides flush
guards, [`Slot`]s, atomics, and shared handles to cover these cases.

See [`_guide::concurrency`](https://docs.rs/metrique/latest/metrique/_guide/concurrency/) for details and examples.

### Using sampling to deal with too-many-metrics

Generally, metrique is fast enough to preserve everything as a full event. But this isn't always possible. Before you reach for client side aggregation, consider [sampling](https://docs.rs/metrique/latest/metrique/_guide/sampling/).

## Controlling metric output

### Setting units for metrics

You can provide units for your metrics. These will be included in the output format. You can find all available units in `metrique::unit::*`. Note that these are an open set and the custom units may be defined.

```rust
use metrique::unit_of_work::metrics;
use metrique::unit::Megabyte;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,

    #[metrics(unit = Megabyte)]
    request_size: usize
}
```

### Renaming metric fields

> the complex interaction between naming, prefixing, and inflection is deterministic, but sometimes might
> not do what you expect. It is critical that you add [tests](https://docs.rs/metrique/latest/metrique/_guide/testing/) that validate that
> the keys being produced match your expectations

You can customize how metric field names appear in the output using several approaches:

#### Rename all fields with a consistent case style

Use the `rename_all` attribute on the struct to apply a consistent naming convention to all fields:

```rust
use metrique::unit_of_work::metrics;

// All fields will use kebab-case in the output
#[metrics(rename_all = "kebab-case")]
struct RequestMetrics {
    // Will appear as "operation-name" in metrics output
    operation_name: &'static str,
    // Will appear as "request-size" in metrics output
    request_size: usize
}
```

Supported case styles include: `"PascalCase"`, `"camelCase"`, `"snake_case"`.

**Important:** `rename_all` is transitive—it will apply to all child structures that are `#[metrics(flatten)]`'d into the entry. **You SHOULD only set `rename_all` on your root struct.** If a struct explicitly sets a name scheme with `rename_all`, it will not be overridden by a parent.

#### Add a prefix to all fields

Use the `prefix` attribute on structs to add a consistent prefix to all fields:

```rust
use metrique::unit_of_work::metrics;

// All fields will be prefixed with "api_"
#[metrics(rename_all = "PascalCase", prefix = "api_")]
struct ApiMetrics {
    // Will appear as "ApiLatency" in metrics output
    latency: usize,
    // Will appear as "ApiErrors" in metrics output
    errors: usize
}
```

#### Add a prefix to all metrics in a subfield

Use the `prefix` attribute on `flatten` to add a consistent prefix to fields of the
included struct:

```rust
use metrique::unit_of_work::metrics;

use std::collections::HashMap;

#[metrics(subfield)]
struct DownstreamMetrics {
    // our downstream calls their metric just "success", so we don't know who succeedded
    success: bool,
}

// using `subfield_owned` to allow closing over the `HashMap`
#[metrics(subfield_owned)]
struct OtherDownstreamMetrics {
    // the prefix will be *SKIPPED* within this field, since it is included using `flatten_entry`
    //
    // the prefix is skipped since prepending a prefix would require allocating a new String,
    // and metrique will rather not have code that does that.
    #[metrics(flatten_entry, no_close)]
    prefix_skipped: HashMap<String, u32>,
    // another downstream that calls their metric just "success", so we don't know who succeedded
    success: bool,
}

#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
    // This is our success field, will appear as "Success" in metrics output
    success: bool,
    // Their success field will appear as "DownstreamSuccess" in metrics output
    #[metrics(flatten, prefix="Downstream")]
    downstream: DownstreamMetrics,
    #[metrics(flatten, prefix="OtherDownstream")]
    other_downstream: OtherDownstreamMetrics,
}
```

Prefixes will be inflected to the case metrics are emitted in, so if you let `rename_all`
vary, the inner metric name will be:

 1. in `rename_all = "Preserve"`, `Downstreamsuccess` / `OtherDownstreamsuccess`
 2. in `rename_all = "PascalCase"`, `DownstreamSuccess` / `OtherDownstreamSuccess`
 3. in `rename_all = "kebab-case"`, `downstream-success` / `other-downstream-success`
 4. in `rename_all = "snake_case"`, `downstream_success` / `other_downstream_success`

#### Rename individual fields

Use the `name` attribute on individual fields to override their names:

```rust
use metrique::unit_of_work::metrics;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    // Will appear as "CustomOperationName" in metrics output
    #[metrics(name = "CustomOperationName")]
    operation: &'static str,

    request_size: usize
}
```

#### Combining renaming strategies

You can combine these approaches, with field-level renames taking precedence over container-level rules:

```rust
use metrique::unit_of_work::metrics;

#[metrics(rename_all = "kebab-case")]
struct Metrics {
    // Will appear as "foo-bar" in metrics output
    foo_bar: usize,

    // Will appear as "custom_name" in metrics output (not kebab-cased)
    #[metrics(name = "custom_name")]
    overridden_field: &'static str,

    // Nested metrics can have their own renaming rules
    #[metrics(flatten, prefix="his-")]
    nested: PrefixedMetrics,
}

#[metrics(rename_all = "PascalCase", prefix = "api_")]
struct PrefixedMetrics {
    // Will appear as "his-ApiLatency" in metrics output (explicit rename_all overrides the parent)
    latency: usize,

    // Will appear as "his-exact_name" in metrics output (overrides both struct prefix and case, but not external prefix)
    #[metrics(name = "exact_name")]
    response_time: usize,
}
```

## Types in metrics

Example of a metrics struct:

```rust
use metrique::{Counter, Slot};
use metrique::timers::{EpochSeconds, Timer, Timestamp, TimestampOnClose};
use metrique::unit::{Byte, Second};
use metrique::unit_of_work::metrics;
use metrique::writer::value::ToString;

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[metrics(subfield)]
struct NestedMetrics {
    nested_metric: f64,
}

#[metrics]
struct MyMetrics {
    integer_value: u32,

    floating_point_value: f64,

    // emitted as f64 with unit of bytes
    #[metrics(unit = Byte)]
    floating_point_value_bytes: f64,

    // emitted as 0 if false, 1 if true
    boolean: bool,

    // emitted as a Duration (default is as milliseconds)
    duration: Duration,

    // emitted as a Duration in seconds
    #[metrics(unit = Second)]
    duration_seconds: Duration,

    // timer, emitted as a duration
    timer: Timer,

    // optional value - emitted only if present
    optional: Option<u64>,

    // use of Formatter
    #[metrics(format = EpochSeconds)]
    end_timestamp: TimestampOnClose,

    // use of Formatter behind Option
    #[metrics(format = EpochSeconds)]
    end_timestamp_opt: Option<Timestamp>,

    // you can also have values that are atomics
    counter: Counter,
    // or behind an Arc
    counter_behind_arc: Arc<Counter>,

    // or Slots
    #[metrics(unit = Byte)]
    value_behind_slot: Slot<f64>,

    // or just values that are behind an Arc<Mutex>
    #[metrics(unit = Byte)]
    value_behind_arc_mutex: Arc<Mutex<f64>>,

    // ..and also an Option
    #[metrics(unit = Byte)]
    value_behind_opt_arc_mutex: Arc<Mutex<Option<f64>>>,

    // You can format values that implement Display as strings
    //
    // Since IpAddr doesn't implement CloseValue, but rather `Display` directly,
    // you'll need `no_close`.
    //
    // It is also possible to define your own custom formatters. Consult the documentation
    // for `ValueFormatter` for more info.
    #[metrics(format = ToString, no_close)]
    source_ip_addr: IpAddr,

    // you can have nested subfields
    #[metrics(flatten)]
    nested: NestedMetrics,
}
```

Ordinary fields in metrics need to implement [`CloseValue`]`<Output: `[`metrique_writer::Value`](https://docs.rs/metrique/latest/metrique/writer/trait.Value.html)`>`.

If you use a formatter (`#[metrics(format)]`), your field needs to implement [`CloseValue`],
and its output needs to be supported by the [formatter](#custom-valueformatters) instead of
implementing [`metrique_writer::Value`](https://docs.rs/metrique/latest/metrique/writer/trait.Value.html).

Nested fields (`#[metrics(flatten)]`) need to implement [`CloseEntry`].

## Customization

If the standard primitives in `metrique` don't serve your needs, there's a good
chance you might be able to implement them yourself.

### Custom [`CloseValue`] and [`CloseValueRef`]

If you want to change the behavior when metrics are closed, you can
implement [`CloseValue`] or [`CloseValueRef`] yourself ([`CloseValueRef`]
does not take ownership and will also also work behind smart pointers,
for example for `Arc<YourValue>`).

For instance, here is an example for adding a custom timer type that calculates the time from when it was created, to when it finished, on close (it doesn't do anything that `timers::Timer` doesn't do, but is useful as an example).

```rust
use metrique::{CloseValue, CloseValueRef};
use std::time::{Duration, Instant};

struct MyTimer(Instant);
impl Default for MyTimer {
    fn default() -> Self {
        Self(Instant::now())
    }
}

// this does not take ownership, and therefore should implement `CloseValue` for both &T and T
impl CloseValue for &'_ MyTimer {
    type Closed = Duration;

    fn close(self) -> Self::Closed {
        self.0.elapsed()
    }
}

impl CloseValue for MyTimer {
    type Closed = Duration;

    fn close(self) -> Self::Closed {
        self.close_ref() /* this proxies to the by-ref implementation */
    }
}
```

[`CloseValue`]: https://docs.rs/metrique/0.1/metrique/trait.CloseValue.html
[`CloseValueRef`]: https://docs.rs/metrique/0.1/metrique/trait.CloseValueRef.html

### Custom [`ValueFormatter`]s

You can implement custom formatters by creating a custom value formatter using the [`ValueFormatter`] trait that formats the value into a [`ValueWriter`], then referring to it using `#[metrics(format)]`.

An example use would look like the following:

```rust
use metrique::unit_of_work::metrics;

use std::time::SystemTime;
use chrono::{DateTime, Utc};

/// Format a SystemTime as UTC time
struct AsUtcDate;

// observe that `format_value` is a static method, so `AsUtcDate`
// is never initialized.

impl metrique::writer::value::ValueFormatter<SystemTime> for AsUtcDate {
    fn format_value(writer: impl metrique::writer::ValueWriter, value: &SystemTime) {
        let datetime: DateTime<Utc> = (*value).into();
        writer.string(&datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
}

#[metrics]
struct MyMetric {
    #[metrics(format = AsUtcDate)]
    my_field: SystemTime,
}
```

[`ValueFormatter`]: https://docs.rs/metrique/latest/metrique/writer/value/trait.ValueFormatter.html
[`ValueWriter`]: https://docs.rs/metrique/latest/metrique/writer/trait.ValueWriter.html

## Destinations

`metrique` metrics are normally written via a background queue to a file, stdout, or a network socket.
The global [`ServiceMetrics`] sink is the easiest way to get started, but you can also create
locally-defined global sinks or use `EntrySink` directly for non-global or specifically-typed sinks.

See [`_guide::sinks`](https://docs.rs/metrique/latest/metrique/_guide/sinks/) for details on sink types, destinations,
and alternatives to `ServiceMetrics`.

## Sampling

High-volume services may want to sample metrics to reduce CPU and agent load. `metrique` supports
fixed-fraction sampling and a congressional sampler that preserves rare events. A common pattern is
to tee metrics into an archived log of record and a sampled stream for CloudWatch.

See [`_guide::sampling`](https://docs.rs/metrique/latest/metrique/_guide/sampling/) for details and a full example.

## Testing

`metrique` provides test utilities for introspecting emitted entries without reading EMF directly.
Use `TestEntrySink` to capture entries and assert on their values and metrics.

See [`_guide::testing`](https://docs.rs/metrique/latest/metrique/_guide/testing/) for details, examples, and
debugging tips.

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

In that case, you should not be using `BackgroundQueue` or sampling. It is probably fine to use the `Format` implementations in that case, but it is recommended to test and audit your use-case to make sure nothing is being missed.

### Use of exporters

The `metrique` library does not currently contain any code that exports the metrics outside of the current process. To make a working system, you normally need to integrate the `metrique` library with some exporter such as the [Amazon CloudWatch Agent].

It is your responsibility to ensure that any agents you are using are kept up to date and configured in a secure manner.

[Amazon CloudWatch Agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Generation_CloudWatch_Agent.html
