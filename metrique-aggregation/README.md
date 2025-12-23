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
use metrique_aggregation::histogram::{Histogram, LinearAggregationStrategy};
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct QueryMetrics {
    query_id: String,
    
    #[metrics(unit = Millisecond)]
    backend_latency: Histogram<Duration, LinearAggregationStrategy>,
}

fn execute_query(query_id: String) {
    let mut metrics = QueryMetrics {
        query_id,
        backend_latency: Histogram::new(LinearAggregationStrategy::new(10.0, 10)),
    };
    
    // Record multiple observations
    metrics.backend_latency.add_value(Duration::from_millis(45));
    metrics.backend_latency.add_value(Duration::from_millis(67));
    metrics.backend_latency.add_value(Duration::from_millis(52));
    
    // When metrics drops, emits a single entry with the distribution
}
```

## Histogram types

- **`Histogram<T, S>`** - Standard histogram that requires `&mut self` to add values
- **`AtomicHistogram<T, S>`** - Thread-safe histogram that can add values with `&self`

## Aggregation strategies

Strategies determine how observations are stored and emitted:

- **`LinearAggregationStrategy`** - Fixed-width buckets (e.g., 0-10ms, 10-20ms, 20-30ms)
- **`AtomicLinearAggregationStrategy`** - Thread-safe version of linear bucketing
- **`SortAndMerge`** - Stores all observations and sorts them on emission

Choose your strategy based on your precision needs and memory constraints. Linear strategies use less memory but lose some precision. SortAndMerge preserves all observations but uses more memory.

## Future work

This crate currently provides histogram implementations. Future versions of metrique will include a full aggregation system with `Aggregated<T>` fields and sink-level aggregation. See the [aggregation RFC](https://github.com/awslabs/metrique/blob/aggregation-rfc/docs/aggregated.md) for the planned design.
