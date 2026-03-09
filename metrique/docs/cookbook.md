# Principles and Patterns

This guide covers the principles behind effective metrics instrumentation and
helps you choose the right pattern for your use case.

## Principles

### Principle 1: Prefer unit-of-work metrics over time-aggregated metrics

Do not aggregate client-side unless necessary. Record metrics directly associated
with a unit of work and let your metrics backend perform aggregation.

**Unit-of-work metrics** (API response time, request size, request ID) let you
correlate individual records to debug *why* something happened. **Time-based
metrics** (CPU usage, tokio task count, disk usage) show behavior over time but
cannot explain causation.

A production application typically needs both. Unit-of-work metrics are the primary
focus of `metrique`; see [periodic metrics](#periodic-metrics) for the time-based
case.

### Principle 2: Define metrics explicitly

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
| [Aggregated](#aggregated) | >1000 events/sec where individual records are too expensive | Loses per-record context; consider combining with sampling |
| [Periodic (gauges)](#periodic-metrics-gauges) | System resources with no natural unit of work | Point-in-time only |
| [Global counters](#global-counters) | Deeply nested code where threading context is impractical | Loses request correlation |

### Unit-of-work

The most common pattern. Each request, job, or event gets its own metric record
with full context for debugging.

See the [Getting Started](crate#getting-started-applications) section and the
[unit-of-work-simple](https://github.com/awslabs/metrique/blob/main/metrique/examples/unit-of-work-simple.rs)
example.

### Sampled unit-of-work

When you want unit-of-work metrics but full emission is too expensive, sample
the stream. The [congressional sampler](`crate::writer::sample::CongressSample`)
gives rare events (errors, unusual operations) a higher sampling rate so they
aren't lost. A common setup is to tee into an archived log of record (all entries)
and a sampled stream for CloudWatch.

See [`_documentation::sampling`](crate::_documentation::sampling) for details and
a full example.

### Aggregated

When individual records are too expensive (roughly >1000/sec per source), aggregate
while preserving distributions via histograms. Consider combining with
[sampling](crate::_documentation::sampling) to keep some raw records for debugging.

Two flavors:

- **Embedded**: aggregate sub-operations within a single unit of work. See the
  [embedded example](https://github.com/awslabs/metrique/blob/main/metrique-aggregation/examples/embedded.rs).
- **Sink-level**: aggregate across units of work. See the
  [sink_level example](https://github.com/awslabs/metrique/blob/main/metrique-aggregation/examples/sink_level.rs).

See [`metrique-aggregation`](https://docs.rs/metrique-aggregation) for full details.

### Periodic metrics (gauges)

Emit a metric struct on a timer for resources with no natural unit of work (CPU,
memory, open file descriptors). These are point-in-time snapshots.

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

Unlike unit-of-work metrics, periodic metrics cannot explain *what* caused a change.
Prefer unit-of-work or embedded aggregation when possible.

### Global counters

Use only when threading a metrics context is impractical - code 10+ layers deep,
or across many trait boundaries. Global counters lose request correlation.

See the
[global-counter example](https://github.com/awslabs/metrique/blob/main/metrique/examples/global-counter.rs).

## "My TPS is too high"

Before dismissing unit-of-work metrics, consider
[sampling](crate::_documentation::sampling). The
[congressional sampler](`crate::writer::sample::CongressSample`) preserves rare
events while reducing volume.

For truly high-frequency events, [`metrique-aggregation`](https://docs.rs/metrique-aggregation)
provides efficient aggregation with histograms. The best approach is often both:
aggregated metrics for dashboards and alarms, plus a sampled stream of raw events
for debugging.
