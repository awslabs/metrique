# Metrique [![Build Status]][actions] [![Latest Version]][crates.io] [![Released API docs]][docs.rs] [![Apache-2.0 licensed]][license]

[Build Status]: https://github.com/awslabs/metrique/actions/workflows/build.yml/badge.svg
[actions]: https://github.com/awslabs/metrique/actions?query=workflow%3Abuild
[Latest Version]: https://img.shields.io/crates/v/metrique.svg
[crates.io]: https://crates.io/crates/metrique
[Released API docs]: https://docs.rs/metrique/badge.svg
[docs.rs]: https://docs.rs/metrique
[Apache-2.0 licensed]: https://img.shields.io/badge/license-Apache_2.0-blue.svg
[license]: ./LICENSE

**Metrique is a set of crates for collecting and exporting *unit-of-work* metrics.**

```rust
use metrique::unit_of_work::metrics;

#[metrics]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: Timestamp,
    number_of_ducks: usize,
    #[metrics(unit = Millisecond)]
    operation_time: Timer,
}
```

This currently supports exporting metrics in [Amazon EMF] format to CloudWatch.

More formats might be supported in future versions. You can also configure a custom
format using the [`Format`] trait. If you do, you can optionally implement a custom
[`EntrySink`] if you need flush functionality beyond writing bytes to an arbitrary I/O destination.

## Why Metrique?

Metrique is designed for high-performance, structured metrics collection with minimal runtime overhead. Metrique is built around the principle that a metric associated with a specific action is more valuable than those that are only available aggregated over time. We call these metrics "unit-of-work" metrics because they correspond to a single unit of application work.

### Performance
Unlike metrics libraries that collect metrics in a `HashMap`, `metrique` uses plain structs. This eliminates allocation and hashmap lookups when producing metrics, resulting in significantly lower CPU overhead and memory pressure. This is especially important for high-throughput services.

Compared to libraries that rely on `HashMap`s or similar containers, the overhead of `metrique` can be 50x lower!

### Structured Metrics with Type Safety
Because metrique builds on plain structs, metric structure is enforced at compile time. Your metrics are defined as structs with the `#[metrics]` attribute, ensuring consistency and catching errors early rather than at runtime. Structuring your metrics up front has some up front cost but it pays for itself in the long term.

### Minimal Allocation Overhead
`metrique-writer`, the serialization library for `metrique`, enables low (and sometimes 0) allocation formatting for EMF. Coupled with the fact that metrics-are-just-structs, this can significantly reduce allocator pressure.

### Why use `metrique`?

#### Instead of [OpenTelemetry](https://opentelemetry.io/)
OTel and metrique solve different problems and future work may add an OTel backend for metrique. `metrique` is about emitting events that capture all the metrics associated with single unit of work, in Rust, as efficiently as possible.

