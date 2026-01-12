# metrique-aggregation

Aggregation system for combining multiple metric observations into single entries.

**Aggregation is an optional optimization for specific high-volume scenarios. For most applications, [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md) is the better approach.**

When emitting high-frequency metrics, you often want to aggregate multiple observations into a single metric entry rather than emitting each one individually. This crate provides an aggregation system that collects observations and emits them as distributions, sums, or other aggregate forms.

## When to Use Aggregation

Consider aggregation when:

- **High-frequency, low-level events**: TLS handshakes, storage operations, or other infrastructure-level metrics
- **Fan-out operations**: A single unit of work spawns multiple sub-operations you want to aggregate
- **Background processing**: Queue workers that generate one metric per processed item

**Most request/response services should use sampling instead of aggregation.**

## Quick Start

Use the `#[aggregate]` macro to define aggregatable metrics:

```rust,no_run
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, histogram::Histogram, value::Sum};
use metrique_aggregation::traits::Aggregate;
use metrique::unit::{Millisecond, Byte};
use std::time::Duration;

#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
    
    #[aggregate(strategy = Sum)]
    #[metrics(unit = Byte)]
    response_size: usize,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: String,
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
}

let mut metrics = RequestMetrics {
    request_id: "query-123".to_string(),
    api_calls: Aggregate::default(),
};

// Add multiple observations
metrics.api_calls.insert(ApiCall {
    latency: Duration::from_millis(45),
    response_size: 1024,
});
metrics.api_calls.insert(ApiCall {
    latency: Duration::from_millis(67),
    response_size: 2048,
});

// When metrics drops, emits a single entry with aggregated values
```

**Output**: Single metric entry with `RequestId: "query-123"`, `Latency: [45ms, 67ms]`, `ResponseSize: 3072`

## Core Concepts

### Field-Level Strategies

Individual fields use aggregation strategies that implement `AggregateValue<T>`:

- **`Sum`** - Sums values together (use for counts, totals)
- **`Histogram<T>`** - Collects values into a distribution (use for latency, sizes)
- **`LastValueWins`** - Keeps the most recent value (use for gauges, current state)

### Entry-Level Aggregation

The `#[aggregate]` macro generates implementations that define how complete entries are combined. It creates the merge logic, key extraction, and aggregation strategy for your type.

### Keys

Fields marked with `#[aggregate(key)]` become grouping keys. Entries with the same key are merged together:

```rust,no_run
#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,
    
    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,
}
```

Calls to the same endpoint will be aggregated together, while different endpoints remain separate.

## Usage Patterns

### Embedded Aggregation

Use `Aggregate<T>` as a field in your metrics struct when a single unit of work fans out to multiple sub-operations:

```rust,no_run
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, histogram::Histogram, value::Sum};
use metrique_aggregation::traits::Aggregate;
use std::time::Duration;

#[aggregate]
#[metrics]
struct BackendCall {
    #[aggregate(strategy = Sum)]
    requests_made: u64,
    
    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,
    
    #[aggregate(strategy = Sum)]
    errors: u64,
}

#[metrics(rename_all = "PascalCase")]
struct DistributedQuery {
    query_id: String,
    #[metrics(flatten)]
    backend_calls: Aggregate<BackendCall>,
}

async fn execute_query(query: &str) {
    let mut metrics = DistributedQuery {
        query_id: uuid::Uuid::new_v4().to_string(),
        backend_calls: Aggregate::default(),
    };

    // Fan out to multiple backends
    for backend in &["shard1", "shard2", "shard3"] {
        let start = std::time::Instant::now();
        let result = call_backend(backend, query).await;
        
        metrics.backend_calls.insert(BackendCall {
            requests_made: 1,
            latency: start.elapsed(),
            errors: if result.is_err() { 1 } else { 0 },
        });
    }
    
    // Metrics automatically emitted when dropped
}
```

**Output**: Single entry with `QueryId: "550e8400-..."`, `RequestsMade: 3`, `Latency: [45ms, 67ms, 52ms]`, `Errors: 1`

See [`examples/embedded.rs`](examples/embedded.rs) for a complete working example.

### Sink-Level Aggregation

Use `WorkerSink` or `MutexSink` for extremely high-rate background processing where you want aggregation across many separate operations:

