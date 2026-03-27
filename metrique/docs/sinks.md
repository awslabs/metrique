# Sinks and Destinations

## Destinations

`metrique` metrics are normally written via a [`BackgroundQueue`], which performs
the formatting and I/O in a background thread. `metrique` supports writing to the
following destinations:

1. Via [`output_to_makewriter`] to a `tracing_subscriber::fmt::MakeWriter`, for example a
   `tracing_appender::rolling::RollingFileAppender` that writes the metric
   to a rotating file with a rotation period.
2. Via [`output_to`] to a [`std::io::Write`], for example to standard output or a
   network socket, often used for sending EMF logs to a local metric agent process.
3. To an in-memory [`TestEntrySink`] for tests (see [`testing`]).
4. To [`DevNullSink`] to suppress all output (for instance, to conditionally disable metrics at runtime via an environment variable).

You can find examples setting up EMF uploading in the [EMF docs].

[`BackgroundQueue`]: https://docs.rs/metrique/latest/metrique/writer/sink/struct.BackgroundQueue.html
[`DevNullSink`]: https://docs.rs/metrique/latest/metrique/writer/sink/struct.DevNullSink.html
[`TestEntrySink`]: https://docs.rs/metrique/latest/metrique/test_util/struct.TestEntrySink.html
[`output_to_makewriter`]: https://docs.rs/metrique/latest/metrique/writer/trait.FormatExt.html#method.output_to_makewriter
[`output_to`]: https://docs.rs/metrique/latest/metrique/writer/trait.FormatExt.html#method.output_to
[`std::io::Write`]: https://doc.rust-lang.org/std/io/trait.Write.html

## Sink types

### Background Queue

The default [`BackgroundQueue`] implementation buffers entries
in memory and writes them to the output stream in a background thread. This is ideal for high-throughput
applications where you want to minimize the impact of metric writing on your application's performance.

