metrique is a crate to emit unit-of-work metrics

- [`#[metrics]` macro reference](https://docs.rs/metrique/0.1/metrique/unit_of_work/attr.metrics.html)

Unlike many popular metric frameworks that are based on the concept of your application having a fixed-ish set of counters and gauges, which are periodically updated to a central place, metrique is based on the concept of structured **metric records**. Your application emits a series of metric records - that are essentially structured log entries - to an observability service such as [Amazon CloudWatch], and the observability service allows you to view and alarm on complex aggregations of the metrics.

The log entries being structured means that you can easily use problem-specific aggregations to track down the cause of issues, rather than only observing the symptoms.

[Amazon CloudWatch]: https://docs.aws.amazon.com/AmazonCloudWatch

## Getting Started (Applications)

Most metrics your application records will be "unit of work" metrics. In a classic HTTP server, these are typically tied to the request/response scope.

You declare a struct that represents the metrics you plan to capture over the course of the request and annotate it with `#[metrics]`. That makes it possible to write it to a `Sink`. Rather than writing to the sink directly, you typically use `append_on_drop(sink)` to obtain a guard that will automatically write to the sink when dropped.

The simplest way to emit the entry is by emitting it to a global entry sink, defined by using the [`metrique_writer::sink::global_entry_sink`] macro. That will create a global rendezvous point - you can attach a destination by using [`attach`] or [`attach_to_stream`], and then write to it by using the [`sink`] method (you must attach a destination before calling [`sink`], otherwise you will encounter a panic!).

The example below will write the metrics to an [`tracing_appender::rolling::RollingFileAppender`]
in EMF format.

[`sink`]: metrique_writer::GlobalEntrySink::sink
[`attach`]: metrique_writer::AttachGlobalEntrySink::attach
[`attach_to_stream`]: metrique_writer::AttachGlobalEntrySinkExt::attach_to_stream

```rust
use std::path::PathBuf;

use metrique::unit_of_work::metrics;
use metrique::timers::{Timestamp, Timer};
use metrique::unit::Millisecond;
use metrique_writer::{GlobalEntrySink, sink::global_entry_sink};
use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, sink::AttachHandle};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

// define our global entry sink
global_entry_sink! { ServiceMetrics }

// define operation as an enum (you can also define operation as a &'static str)
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

### Timing Events

`metrique` provides several timing primitives to simplify measuring time. They are all mockable via
[`metrique-timesource`]:

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

Sometimes, managing metrics with a simple ownership and mutable reference pattern does not work well. The
`metrique` crate provides some tools to help more complex situations

#### Controlling the point of metric emission

Sometimes, your code does not have a single exit point at which you want to report your metrics = maybe
your operation spawns some post-processing tasks, and you want your metric entry to include information
from all of them.

You don't want to wrap your parent metric in an `Arc`, as that will prevent you from having mutable access
to metric fields, but you still want to delay metric emission.

To allow for that, the [`AppendAndCloseOnDrop`] guard (which is what the `<MetricName>Guard` aliases point to)
has `flush_guard` and `force_flush_guard` functions. The flush guards are type-erased (they have
types `FlushGuard` and `ForceFlushGuard`, which don't mention the type of the metric entry).

The metric will then be emitted when either:

1. The owner handle of the metric and *all* the `FlushGuard`s have been dropped
2. The owner handle of the metric and *any* of the `ForceFlushGuard`s have been dropped.

This makes `force_flush_guard` useful to emit a metric via a timeout even if some
of the downstream tasks have not completed, which is useful since you normally
want metrics even (maybe *especially*) when things are stuck (the downstream tasks
presumably have access to the metric struct via an [`Arc`](#using-atomics)
or [`Slot`](#using-slots-to-send-values), which if they eventually finish,
will let them safely write a value to the now-dead metric).

See the examples below to see how the flush guards are used.

#### Using `Slot`s to send values

In some cases, you might want a sub-task (potentially a Tokio task, but maybe just a sub-component of your code)
to be able to add some metric fields to your metric entry, but without forcing an ownership relationship.

In that case, you can use `Slot`, which creates a oneshot channel, over which the value of the metric can be sent.

Note that `Slot` by itself does not delay the parent metric entry's emission in any way. If your metric entry
is emitted (for example, when your request is finished) before the slot is filled, the metric entry will just
skip the metrics behind the `Slot`. One option is to make your request wait for the slot
to be filled - either by [`join`]ing your subtask or by using `Slot::wait_for_data`.

Another option is to use techniques for [controlling the point of metric emission](#controlling-the-point-of-metric-emission) - to make that easy, `Slot::open` has a `OnParentDrop::Wait` mode, that holds on to a `FlushGuard` until the slot is closed.

```rust
use metrique_writer::{GlobalEntrySink, sink::global_entry_sink};
use metrique::unit_of_work::metrics;
use metrique::{SlotGuard, Slot, OnParentDrop};

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,

    // When using a nested field, you must explicitly flatten the fields into the root
    // metric and explicitly `close` it to collect results.
    #[metrics(flatten)]
    downstream_operation: Slot<DownstreamMetrics>
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            operation,
            downstream_operation: Default::default()
        }.append_on_drop(ServiceMetrics::sink())
    }
}

// sub-fields can also be declared with `#[metrics]`
#[metrics(subfield)]
#[derive(Default)]
struct DownstreamMetrics {
    number_of_ducks: usize
}