```rust,no_run
use metrique_aggregation::{aggregate, KeyedAggregator, WorkerSink};
use std::time::Duration;

#[aggregate]
#[metrics]
struct QueueItem {
    #[aggregate(key)]
    item_type: String,
    
    #[aggregate(key)]
    priority: u8,
    
    #[aggregate(strategy = Sum)]
    items_processed: u64,
    
    #[aggregate(strategy = Histogram<Duration>)]
    processing_time: Duration,
}

async fn setup_queue_processor() {
    let base_sink = ServiceMetrics::sink();
    let keyed_aggregator = KeyedAggregator::<QueueItem>::new(base_sink);
    let sink = WorkerSink::new(keyed_aggregator, Duration::from_secs(60));
    
    // Process queue items
    while let Ok(item) = queue.recv().await {
        let start = std::time::Instant::now();
        process_item(&item).await;
        
        QueueItem {
            item_type: item.type_name(),
            priority: item.priority,
            items_processed: 1,
            processing_time: start.elapsed(),
        }
        .close_and_merge(sink.clone());
    }
    
    // Periodically flushes aggregated results (every 60 seconds)
}
```

**Output**: Multiple aggregated entries like `ItemType: "email", Priority: 1, ItemsProcessed: 1247, ProcessingTime: [histogram]`

**`WorkerSink`** runs a background thread that flushes periodically. **`MutexSink`** is a synchronous alternative that flushes manually or when a threshold is reached.

See [`examples/sink_level.rs`](examples/sink_level.rs) for a complete working example.

### Split Aggregation

Use `SplitSink` to aggregate the same data to multiple destinations - useful for combining precise aggregated metrics with sampled raw events:

```rust,no_run
use metrique_aggregation::{KeyedAggregator, WorkerSink};
use metrique_aggregation::split_sink::{SplitSink, RawSink};

// Aggregator for precise counts
let aggregator = KeyedAggregator::<QueueItem>::new(aggregated_sink);

// Raw sink for sampling individual events
let raw = RawSink::new(raw_events_sink);

// Combine them
let split = SplitSink::new(aggregator, raw);
let sink = WorkerSink::new(split, Duration::from_secs(60));

// Each entry goes to both sinks
QueueItem { /* ... */ }.close_and_merge(sink.clone());
```

This gives you:
- **Precise aggregated metrics**: Exact counts and distributions
- **Raw event samples**: Individual events for tracing and debugging

See [`examples/split.rs`](examples/split.rs) for a complete working example.

## Aggregation Traits and How They Work Together

The aggregation system is built on several traits that work together:

- **[`AggregateValue<T>`](src/traits.rs)** - Defines how individual field values are merged (Sum, Histogram, LastValueWins)
- **[`Merge`](src/traits.rs)** - Defines how complete entries are merged together by consuming the source
- **[`MergeRef`](src/traits.rs)** - Like `Merge`, but merges by reference (enables `SplitSink` to send to multiple destinations)
- **[`Key`](src/traits.rs)** - Extracts grouping keys from entries to determine which entries should be merged
- **[`AggregateStrategy`](src/traits.rs)** - Ties together the source type, merge behavior, and key extraction
- **[`AggregateSink<T>`](src/traits.rs)** - Destination that accepts and aggregates entries

The `#[aggregate]` macro generates implementations of these traits for your type. For most use cases, you don't need to implement these manually - the macro handles it.

## Histogram Strategies

Histograms support different bucketing strategies:

- **`ExponentialAggregationStrategy`** (default) - Exponential bucketing with ~6.25% error, memory efficient
- **`SortAndMerge`** - Stores all observations exactly for perfect precision
- **`AtomicExponentialAggregationStrategy`** - Thread-safe exponential bucketing for `SharedHistogram`

```rust,ignore
use metrique_aggregation::histogram::{Histogram, SortAndMerge};

#[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
latency: Duration,
```

## When NOT to Use Aggregation

- **Request/response services**: Use sampling instead
- **Low-frequency events**: Aggregation overhead isn't worth it
- **Need individual event details**: Aggregation loses individual event context
- **Simple counting**: Basic counters don't need aggregation complexity

## Next Steps

- See the [`examples/`](examples/) directory for complete working examples
- Read the [trait documentation](src/traits.rs) for advanced usage
- Learn about [sampling](https://github.com/awslabs/metrique/blob/main/docs/sampling.md) for the recommended approach for most applications
