# metrique-aggregation

Aggregation system for combining multiple metric observations into single entries.

When emitting high-frequency metrics, you often want to aggregate multiple observations into a single metric entry rather than emitting each one individually. This crate provides an aggregation system that collects observations and emits them as distributions, sums, or other aggregate forms.

## When to use this

Use aggregation when you have many observations of the same metric within a single unit of work:

- A distributed query that fans out to multiple backend services
- Processing a batch of items where you want to track per-item latency
- Any operation that generates multiple measurements you want to aggregate

For most applications, [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md) is a better approach than aggregation. Consider aggregation when you need precise distributions or totals for high-frequency events.

## Quick start

Use the `#[aggregate]` macro to define aggregatable metrics:

```rust
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, histogram::Histogram, counter::Counter};
use metrique_aggregation::traits::Aggregate;
use metrique_writer::unit::{Millisecond, Byte};
use std::time::Duration;

#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
    
    #[aggregate(strategy = Counter)]
    #[metrics(unit = Byte)]
    response_size: usize,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: String,
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
}

fn main() {
    let mut metrics = RequestMetrics {
        request_id: "query-123".to_string(),
        api_calls: Aggregate::default(),
    };
    
    // Add multiple observations
    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(45),
        response_size: 1024,
    });
    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(67),
        response_size: 2048,
    });
    
    // When metrics drops, emits a single entry with aggregated values
}
```

## How it works

The aggregation system has two levels:

### Field-level aggregation

Individual fields use aggregation strategies that implement `AggregateValue<T>`:

- **`Counter`** - Sums values together
- **`Histogram<T>`** - Collects values into a distribution
- **`LastValueWins`** - Keeps the most recent value

### Entry-level aggregation

The `#[aggregate]` macro generates an implementation of `AggregateEntry` that combines all fields according to their strategies. The macro creates an aggregated version of your struct where each field is replaced with its aggregated type.

## Aggregation patterns

### Simple aggregation with `Aggregate<T>`

Use `Aggregate<T>` as a field in your metrics struct for straightforward aggregation:

```rust,ignore
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
    request_id: String,
}
```

### Thread-safe aggregation with `MutexAggregator<T>`

Use `MutexAggregator<T>` when you need to aggregate from multiple threads or use `merge_on_drop`:

```rust,ignore
let metrics = RequestMetrics {
    api_calls: MutexAggregator::new(),
    request_id: "1234".to_string(),
};

// Create a guard that merges on drop
let mut call = ApiCall {
    latency: Duration::from_millis(100),
    response_size: 50,
}.merge_on_drop(&metrics.api_calls);

// Modify the call before it's merged
call.response_size = 75;
// Automatically merged when guard drops
```

### Keyed aggregation with `KeyedAggregationSink`

Use `KeyedAggregationSink` to aggregate by key with time-based flushing:

```rust,no_run
# use metrique_aggregation::keyed_sink::KeyedAggregationSink;
# use std::time::Duration;
# use metrique_aggregation::aggregate;
# use metrique::unit_of_work::metrics;
# use metrique_aggregation::histogram::Histogram;
# #[aggregate]
# #[metrics]
# struct ApiCall {
#     #[aggregate(key)]
#     endpoint: String,
#     #[aggregate(strategy = Histogram<Duration>)]
#     latency: Duration,
# }
# let my_sink = metrique_writer::test_util::test_entry_sink().sink;
let sink = KeyedAggregationSink::<ApiCall, _>::new(
    my_sink,
    Duration::from_secs(60), // Flush every 60 seconds
);

sink.send(ApiCall {
    endpoint: "GetItem".to_string(),
    latency: Duration::from_millis(10),
});
```

## Histogram strategies

Histograms support different bucketing strategies:

- **`ExponentialAggregationStrategy`** (default) - Exponential bucketing with ~6.25% error
- **`SortAndMerge`** - Stores all observations exactly for perfect precision
- **`AtomicExponentialAggregationStrategy`** - Thread-safe exponential bucketing for `SharedHistogram`

```rust,ignore
use metrique_aggregation::histogram::{Histogram, SortAndMerge};

#[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
latency: Duration,
```

## Manual implementation

While `#[aggregate]` is the recommended approach, you can implement `AggregateEntry` manually for full control. See the `manual_aggregation` example for details.