Background queues are normally set up by using [`ServiceMetrics::attach_to_stream`],
which creates a [`BackgroundQueue`], connects it to the global
[`ServiceMetrics`] sink, and returns a handle that keeps the background thread alive.
If you need a separate global sink (for example, to write some metrics to a different destination),
see [`global_entry_sink`](#creating-a-locally-defined-global-sink).

```rust,ignore
use metrique::emf::Emf;
use metrique::ServiceMetrics;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};

let handle = ServiceMetrics::attach_to_stream(
    Emf::builder("Ns".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);
```

### Immediate Flushing for ephemeral environments

In environments like AWS Lambda, a background queue provides less benefit due to low concurrency
and requires a custom graceful shutdown to drain. For these cases, consider using the
[`FlushImmediately`] implementation.

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

Note that [`FlushImmediately`] will block while writing each entry, so it's not suitable for
latency-sensitive or high-throughput applications.

## Sinks other than `ServiceMetrics`

In most applications, it is the easiest to emit metrics to the global [`ServiceMetrics`] sink,
which is a global variable that serves as a rendezvous point between the part of the code that
generates metrics (which calls [`sink`]) and the code that chooses the destination
(which calls [`attach_to_stream`] or [`attach`]).

If use of this global is not desirable, you can
[create a locally-defined global sink](#creating-a-locally-defined-global-sink) or
[use EntrySink directly](#creating-a-non-global-sink). When using [`EntrySink`] directly,
it is possible, but not mandatory, to use a slightly-faster non-`dyn` API. This requires that all entries going into the sink are the same type, which is not a requirement with global sinks.

### Creating a locally-defined global sink

You can create a different global sink by using the [`global_entry_sink`] macro. That will create a new
global sink that behaves exactly like, but is distinct from, [`ServiceMetrics`]. This is normally
useful when some of your metrics need to go to a separate destination than the others.

For example:

```rust,ignore
use metrique::emf::Emf;
use metrique::ServiceMetrics;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique::writer::sink::global_entry_sink;
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
struct MyEntry {
    value: u32
}

global_entry_sink! { AuditMetrics }

// Primary metrics go to the "MyService" namespace
let _primary = ServiceMetrics::attach_to_stream(
    Emf::builder("MyService".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

// Audit metrics go to a separate "MyService/Audit" namespace
let _audit = AuditMetrics::attach_to_stream(
    Emf::builder("MyService/Audit".to_string(), vec![vec![]])
        .build()
        .output_to(std::io::stdout())
);

let metric = MyEntry::default().append_on_drop(AuditMetrics::sink());
```

### Creating a specifically-typed non-global sink

If you are not using a global sink, you can also create a sink that is specific to
your entry type. While the global sink API, which uses [`BoxEntrySink`] and dynamic dispatch,
is plenty fast for most purposes, using a fixed entry type avoids virtual dispatch which
improves performance in *very*-high-throughput cases. It can also reduce overhead when your metric struct has a large number of fields.

To use this API, create a sink for [`RootMetric<MyEntry>`], the wrapper type that adds
metadata (like timestamps) to your entry. For example, a
`BackgroundQueue<RootMetric<MyEntry>>`. Of course, you can use sink types
other than [`BackgroundQueue`], like
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

[`global_entry_sink`]: https://docs.rs/metrique/latest/metrique/writer/sink/macro.global_entry_sink.html
[`BackgroundQueue::new`]: https://docs.rs/metrique/latest/metrique/writer/sink/struct.BackgroundQueue.html#method.new
[`BoxEntrySink`]: https://docs.rs/metrique/latest/metrique/writer/struct.BoxEntrySink.html
[`BACKGROUND_QUEUE_METRICS`]: https://docs.rs/metrique/latest/metrique/writer/sink/constant.BACKGROUND_QUEUE_METRICS.html

## Metrics being dropped

The `metrique` library is intended to be used for operational metrics, and therefore it is intentionally designed to drop metrics under high-load conditions rather than having the application grind to a halt.

There are 2 places where this can happen:

1. [`BackgroundQueue`] will drop the oldest entry in the queue under load (see [`BACKGROUND_QUEUE_METRICS`] for the overflow counter and other queue diagnostics).
2. It is possible to explicitly enable sampling (by using
   [`sample_by_fixed_fraction`] or [`sample_by_congress_at_fixed_entries_per_second`]).
   If sampling is being used, metrics will be dropped at random.

If your application's security relies on metric entries not being dropped (for example,
if you use metric entries to track user log-in operations, and your application relies on log-in operations not being dropped), it is your responsibility to engineer your application to avoid the metrics being dropped.

In that case, you should not be using [`BackgroundQueue`] or sampling. It is probably fine to use the [`Format`] implementations in that case, but it is recommended to test and audit your use-case to make sure nothing is being missed.

## Metric source integrations

Global entry sinks support subscribing background metric sources that
automatically append entries at a configured interval. The subscription
is tied to the [`AttachHandle`] — when it drops, the background task is
aborted.

Available integrations (via [`metrique-util`]):

- **Tokio runtime metrics** — enable the `tokio-metrics-bridge` feature on
  `metrique-util` and call [`subscribe_tokio_runtime_metrics`] to
  start appending [`RuntimeMetrics`] snapshots (worker
  utilization, queue depths, poll durations, and more).

[`AttachHandle`]: https://docs.rs/metrique-writer/latest/metrique_writer/sink/struct.AttachHandle.html
[`metrique-util`]: https://docs.rs/metrique-util/latest/metrique_util/
[`subscribe_tokio_runtime_metrics`]: https://docs.rs/metrique-util/latest/metrique_util/trait.AttachGlobalEntrySinkTokioMetricsExt.html#method.subscribe_tokio_runtime_metrics
[`RuntimeMetrics`]: https://docs.rs/tokio-metrics/latest/tokio_metrics/struct.RuntimeMetrics.html

## Use of exporters

The `metrique` library does not currently contain any code that exports the metrics outside of the current process. To make a working system, you normally need to integrate the `metrique` library with some exporter such as the [Amazon CloudWatch Agent].

It is your responsibility to ensure that any agents you are using are kept up to date and configured in a secure manner.

[Amazon CloudWatch Agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Generation_CloudWatch_Agent.html

[`attach_to_stream`]: https://docs.rs/metrique/latest/metrique/writer/trait.AttachGlobalEntrySinkExt.html#method.attach_to_stream
[`attach`]: https://docs.rs/metrique/latest/metrique/writer/trait.AttachGlobalEntrySink.html#method.attach
[`EntrySink`]: https://docs.rs/metrique/latest/metrique/writer/trait.EntrySink.html
[`FlushImmediately`]: https://docs.rs/metrique/latest/metrique/writer/sink/struct.FlushImmediately.html
[`Format`]: https://docs.rs/metrique/latest/metrique/writer/format/trait.Format.html
[`RootMetric<MyEntry>`]: https://docs.rs/metrique/latest/metrique/type.RootMetric.html
[`sample_by_congress_at_fixed_entries_per_second`]: https://docs.rs/metrique/latest/metrique/writer/sample/trait.SampledFormatExt.html#method.sample_by_congress_at_fixed_entries_per_second
[`sample_by_fixed_fraction`]: https://docs.rs/metrique/latest/metrique/writer/sample/trait.SampledFormatExt.html#method.sample_by_fixed_fraction
[`ServiceMetrics::attach_to_stream`]: https://docs.rs/metrique/latest/metrique/writer/trait.AttachGlobalEntrySinkExt.html#method.attach_to_stream
[`ServiceMetrics`]: https://docs.rs/metrique/latest/metrique/struct.ServiceMetrics.html
[`sink`]: https://docs.rs/metrique/latest/metrique/writer/trait.GlobalEntrySink.html#method.sink
[`testing`]: https://docs.rs/metrique/latest/metrique/_guide/testing/
[EMF docs]: https://docs.rs/metrique/latest/metrique/emf/