#### Instead of [metrics.rs](https://metrics.rs/)
`metrique` is actually compatible with `metrics.rs` via the [`metrique-metricsrs`](https://crates.io/crates/metrique-metricsrs) crate! This allows you to periodically
flush the contents of metrics collected via libraries already compatible with `metrics.rs` as a single event.

However, if your goal is to emit structured events that produce metrics with as little overhead as possible:
- Metrique avoids `HashMap`-based metric storage, reducing allocation pressure and the overhead of recording metrics
- Compile-time metric definition prevents typos and makes it obvious exactly what metrics your application produces

## Getting Started

Most applications and libraries will use [`metrique`](metrique) directly and configure a writer with [`metrique-writer`](metrique-writer). See the [examples](metrique/examples) for several examples of different common patterns.

Applications will define a metrics struct that they annotate with `#[metrics]`:
```rust
use metrique::unit_of_work::metrics;

#[metrics(value(string))]
enum Operation {
     CountDucks,
}

#[metrics]
struct RequestMetrics {
    operation: Operation, // you can use `operation: &'static str` if you prefer
    #[metrics(timestamp)]
    timestamp: Timestamp,
    number_of_ducks: usize,
    #[metrics(unit = Millisecond)]
    operation_time: Timer,
}
```

On its own, this is just a normal struct, there is no magic. To use it as a metric, you can call `.append_on_drop`:
```rust
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
```

The `guard` object can still be mutated via `DerefMut` impl:
```rust
async fn count_ducks() {
    let mut metrics = RequestMetrics::init(Operation::CountDucks);
    metrics.number_of_ducks = 5;
    // metrics flushes as scope drops
    // timer records the total time until scope exits
}
```

But when it drops, it will be appended to the queue to be formatted and flushed.

To control how it is written, when you start your application, you must configure a queue:
```rust
pub use metrique::ServiceMetrics;

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
```

> See [`metrique-writer`](metrique-writer) for more information about queues and destinations.

You can either attach it to a global destination or thread the queue to the location you construct your metrics object directly. Currently, only formatters for [Amazon EMF] are provided, but more may be added in the future.
You can also configure a custom format using the [`Format`] trait. If you do, you can optionally implement a custom
[`EntrySink`] if you need flush functionality beyond writing bytes to an arbitrary I/O destination.

## Aggregation

When you have many observations of the same metric within a single unit of work, you can use histograms to aggregate them into a distribution rather than emitting each observation individually.

The [`metrique-aggregation`](metrique-aggregation) crate provides histogram types that collect observations and emit them as distributions:

```rust
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::{Histogram, ExponentialAggregationStrategy};
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct QueryMetrics {
    query_id: String,
    
    #[metrics(unit = Millisecond)]
    backend_latency: Histogram<Duration, ExponentialAggregationStrategy>,
}
```

Common use cases include:
- Distributed queries that fan out to multiple backend services
- Batch processing where you want to track per-item latency
- Any operation that generates multiple measurements to aggregate

For most applications, [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md) is a better approach than aggregation. Consider histograms when you need precise distributions for high-frequency events.

## Glossary

 - **dimension**: The keys for metrics are generally of the form `(name, dimensions)`. Metric
   backends have ways of aggregating metrics according to some sets of dimensions.

   For example, a metric named `RequestCount` can be emitted with dimensions
   `[(Status, <http status>), (Operation, <operation>)]`. Then, the metric backend could allow
   for counting the requests with status 500 for operation `Frobnicate`.
 - **entry io stream**: An object that implements [`EntryIoStream`] - should be wrapped into
   an [`EntrySink`] before use - see the [`EntryIoStream`] docs for more details.
 - **entry sink**: An object that implements [`EntrySink`], that normally writes entries as
   metric records to some entry destination outside the program. Normally a [`BackgroundQueue`]
   or a [`FlushImmediately`].
 - **guard**: a Rust object that performs some action on drop. In a metrique context, normally an
   [`AppendAndCloseOnDrop`] that emits a metric entry when dropped.
 - **metric**: A *metric* is a `(name, dimensions)` key that can have values associated with
   it. Generally, a metric contains **metric datapoint**s.
 - **metric backend**: The backend being used to aggregate metrics. `metrique` currently
   comes with support for the [Amazon EMF] backend, but support can be added to other
   backends.
 - **metric datapoint**: A single point of `(name, dimensions, multiplicity, time, value)`,
   generally not represented explicitly but rather being emitted from fields in a
   *metric entry*. Metric datapoints have a value that is an integer or floating point, and can
   come with some sort of *multiplicity*.
 - **metric entry**: something that implements [`Entry`] (when using `metrique` rather
   than using `metrique-writer` directly, this will be a [`RootEntry`] wrapping an
   [`InflectableEntry`]). Will create a metric record (e.g., an EMF
   JSON entry) when emitted.
 - **metric record**: the data recorded created from emitting a metric entry and sent
   to the metric backend. Will create metric datapoints for the included metrics
 - **multiplicity**: Is a property of a metric value, that allows it to count as a large number
   of datapoints with `O(1)` emission complexity. `metrique` allows users to emit metric datapoint
   with multiplicity.
 - **property**: In addition to *metric datapoints*, *metric entries* can also contain string-valued
   properties, that are normally not automatically aggregated directly by the metric backend, but can
   be used as keys for aggregations - for example, it is sometimes useful to include the
   host machine and software version as properties.
 - **slot**: A [`Slot`], which can be used in `metrique` to write to a part of a metric entry from a
   different task or thread. A [`Slot`] can also hold a reference to a [`FlushGuard`] that can delay
   metric entry emission until the [`Slot`] is finalized.

[`AppendAndCloseOnDrop`]: https://docs.rs/metrique/0.1/metrique/struct.AppendAndCloseOnDrop.html
[`BackgroundQueue`]: https://docs.rs/metrique-writer/0.1/metrique_writer/sink/struct.BackgroundQueue.html
[`Entry`]: https://docs.rs/metrique-writer/0.1/metrique_writer/trait.Entry.html
[`EntryIoStream`]: https://docs.rs/metrique-writer/0.1/metrique_writer/trait.EntryIoStream.html
[`EntrySink`]: https://docs.rs/metrique-writer/0.1/metrique_writer/trait.EntrySink.html
[`Format`]: https://docs.rs/metrique/0.1/metrique/writer/format/trait.Format.html
[`FlushGuard`]: https://docs.rs/metrique/0.1/metrique/slot/struct.FlushGuard.html
[`FlushImmediately`]: https://docs.rs/metrique-writer/0.1/metrique_writer/sink/struct.FlushImmediately.html
[`InflectableEntry`]: https://docs.rs/metrique/0.1/metrique/trait.InflectableEntry.html
[`RootEntry`]: https://docs.rs/metrique/0.1/metrique/struct.RootEntry.html
[`Slot`]: https://docs.rs/metrique/0.1/metrique/slot/struct.Slot.html

## Security

See [CONTRIBUTING](CONTRIBUTING.md#security-issue-notifications) for more information.

[Amazon EMF]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html


## License

This project is licensed under the Apache-2.0 License.