async fn handle_request_discard() {
    let mut metrics = RequestMetrics::init("DoSomething");
    let downstream_metrics = metrics.downstream_operation.open(OnParentDrop::Discard).unwrap();

    // NOTE: if `downstream_metrics` is not dropped before `metrics` (the parent object),
    // no data associated with `downstream_metrics` will be emitted
    tokio::task::spawn(async move {
        call_downstream_service(downstream_metrics)
    });

    // If you want to ensure you don't drop data from a slot if background is still in-flight, you can wait explicitly:
    metrics.downstream_operation.wait_for_data().await;
}

async fn handle_request_on_parent_wait() {
    let mut metrics = RequestMetrics::init("DoSomething");
    let guard = metrics.flush_guard();
    let downstream_metrics = metrics.downstream_operation.open(OnParentDrop::Wait(guard)).unwrap();

    // NOTE: if `downstream_metrics` is not dropped before `metrics` (the parent object),
    // no data associated with `downstream_metrics` will be emitted
    tokio::task::spawn(async move {
        call_downstream_service(downstream_metrics)
    });

    // The metric will be emitted when the downstream service finishes
}


async fn call_downstream_service(mut metrics: SlotGuard<DownstreamMetrics>) {
    // can mutate the struct directly w/o using atomics.
    metrics.number_of_ducks += 1
}
```

#### Using Atomics

You might want to "fan out" work to multiple scopes that are in the background or otherwise operating in parallel. You can
accomplish this by using atomic field types to store the metrics, and fanout-friendly wrapper APIs on your metrics entry.

Anything that implements `CloseValue` can be used as a field. `metrique` provides a number of basic primitives such as `Counter`, a thin wrapper around `AtomicU64`. Most `std::sync::atomic` types also implement `CloseValueRef` directly. If you need to build your own primitives, simply ensure they implement `CloseValueRef`. By using primitives that can be mutated through shared references, you make it possible to use `Handle` or your own `Arc` to share the metrics entry around multiple owners or tasks.

For further usage of atomics for concurrent metric updates, see [the fanout example][unit-of-work-fanout].

```rust
use metrique_writer::{GlobalEntrySink, sink::global_entry_sink};
use metrique::unit_of_work::metrics;
use metrique::Counter;

use std::sync::Arc;

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    number_of_concurrent_ducks: Counter
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            operation,
            number_of_concurrent_ducks: Default::default()
        }.append_on_drop(ServiceMetrics::sink())
    }
}

fn count_concurrent_ducks() {
    let mut metrics = RequestMetrics::init("CountDucks");

    // convenience function to wrap `entry` in an `Arc`. This makes a cloneable metrics handle.
    let handle = metrics.handle();
    for i in 0..10 {
        let handle = handle.clone();
        std::thread::spawn(move || {
            handle.number_of_concurrent_ducks.add(i);
        });
    }
    // Each handle is keeping the metric entry alive!
    // The metric will not be flushed until all handles are dropped!
    // TODO: add an API to spawn a task that will force-flush the entry after a timeout.
}
```

[unit-of-work-fanout]: https://github.com/awslabs/metrique/blob/main/metrique/examples/unit-of-work-fanout.rs

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

Use the `prefix` attribute to add a consistent prefix to all fields:

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

You can combine these approaches, with field-level renames taking precedence over struct-level rules:

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
    #[metrics(flatten)]
    nested: PrefixedMetrics,
}

#[metrics(rename_all = "PascalCase", prefix = "api_")]
struct PrefixedMetrics {
    // Will appear as "ApiLatency" in metrics output (explicit rename_all overrides the parent)
    latency: usize,

    // Will appear as "exact_name" in metrics output (overrides both prefix and case)
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

    // you can have nested subfields
    #[metrics(flatten)]
    nested: NestedMetrics,
}
```

Ordinary fields in metrics need to implement [`CloseValue`]`<Output: `[`metric_writer::Value`]`>`.

If you use a formatter (`#[metrics(format)]`), your field needs to implement [`CloseValue`],
and its output needs to be supported by the [formatter](#custom-valueformatters) instead of
implementing [`metric_writer::Value`].

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

[`CloseValue`]: metrique::CloseValue
[`CloseValueRef`]: metrique::CloseValueRef

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

impl metrique_writer::value::ValueFormatter<SystemTime> for AsUtcDate {
    fn format_value(writer: impl metrique_writer::ValueWriter, value: &SystemTime) {
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

[`ValueFormatter`]: metrique_writer::value::ValueFormatter
[`ValueWriter`]: metrique_writer::ValueWriter

## Testing

### Testing emitted metrics

`metrique` provides `test_entry` which allows introspecting the entries that are emitted (without needing to read EMF directly). You can use this functionality in combination with the `TestEntrySink` to test that you are emitting the metrics that you expect:

```rust
# #[allow(clippy::test_attr_in_doctest)]

use metrique::unit_of_work::metrics;

use metrique_writer::test_util::{self, TestEntrySink};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    number_of_ducks: usize
}

#[test]
# fn test_in_doctests_is_a_lie() {}
fn test_metrics () {
    let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
    let metrics = RequestMetrics {
        operation: "SayHello",
        number_of_ducks: 10
    }.append_on_drop(sink);

    // In a real application, you would run some API calls, etc.

    let entries = inspector.entries();
    assert_eq!(entries[0].values["Operation"], "SayHello");
    assert_eq!(entries[0].metrics["NumberOfDucks"].as_u64(), 10);
}
```

## Debugging common issues

### No entries in the log

If you see empty files e.g. "service_log.{date}.log", this is could be because your entries are invalid and being dropped by `metrique-writer`. This will occur if your entry is invalid (e.g. if you have two fields with the same name). Enable tracing logs to see the errors.

```rust
# #[allow(clippy::needless_doctest_main)]
fn main() {
    tracing_subscriber::fmt::init();
}
```

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