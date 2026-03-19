# Principles and Patterns

This guide covers the principles behind effective metrics instrumentation and
helps you choose the right pattern for your use case.

## Principles

### Principle 1: Unit-of-work metrics provide more value when debugging

Do not aggregate client-side unless necessary. When metrics are aggregated client
side, critical debugging information is lost. For example, you cannot tell whether
two fields spiked concurrently or whether they were both high at unrelated points
during your aggregation window. Record metrics directly associated with a unit of
work and let your metrics backend perform aggregation.

**Unit-of-work metrics** (API response time, request size, request ID) let you
correlate individual records to debug *why* something happened. **Time-based
metrics** (CPU usage, tokio task count, disk usage) show behavior over time but
cannot explain causation.

A production application typically needs both. Unit-of-work metrics are the primary
focus of `metrique`; see [periodic metrics](#periodic-metrics-gauges-counters-metadata) for the time-based
case.

### Principle 2: Treat metrics as a critical component of your application

Having every metric defined in a single struct (or a small set of structs) rather
than scattered throughout the codebase yields significant benefits:

- **Discoverability**: new team members see every metric at a glance
- **Code review**: metric changes are visible in one place
- **Testing**: straightforward to assert on exact metrics emitted
- **Consistency**: naming conventions and units enforced by the struct definition

This is the approach `metrique` is designed around - metrics are plain structs,
defined up front, with compile-time enforcement.

## Choosing the right pattern

| Pattern | When to use | Trade-off |
|---------|-------------|-----------|
| [Unit-of-work](#unit-of-work) | Clear unit of work (request, job, event) | Full context per record |
| [Sampled unit-of-work](#sampled-unit-of-work) | Unit-of-work metrics at high volume where full emission is too expensive | Loses some records; rare events preserved by congressional sampler |
| [Aggregated](#aggregated) | High-frequency events where individual records are too expensive | Loses per-record context; consider combining with sampling |
| [Shared state in unit-of-work](#shared-state-in-unit-of-work) | Global counters, config, or gauges that benefit from request correlation | Richer metadata per record; prefer over standalone periodic metrics |
| [Periodic (gauges, counters, metadata)](#periodic-metrics-gauges-counters-metadata) | Dedicated time series, or lightweight standalone emission | Point-in-time only; loses request correlation |

### Unit-of-work

The most common pattern. Each request, job, or event gets its own metric record
with full context for debugging.

See the [Getting Started](https://docs.rs/metrique/latest/metrique/#getting-started-applications) section and the
[unit-of-work-simple](https://github.com/awslabs/metrique/blob/main/metrique/examples/unit-of-work-simple.rs)
example.

### Sampled unit-of-work

When you want unit-of-work metrics but full emission is too expensive, sample
the stream. The [congressional sampler](https://docs.rs/metrique/latest/metrique/writer/sample/struct.CongressSample.html)
gives rare events (errors, unusual operations) a higher sampling rate so they
aren't lost. A common setup is to tee into an archived log of record (all entries)
and a sampled stream for CloudWatch.

See [`_guide::sampling`](https://docs.rs/metrique/latest/metrique/_guide/sampling/) for details and
a full example.

### Aggregated

When individual records are too expensive for your throughput, aggregate
while preserving distributions via histograms. The threshold depends on your
infrastructure and metric backend; profile to find the right balance.
Consider using [`tee()`](https://docs.rs/metrique/latest/metrique/writer/stream/fn.tee.html) to combine an
aggregated stream for dashboards with a
[sampled](https://docs.rs/metrique/latest/metrique/_guide/sampling/) stream of raw records for debugging.

Two flavors:

- **Embedded**: aggregate sub-operations within a single unit of work. See the
  [embedded example](https://github.com/awslabs/metrique/blob/main/metrique-aggregation/examples/embedded.rs).
- **Sink-level**: aggregate across units of work. See the
  [sink_level example](https://github.com/awslabs/metrique/blob/main/metrique-aggregation/examples/sink_level.rs).

See [`metrique-aggregation`](https://docs.rs/metrique-aggregation) for full details.

### Shared state in unit-of-work

Global state (in-flight counters, feature flags, config, node group) is most
valuable when emitted alongside per-request metrics. Attaching it to each
unit-of-work record lets you correlate: "this request was throttled because
`ThrottlePolicy` was `Throttle` and there were 47 requests in flight."

Several primitives support this:

- [`State<T>`](https://docs.rs/metrique-util/latest/metrique_util/struct.State.html) (requires the `state` feature on
  `metrique-util`): an
  atomically swappable value. The first read captures a snapshot, so the
  emitted metric matches the state seen during processing.
- [`Counter`](https://docs.rs/metrique/latest/metrique/struct.Counter.html): a lock-free counter with
  `increment_scoped()` for tracking in-flight work.
- [`OnceLock<T>`](https://doc.rust-lang.org/std/sync/struct.OnceLock.html): for values initialized once at
  startup (node group, build version).

These can be `&'static` references or fields inside an `Arc<SharedState>`.
Either way, flatten them into your per-request metrics struct so they appear
in every record.

See the [global-state example](https://github.com/awslabs/metrique/blob/main/metrique/examples/global-state.rs)
and the [concurrency guide](https://docs.rs/metrique/latest/metrique/_guide/concurrency/) for details.

### Periodic metrics (gauges, counters, metadata)

Applications often have state that exists outside any single request: system
gauges (CPU, memory), global counters (total requests served, cache hits),
and metadata (node group, config version, feature flags).

**Prefer attaching this data to unit-of-work metrics** using the primitives
described in [shared state in unit-of-work](#shared-state-in-unit-of-work).
When a gauge or counter appears on every request record, you can correlate:
"latency spiked while memory was at 85% and there were 200 requests in
flight." Even data like CPU or memory usage benefits from appearing on
request records as context.

That said: For a dedicated time series or lightweight
standalone emission, you can also emit a metric struct on a timer:

```rust
use metrique::unit_of_work::metrics;
use metrique::CloseValue;
use metrique::ServiceMetrics;
use metrique::writer::{EntrySink, GlobalEntrySink};
use std::thread;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct SystemUsage {
    cpu_percent: f64,
    memory_mb: u64,
    open_file_descriptors: u64,
}

fn start_periodic_metrics() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(60));
        ServiceMetrics::sink().append(metrique::RootEntry::new(
            SystemUsage {
                cpu_percent: 0.0,   // collect real values here
                memory_mb: 0,
                open_file_descriptors: 0,
            }
            .close(),
        ));
    });
}
```

With periodic metrics it's important to consider emission time bias: for
example, if you are running a metric that records queue lengths on a tokio
task, this metric won't be reported if the runtime is stuck. Consider ways
to have the data reported by periodic metrics be time-of-report invariant
(e.g. track high water marks or histograms for the full range of values).

## "My TPS is too high"

Before dismissing unit-of-work metrics, consider
[sampling](https://docs.rs/metrique/latest/metrique/_guide/sampling/). The
[congressional sampler](https://docs.rs/metrique/latest/metrique/writer/sample/struct.CongressSample.html) preserves rare
events while reducing volume.

For truly high-frequency events, [`metrique-aggregation`](https://docs.rs/metrique-aggregation)
provides efficient aggregation with histograms. The best approach is often both:
aggregated metrics for dashboards and alarms, plus a sampled stream of raw events
for debugging.

## Metrics as logs vs. metrics as metrics

`metrique` blurs the line between "logs" and "metrics." Each metric entry is a
structured record that can serve both purposes:

- **Metrics as metrics**: numeric observations (latency, count, size) published to
  a metrics backend like CloudWatch for dashboards and alarms.
- **Metrics as logs**: the same records, with full context (request ID, operation,
  status code), archived for offline querying and debugging.

A common pattern is to [tee](https://docs.rs/metrique/latest/metrique/_guide/sampling/) the output into
both destinations: a sampled stream for the metrics backend and an unsampled
archive for log analysis. This gives you aggregated dashboards *and* the ability
to drill into individual records when debugging.
