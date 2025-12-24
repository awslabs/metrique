# metrique-aggregation

Histogram implementations for aggregating metrique metrics.

When emitting high-frequency metrics, you often want to aggregate multiple observations into a single metric entry rather than emitting each one individually. This crate provides histogram types that collect observations and emit them as distributions.

## When to use this

Use histograms when you have many observations of the same metric within a single unit of work. For example:

- A distributed query that fans out to multiple backend services
- Processing a batch of items where you want to track per-item latency
- Any operation that generates multiple measurements you want to aggregate

For most applications, [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md) is a better approach than aggregation. Consider histograms when you need precise distributions for high-frequency events.

## Example

```rust
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct QueryMetrics {
    query_id: String,
    
    #[metrics(unit = Millisecond)]
    backend_latency: Histogram<Duration>,
}

fn execute_query(query_id: String) {
    let mut metrics = QueryMetrics {
        query_id,
        backend_latency: Histogram::default(),
    };
    
    // Record multiple observations
    metrics.backend_latency.add_value(Duration::from_millis(45));
    metrics.backend_latency.add_value(Duration::from_millis(67));
    metrics.backend_latency.add_value(Duration::from_millis(52));
    
    // When metrics drops, emits a single entry with the distribution
}
```

## Histogram types

- **`Histogram<T, S = ExponentialAggregationStrategy>`** - Standard histogram that requires `&mut self` to add values. Uses exponential bucketing by default.
- **`SharedHistogram<T, S>`** - Thread-safe histogram that can add values with `&self`

## Aggregation strategies

By default, histograms use `ExponentialAggregationStrategy`. To use a different strategy, specify it as the second type parameter:

```rust
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use std::time::Duration;

let histogram: Histogram<Duration, SortAndMerge> = Histogram::new(SortAndMerge::new());
```

Available strategies:

- **`ExponentialAggregationStrategy`** (default) - Exponential bucketing with ~6.25% error. Best for most use cases.
- **`AtomicExponentialAggregationStrategy`** - Thread-safe version of exponential bucketing for use with `SharedHistogram`
- **`SortAndMerge`** - Stores all observations exactly and sorts them on emission. Perfect precision but higher memory usage.

Exponential strategies provide better precision across a wide range of values. SortAndMerge preserves all observations exactly but uses more memory.

## Future work

This crate currently provides histogram implementations. Future versions of metrique will include a full aggregation system with `Aggregated<T>` fields and sink-level aggregation. See the [aggregation RFC](https://github.com/awslabs/metrique/blob/aggregation-rfc/docs/aggregated.md) for the planned design.
