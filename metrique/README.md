metrique is a crate to emit unit-of-work metrics

- [`#[metrics]` macro reference](https://docs.rs/metrique/0.1/metrique/unit_of_work/attr.metrics.html)

Unlike many popular metric frameworks that are based on the concept of your application having a fixed-ish set of counters and gauges, which are periodically updated to a central place, metrique is based on the concept of structured **metric records**. Your application emits a series of metric records - that are essentially structured log entries - to an observability service such as [Amazon CloudWatch], and the observability service allows you to view and alarm on complex aggregations of the metrics.

The log entries being structured means that you can easily use problem-specific aggregations to track down the cause of issues, rather than only observing the symptoms.

[Amazon CloudWatch]: https://docs.aws.amazon.com/AmazonCloudWatch

## Getting Started (Applications)

Most metrics your application records will be "unit of work" metrics. In a classic HTTP server, these are typically tied to the request/response scope.

You declare a struct that represents the metrics you plan to capture over the course of the request and annotate it with `#[metrics]`. That makes it possible to write it to a `Sink`. Rather than writing to the sink directly, you typically use `append_on_drop(sink)` to obtain a guard that will automatically write to the sink when dropped.

The simplest way to emit the entry is by emitting it to the [`ServiceMetrics`] global sink. That is a global
rendezvous point - you can attach a destination by using [`attach`] or [`attach_to_stream`], and then write to it
by using the [`sink`] method (you must attach a destination before calling [`sink`], otherwise you will encounter
a panic!).

If the global sink is not suitable, see
[sinks other than `ServiceMetrics`](#sinks-other-than-servicemetrics).

The example below will write the metrics to an [`tracing_appender::rolling::RollingFileAppender`]
in EMF format.

[`sink`]: metrique_writer::GlobalEntrySink::sink
[`attach`]: metrique_writer::AttachGlobalEntrySink::attach
[`attach_to_stream`]: metrique_writer::AttachGlobalEntrySinkExt::attach_to_stream

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

Enums can be used as entries with different fields per variant:

```rust
use metrique::unit_of_work::metrics;

// generally entry enums will be used as subfields,
// though they can also be root containers
#[metrics(subfield_owned)]
enum RequestResult {
    Success {
        response_size: usize,
        cache_hit: bool,
    },
    Error {
        error_code: String,
    },
}

#[metrics]
struct RequestMetrics {
    operation: &'static str,
    request_id: String,
    success: bool,
    #[metrics(flatten)]
    result: RequestResult,
}
```

Entry enums handle container and field-level attributes like structs. See the [macro documentation](https://docs.rs/metrique/latest/metrique/unit_of_work/attr.metrics.html#enums) for details.

### Timing Events

`metrique` provides several timing primitives to simplify measuring time. They are all mockable via
[`metrique_timesource`]:

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
to be filled - either by waiting for your subtask to complete or by using `Slot::wait_for_data`.

Another option is to use techniques for [controlling the point of metric emission](#controlling-the-point-of-metric-emission) - to make that easy, `Slot::open` has a `OnParentDrop::Wait` mode, that holds on to a `FlushGuard` until the slot is closed.

```rust
use metrique::writer::GlobalEntrySink;
use metrique::unit_of_work::metrics;
use metrique::{ServiceMetrics, SlotGuard, Slot, OnParentDrop};

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
use metrique::writer::GlobalEntrySink;
use metrique::unit_of_work::metrics;
use metrique::{Counter, ServiceMetrics};

use std::sync::Arc;

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

### Using sampling to deal with too-many-metrics

Generally, metrique is fast enough to preserve everything as a full event. But this isn't always possible. Before you reach for client side aggregation, consider sampling: You can use the built in support for [sampling](https://docs.rs/metrique/0.1.5/metrique/emf/struct.Emf.html#method.with_sampling)

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
> not do what you expect. It is critical that you add [tests](#testing-emitted-metrics) that validate that
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

Ordinary fields in metrics need to implement [`CloseValue`]`<Output: `[`metrique_writer::Value`]`>`.

If you use a formatter (`#[metrics(format)]`), your field needs to implement [`CloseValue`],
and its output needs to be supported by the [formatter](#custom-valueformatters) instead of
implementing [`metrique_writer::Value`].

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

[`ValueFormatter`]: metrique_writer::value::ValueFormatter
[`ValueWriter`]: metrique_writer::ValueWriter

## Destinations

`metrique` metrics are normally written via a [`BackgroundQueue`], which performs
the formatting and I/O in a background thread. `metrique` supports writing to the
following destinations:

1. Via [`output_to_makewriter`] to a [`tracing_subscriber::fmt::MakeWriter`], for example a
   [`tracing_appender::rolling::RollingFileAppender`] that writes the metric
   to a rotating file with a rotation period.
2. Via [`output_to`] to a [`std::io::Write`], for example to standard output or a
   network socket, often used for sending EMF logs to a local metric agent process.
3. To an in-memory [`TestEntrySink`] for tests (see [Testing](#testing)).

You can find examples setting up EMF uploading in the [EMF docs](crate::emf).

[`BackgroundQueue`]: crate::writer::sink::BackgroundQueue
[`TestEntrySink`]: crate::writer::test_util::TestEntrySink
[`output_to_makewriter`]: crate::writer::FormatExt::output_to_makewriter
[`output_to`]: crate::writer::FormatExt::output_to

### Sink types

#### Background Queue

The default [`BackgroundQueue`](crate::writer::sink::BackgroundQueue) implementation buffers entries
in memory and writes them to the output stream in a background thread. This is ideal for high-throughput
applications where you want to minimize the impact of metric writing on your application's performance.

Background queues are normally set up by using `ServiceMetrics::attach_to_stream`:

```rust
use metrique::emf::Emf;
use metrique::ServiceMetrics;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};

let handle = ServiceMetrics::attach_to_stream(
    Emf::builder("Ns".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

# use metrique::unit_of_work::metrics;
# #[metrics]
# struct MyEntry {}
# MyEntry {}.append_on_drop(ServiceMetrics::sink());
```

#### Immediate Flushing for ephemeral environments

For simpler use cases, especially in environments like AWS Lambda where background threads are not
ideal, you can use the [`FlushImmediately`](crate::writer::sink::FlushImmediately) implementation.

```rust
use metrique::emf::Emf;
use metrique::ServiceMetrics;
use metrique::writer::{AttachGlobalEntrySink, FormatExt, GlobalEntrySink};
use metrique::writer::sink::FlushImmediately;
use metrique::unit_of_work::metrics;

#[metrics]
struct MyMetrics {
    value: u64,
}

fn main() {
    let sink = FlushImmediately::new_boxed(
        Emf::no_validations(
            "MyNS".to_string(),
            vec![vec![/*your dimensions here */]],
        )
        .output_to(std::io::stdout()),
    );
    let _handle = ServiceMetrics::attach((sink, ()));
    handle_request();
}

fn handle_request() {
    let mut metrics = MyMetrics { value: 0 }.append_on_drop(ServiceMetrics::sink());
    metrics.value += 1;
    // request will be flushed immediately here, as the request is dropped
}
```

Note that `FlushImmediately` will block while writing each entry, so it's not suitable for
latency-sensitive or high-throughput applications.

### Sinks other than `ServiceMetrics`

In most applications, it is the easiest to emit metrics to the global [`ServiceMetrics`] sink,
which is a global variable that serves as a rendezvous point between the part of the code that
generates metrics (which calls [`sink`]) and the code that writes them
to a destination (which calls [`attach_to_stream`] or [`attach`]).

If use of this global is not desirable, you can
[create a locally-defined global sink](#creating-a-locally-defined-global-sink) or
[use EntrySink directly](#creating-a-non-global-sink). When using `EntrySink` directly,
it is possible, but not mandatory, to use a slightly-faster non-`dyn` API.

#### Creating a locally-defined global sink

You can create a different global sink by using the [`global_entry_sink`] macro. That will create a new
global sink that behaves exactly like, but is distinct from, [`ServiceMetrics`]. This is normally
useful when some of your metrics need to go to a separate destination than the others.

For example:

```rust
use metrique::emf::Emf;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique::writer::sink::global_entry_sink;
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
struct MyEntry {
    value: u32
}

global_entry_sink! { MyServiceMetrics }

let handle = MyServiceMetrics::attach_to_stream(
    Emf::builder("Ns".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

let metric = MyEntry::default().append_on_drop(MyServiceMetrics::sink());
```

#### Creating a specifically-typed non-global sink

If you are not using a global sink, you can also create a sink that is specific to
your entry type. While the global sink API, which uses [`BoxEntrySink`] and dynamic dispatch,
is plenty fast for most purposes, using a fixed entry type avoids virtual dispatch which
improves performance in *very*-high-throughput cases.

To use this API, create a sink for `RootMetric<MyEntry>`, for example a
`BackgroundQueue<RootMetric<MyEntry>>`. Of course, you can use sink types
other than `BackgroundQueue`, like
[`FlushImmediately`](#immediate-flushing-for-ephemeral-environments).

For example:

```rust
use metrique::{CloseValue, RootMetric};
use metrique::emf::Emf;
use metrique::writer::{EntrySink, FormatExt};
use metrique::writer::sink::BackgroundQueue;
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
struct MyEntry {
    value: u32
}

type MyRootEntry = RootMetric<MyEntry>;

let (queue, handle) = BackgroundQueue::<MyRootEntry>::new(
    Emf::builder("Ns".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

handle_request(&queue);

fn handle_request(queue: &BackgroundQueue<MyRootEntry>) {
    let mut metric = MyEntry::default();
    metric.value += 1;
    // or you can `metric.append_on_drop(queue.clone())`, but that clones an `Arc`
    // which has slightly negative performance impact
    queue.append(MyRootEntry::new(metric.close()));
}
```

[`global_entry_sink`]: crate::writer::sink::global_entry_sink
[`BackgroundQueue::new`]: crate::writer::sink::BackgroundQueue::new
[`BoxEntrySink`]: crate::writer::BoxEntrySink

#### Creating a boxing non-global sink

[`BoxEntrySink`] can be used without the global sink API, to create a non-global
sink that accepts arbitrary entry types using the same amount of boxing and dynamic
dispatch as a global sink.

Example:

```rust
use metrique::{CloseValue, RootEntry};
use metrique::emf::Emf;
use metrique::writer::{AnyEntrySink, BoxEntrySink, EntrySink, FormatExt};
use metrique::writer::sink::BackgroundQueueBuilder;
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
struct MyEntry {
    value: u32
}

let (queue, handle) = BackgroundQueueBuilder::new().build_boxed(
    Emf::builder("Ns".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

handle_request(&queue);

fn handle_request(queue: &BoxEntrySink) {
    let mut metric = MyEntry::default();
    metric.value += 1;
    // or you can `metric.append_on_drop(queue.clone())`, but that clones an `Arc`
    // which has slightly negative performance impact
    queue.append(RootEntry::new(metric.close()));
}
```

## Sampling

High-volume services may want to trade lower accuracy for lower CPU time spent on metric emission. Offloading metrics to
CloudWatch can become bottlenecked if the agent isn't able to keep up with the rate of written metric entries.

It is common to tee the metric into 2 destinations:

 1. A highly-compressed "log of record" that contains all entries and is eventually persisted to S3 or other long-term storage.
 1. An uncompressed, but sampled, metrics log that is published to CloudWatch.

The sampling can be done naively at some [fixed fraction](`writer::sample::FixedFractionSample`), but at low rates can
cause low-frequency events to be missed. This includes service errors or validation errors, especially when the service is
designed to have an availability much higher than the chosen sample rate. Instead, we recommend the use of the
[congressional sampler](`writer::sample::CongressSample`). It uses a fixed metric emisssion target rate and
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

[`with_sampling`]: emf::Emf::with_sampling

## Testing

### Testing emitted metrics

`metrique` provides `test_entry` which allows introspecting the entries that are emitted (without needing to read EMF directly). You can use this functionality in combination with the `TestEntrySink` to test that you are emitting the metrics that you expect:

> Note: enable the `test-util` feature of `metrique` to enable test utility features.

```rust
# #[allow(clippy::test_attr_in_doctest)]

use metrique::unit_of_work::metrics;

use metrique::test_util::{self, TestEntrySink};

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
